use std::path::Path;

use crate::app_leftovers::{AppLeftoverCandidate, AppLeftoverDeletionStyle};
use crate::error::{RebeccaError, Result, ScanFailureKind};
use crate::model::Platform;
use crate::plan::{
    CleanupPlan, CleanupTarget, CleanupTargetDeletionStyle, CleanupTargetIssueReason,
    EstimateProvenance, EstimateSource,
};
use crate::project_artifacts::ProjectArtifactCandidate;
use crate::protection::{AppLeftoverPathDisposition, ProtectionPolicy};
use crate::safety::{
    PATH_DOES_NOT_EXIST_REASON, PathDisposition, assess_existing_path_with_policy, is_reparse_like,
};
use crate::scan::{ScanProgressEvent, ScanReport};
use crate::scan_cache::{ScanCacheCompatibility, ScanCacheLookup, ScanCacheMiss};

use super::{PlanBuildContext, PlanProgressEvent};

#[derive(Debug, Clone)]
pub(crate) struct MeasuredTarget {
    pub(crate) target: CleanupTarget,
    scan_cache_event: Option<MeasuredScanCacheEvent>,
}

#[derive(Debug, Clone)]
pub(crate) struct MeasuredPath {
    pub(crate) report: ScanReport,
    pub(crate) estimate_source: EstimateSource,
    pub(crate) estimate_provenance: EstimateProvenance,
}

#[derive(Debug, Clone)]
enum MeasuredScanCacheEvent {
    Hit { estimated_bytes: u64 },
    Miss { reason: ScanCacheMiss, pruned: bool },
    WriteSkipped,
}

enum CandidateDisposition {
    Allowed,
    Skipped(String),
    Blocked(String),
}

trait CandidatePath {
    fn path(&self) -> &Path;
    fn assess(candidate: &Self, policy: ProtectionPolicy<'_>) -> CandidateDisposition;
    fn recently_modified_reason(candidate: &Self, min_age_days: u64) -> Option<String>;
}

impl CandidatePath for ProjectArtifactCandidate {
    fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn assess(candidate: &Self, policy: ProtectionPolicy<'_>) -> CandidateDisposition {
        match assess_existing_path_with_policy(&candidate.path, policy) {
            PathDisposition::Allowed => CandidateDisposition::Allowed,
            PathDisposition::Missing => {
                CandidateDisposition::Skipped(PATH_DOES_NOT_EXIST_REASON.to_string())
            }
            PathDisposition::Skipped(reason) => CandidateDisposition::Skipped(reason),
            PathDisposition::Blocked(reason) => CandidateDisposition::Blocked(reason),
        }
    }

    fn recently_modified_reason(candidate: &Self, min_age_days: u64) -> Option<String> {
        crate::project_artifacts::recently_modified_reason(candidate.path(), min_age_days)
    }
}

impl CandidatePath for AppLeftoverCandidate {
    fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn assess(candidate: &Self, policy: ProtectionPolicy<'_>) -> CandidateDisposition {
        match policy.assess_existing_app_leftover_path(&candidate.path) {
            AppLeftoverPathDisposition::Allowed => CandidateDisposition::Allowed,
            AppLeftoverPathDisposition::Missing => {
                CandidateDisposition::Skipped(PATH_DOES_NOT_EXIST_REASON.to_string())
            }
            AppLeftoverPathDisposition::Blocked(reason) => CandidateDisposition::Blocked(reason),
        }
    }

    fn recently_modified_reason(_candidate: &Self, _min_age_days: u64) -> Option<String> {
        None
    }
}

pub(crate) fn finalize_plan(
    request: crate::PlanRequest,
    mut targets: Vec<CleanupTarget>,
) -> CleanupPlan {
    targets.sort_by(|left, right| {
        left.rule_id
            .cmp(&right.rule_id)
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut plan = CleanupPlan::empty(request);
    plan.targets = targets;
    plan.recompute_summary();
    plan
}

pub(crate) fn measure_project_artifact_candidate(
    artifact: ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    min_age_days: u64,
    context: PlanBuildContext<'_>,
) -> Result<MeasuredTarget> {
    measure_project_candidate(
        artifact,
        mode,
        min_age_days,
        context,
        project_artifact_allowed_target,
        project_artifact_skipped_target,
        project_artifact_blocked_target,
    )
}

pub(crate) fn measure_app_leftover_candidate(
    leftover: AppLeftoverCandidate,
    mode: crate::DeleteMode,
    context: PlanBuildContext<'_>,
) -> Result<MeasuredTarget> {
    measure_project_candidate(
        leftover,
        mode,
        0,
        context,
        app_leftover_allowed_target,
        app_leftover_skipped_target,
        app_leftover_blocked_target,
    )
}

pub(crate) fn app_leftover_skipped_target(
    leftover: &AppLeftoverCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::skipped_with_reason_code(
        leftover.rule_id(),
        leftover.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(leftover.restore_hint()))
    .with_deletion_style(cleanup_target_deletion_style(leftover.deletion_style()))
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

pub(crate) fn scan_issue_reason(err: &RebeccaError) -> CleanupTargetIssueReason {
    match err {
        RebeccaError::ScanFailed(failure) if failure.kind == ScanFailureKind::PermissionDenied => {
            CleanupTargetIssueReason::ScanPermissionDenied
        }
        _ => CleanupTargetIssueReason::ScanFailed,
    }
}

pub(crate) fn emit_target_finished<F>(progress: &mut F, target: &CleanupTarget)
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    progress(PlanProgressEvent::TargetFinished {
        rule_id: &target.rule_id,
        path: &target.path,
        status: target.status,
        estimated_bytes: target.estimated_bytes,
    });
}

pub(crate) fn emit_measured_target_progress<F>(progress: &mut F, measured: &MeasuredTarget)
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    if let Some(event) = &measured.scan_cache_event {
        match event {
            MeasuredScanCacheEvent::Hit { estimated_bytes } => {
                progress(PlanProgressEvent::ScanCacheHit {
                    rule_id: &measured.target.rule_id,
                    path: &measured.target.path,
                    estimated_bytes: *estimated_bytes,
                })
            }
            MeasuredScanCacheEvent::Miss { reason, pruned } => {
                progress(PlanProgressEvent::ScanCacheMiss {
                    rule_id: &measured.target.rule_id,
                    path: &measured.target.path,
                    reason: *reason,
                    pruned: *pruned,
                })
            }
            MeasuredScanCacheEvent::WriteSkipped => {
                progress(PlanProgressEvent::ScanCacheWriteSkipped {
                    rule_id: &measured.target.rule_id,
                    path: &measured.target.path,
                })
            }
        }
    }
}

pub(crate) fn dedupe_key(path: &Path, platform: Platform) -> String {
    if let Some(identity) = directory_identity(path) {
        return format!("dir-identity:{}:{}", identity.device, identity.file_index);
    }

    let mut normalized = path.as_os_str().to_string_lossy().replace('\\', "/");

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    if platform.is_windows() {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

#[derive(Debug, Clone, Copy)]
struct DirectoryIdentity {
    device: u64,
    file_index: u64,
}

#[cfg(windows)]
fn directory_identity(path: &Path) -> Option<DirectoryIdentity> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    };

    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || is_reparse_like(&metadata) {
        return None;
    }

    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
        .ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.ok()?;
    let file_index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);

    Some(DirectoryIdentity {
        device: u64::from(info.dwVolumeSerialNumber),
        file_index,
    })
}

#[cfg(unix)]
fn directory_identity(path: &Path) -> Option<DirectoryIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || is_reparse_like(&metadata) {
        return None;
    }

    Some(DirectoryIdentity {
        device: metadata.dev(),
        file_index: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn directory_identity(_path: &Path) -> Option<DirectoryIdentity> {
    None
}

pub(crate) fn prune_scan_cache<F>(context: PlanBuildContext<'_>, progress: &mut F)
where
    F: for<'a> FnMut(PlanProgressEvent<'a>),
{
    if let Some(scan_cache) = context.scan_cache() {
        let report = scan_cache.prune_with_policy(context.scan_cache_policy());
        if report.inspected > 0 {
            progress(PlanProgressEvent::ScanCachePruned { report });
        }
    }
}

fn measure_project_candidate<T, Allowed, Skipped, Blocked>(
    candidate: T,
    mode: crate::DeleteMode,
    min_age_days: u64,
    context: PlanBuildContext<'_>,
    allowed_target: Allowed,
    skipped_target: Skipped,
    blocked_target: Blocked,
) -> Result<MeasuredTarget>
where
    T: CandidatePath,
    Allowed: FnOnce(&T, u64, crate::DeleteMode) -> CleanupTarget,
    Skipped: FnOnce(&T, crate::DeleteMode, CleanupTargetIssueReason, String) -> CleanupTarget,
    Blocked: FnOnce(&T, crate::DeleteMode, CleanupTargetIssueReason, String) -> CleanupTarget,
{
    match T::assess(&candidate, context.protection_policy()) {
        CandidateDisposition::Allowed => {
            if min_age_days > 0
                && let Some(reason) = T::recently_modified_reason(&candidate, min_age_days)
            {
                let target = skipped_target(
                    &candidate,
                    mode,
                    CleanupTargetIssueReason::ProjectArtifactRecentlyModified,
                    reason,
                );
                return Ok(MeasuredTarget {
                    target,
                    scan_cache_event: None,
                });
            }

            let mut scan_cache_event = None;
            let measured_path =
                match measure_path_with_optional_scan_cache(T::path(&candidate), context, |event| {
                    match event {
                        PathMeasureProgressEvent::Scan(ScanProgressEvent::FileMeasured {
                            ..
                        }) => {}
                        PathMeasureProgressEvent::ScanCacheHit { report } => {
                            scan_cache_event = Some(MeasuredScanCacheEvent::Hit {
                                estimated_bytes: report.bytes_scanned,
                            });
                        }
                        PathMeasureProgressEvent::ScanCacheMiss { reason, pruned } => {
                            scan_cache_event =
                                Some(MeasuredScanCacheEvent::Miss { reason, pruned });
                        }
                        PathMeasureProgressEvent::ScanCacheWriteSkipped => {
                            scan_cache_event = Some(MeasuredScanCacheEvent::WriteSkipped);
                        }
                    }
                }) {
                    Ok(measured_path) => measured_path,
                    Err(err) => {
                        let reason_code = scan_issue_reason(&err);
                        let target = skipped_target(&candidate, mode, reason_code, err.to_string());
                        return Ok(MeasuredTarget {
                            target,
                            scan_cache_event,
                        });
                    }
                };

            let target = allowed_target(&candidate, measured_path.report.bytes_scanned, mode)
                .with_estimate_source(measured_path.estimate_source)
                .with_estimate_provenance(measured_path.estimate_provenance);
            Ok(MeasuredTarget {
                target,
                scan_cache_event,
            })
        }
        CandidateDisposition::Skipped(reason) => Ok(MeasuredTarget {
            target: skipped_target(
                &candidate,
                mode,
                CleanupTargetIssueReason::SafetyPolicySkipped,
                reason,
            ),
            scan_cache_event: None,
        }),
        CandidateDisposition::Blocked(reason) => Ok(MeasuredTarget {
            target: blocked_target(
                &candidate,
                mode,
                CleanupTargetIssueReason::SafetyPolicyBlocked,
                reason,
            ),
            scan_cache_event: None,
        }),
    }
}

pub(crate) fn measure_path_with_optional_scan_cache<F>(
    path: &Path,
    context: PlanBuildContext<'_>,
    mut progress: F,
) -> Result<MeasuredPath>
where
    F: for<'a> FnMut(PathMeasureProgressEvent<'a>),
{
    if context.cancellation().is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "scan was cancelled".to_string(),
        ));
    }

    let cacheable_target = context.scan_cache().is_some() && is_cacheable_scan_target(path);
    let mut cache_miss_reason = None;
    if cacheable_target && let Some(store) = context.scan_cache() {
        let compatibility = ScanCacheCompatibility::logical_bytes(context.scan_backend());
        match store.load_with_policy_and_compatibility(
            path,
            context.scan_cache_policy(),
            compatibility,
        ) {
            ScanCacheLookup::Hit(hit) => {
                progress(PathMeasureProgressEvent::ScanCacheHit { report: hit.report });
                let mut evidence = hit.backend_evidence;
                evidence.record_cache_event("scan-cache", "hit", None);
                return Ok(MeasuredPath {
                    report: hit.report,
                    estimate_source: EstimateSource::ScanCache,
                    estimate_provenance: EstimateProvenance::from_backend_confidence_and_source(
                        hit.backend,
                        hit.confidence,
                        hit.backend_source,
                    )
                    .with_backend_evidence(evidence),
                });
            }
            ScanCacheLookup::Miss(outcome) => {
                progress(PathMeasureProgressEvent::ScanCacheMiss {
                    reason: outcome.reason,
                    pruned: outcome.pruned,
                });
                cache_miss_reason = Some(outcome.reason);
            }
        }
    }

    let measured_scan = context.scan_engine().measure_scan_with_backend(
        path,
        context.cancellation(),
        context.scan_backend(),
        |event| {
            progress(PathMeasureProgressEvent::Scan(event));
        },
    )?;
    let report = measured_scan.report;
    let mut estimate_backend_evidence = measured_scan.backend_evidence.clone();

    if let Some(reason) = cache_miss_reason {
        estimate_backend_evidence.record_cache_event(
            "scan-cache",
            "miss",
            Some(reason.label().to_string()),
        );
    }

    if cacheable_target
        && let Some(store) = context.scan_cache()
        && let Err(err) = store.store_measured_scan_with_policy(
            path,
            measured_scan.clone(),
            context.scan_cache_policy(),
        )
    {
        tracing::debug!(
            path = %path.display(),
            error = %err,
            "scan cache write skipped"
        );
        progress(PathMeasureProgressEvent::ScanCacheWriteSkipped);
        estimate_backend_evidence.record_cache_event(
            "scan-cache",
            "write-skipped",
            Some("write-failed".to_string()),
        );
    }

    let mut estimate_provenance = EstimateProvenance::from_measured_scan(&measured_scan);
    estimate_provenance.estimate_backend_evidence = estimate_backend_evidence;

    Ok(MeasuredPath {
        report,
        estimate_source: EstimateSource::FreshScan,
        estimate_provenance,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PathMeasureProgressEvent<'a> {
    Scan(ScanProgressEvent<'a>),
    ScanCacheHit { report: ScanReport },
    ScanCacheMiss { reason: ScanCacheMiss, pruned: bool },
    ScanCacheWriteSkipped,
}

fn is_cacheable_scan_target(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() || metadata.is_dir())
        .unwrap_or(false)
}

fn project_artifact_allowed_target(
    artifact: &ProjectArtifactCandidate,
    estimated_bytes: u64,
    mode: crate::DeleteMode,
) -> CleanupTarget {
    CleanupTarget::allowed(
        artifact.definition.rule_id,
        artifact.path.clone(),
        estimated_bytes,
        mode,
    )
    .with_restore_hint(Some(artifact.definition.restore_hint.to_string()))
    .with_deletion_style(artifact.policy.deletion_style)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
    .with_project_artifact_context(Some(artifact.context.clone()))
}

pub(crate) fn project_artifact_skipped_target(
    artifact: &ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::skipped_with_reason_code(
        artifact.definition.rule_id,
        artifact.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(artifact.definition.restore_hint.to_string()))
    .with_deletion_style(artifact.policy.deletion_style)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
    .with_project_artifact_context(Some(artifact.context.clone()))
}

fn project_artifact_blocked_target(
    artifact: &ProjectArtifactCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::blocked_with_reason_code(
        artifact.definition.rule_id,
        artifact.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(artifact.definition.restore_hint.to_string()))
    .with_deletion_style(artifact.policy.deletion_style)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
    .with_project_artifact_context(Some(artifact.context.clone()))
}

fn app_leftover_allowed_target(
    leftover: &AppLeftoverCandidate,
    estimated_bytes: u64,
    mode: crate::DeleteMode,
) -> CleanupTarget {
    CleanupTarget::allowed(
        leftover.rule_id(),
        leftover.path.clone(),
        estimated_bytes,
        mode,
    )
    .with_restore_hint(Some(leftover.restore_hint()))
    .with_deletion_style(cleanup_target_deletion_style(leftover.deletion_style()))
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

fn app_leftover_blocked_target(
    leftover: &AppLeftoverCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::blocked_with_reason_code(
        leftover.rule_id(),
        leftover.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(leftover.restore_hint()))
    .with_deletion_style(cleanup_target_deletion_style(leftover.deletion_style()))
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

fn cleanup_target_deletion_style(
    deletion_style: AppLeftoverDeletionStyle,
) -> CleanupTargetDeletionStyle {
    match deletion_style {
        AppLeftoverDeletionStyle::PreserveRootContents => {
            CleanupTargetDeletionStyle::PreserveRootContents
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::error::{RebeccaError, ScanFailure, ScanFailureKind, ScanFailurePhase};
    use crate::plan::CleanupTargetIssueReason;

    use super::scan_issue_reason;

    #[test]
    fn scan_issue_reason_preserves_permission_denied() {
        let err = RebeccaError::ScanFailed(ScanFailure {
            kind: ScanFailureKind::PermissionDenied,
            phase: ScanFailurePhase::RootMetadata,
            path: PathBuf::from("/Users/alice/Library/Mail"),
            message: "Operation not permitted".to_string(),
        });

        assert_eq!(
            scan_issue_reason(&err),
            CleanupTargetIssueReason::ScanPermissionDenied
        );
    }

    #[test]
    fn scan_issue_reason_uses_generic_scan_for_other_failures() {
        let err = RebeccaError::ScanFailed(ScanFailure {
            kind: ScanFailureKind::MetadataUnavailable,
            phase: ScanFailurePhase::EntryMetadata,
            path: PathBuf::from("/tmp/cache"),
            message: "metadata unavailable".to_string(),
        });

        assert_eq!(
            scan_issue_reason(&err),
            CleanupTargetIssueReason::ScanFailed
        );
    }
}
