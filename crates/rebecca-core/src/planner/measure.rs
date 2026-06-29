use std::path::{Path, PathBuf};

use crate::app_leftovers::AppLeftoverCandidate;
use crate::error::{RebeccaError, Result};
use crate::model::Platform;
use crate::plan::{
    CleanupPlan, CleanupTarget, CleanupTargetDeletionStyle, CleanupTargetIssueReason,
};
use crate::project_artifacts::ProjectArtifactCandidate;
use crate::protection::{AppLeftoverPathDisposition, ProtectionPolicy};
use crate::safety::{
    PATH_DOES_NOT_EXIST_REASON, PathDisposition, assess_existing_path_with_policy,
};
use crate::scan::{ScanEngine, ScanProgressEvent, ScanReport};
use crate::scan_cache::{ScanCacheLookup, ScanCacheMiss};

use super::{PlanBuildContext, PlanProgressEvent};

#[derive(Debug, Clone)]
pub(crate) struct MeasuredTarget {
    pub(crate) target: CleanupTarget,
    file_progress: Vec<MeasuredFileProgress>,
    scan_cache_event: Option<MeasuredScanCacheEvent>,
}

#[derive(Debug, Clone)]
struct MeasuredFileProgress {
    path: PathBuf,
    file_size: u64,
    files_scanned: u64,
    bytes_scanned: u64,
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
        app_leftover_rule_id(leftover),
        leftover.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(app_leftover_restore_hint(leftover)))
    .with_deletion_style(CleanupTargetDeletionStyle::PreserveRootContents)
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
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

    for event in &measured.file_progress {
        progress(PlanProgressEvent::FileMeasured {
            rule_id: &measured.target.rule_id,
            target_path: &measured.target.path,
            path: event.path.as_path(),
            file_size: event.file_size,
            files_scanned: event.files_scanned,
            bytes_scanned: event.bytes_scanned,
        });
    }
}

pub(crate) fn dedupe_key(path: &Path, _platform: Platform) -> String {
    let mut normalized = path.as_os_str().to_string_lossy().replace('\\', "/");

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    normalized.to_ascii_lowercase()
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
            if min_age_days > 0 {
                if let Some(reason) = T::recently_modified_reason(&candidate, min_age_days) {
                    let target = skipped_target(
                        &candidate,
                        mode,
                        CleanupTargetIssueReason::ProjectArtifactRecentlyModified,
                        reason,
                    );
                    return Ok(MeasuredTarget {
                        target,
                        file_progress: Vec::new(),
                        scan_cache_event: None,
                    });
                }
            }

            let mut file_progress = Vec::new();
            let mut scan_cache_event = None;
            let report =
                measure_path_with_optional_scan_cache(T::path(&candidate), context, |event| {
                    match event {
                        PathMeasureProgressEvent::Scan(ScanProgressEvent::FileMeasured {
                            path,
                            file_size,
                            files_scanned,
                            bytes_scanned,
                        }) => {
                            file_progress.push(MeasuredFileProgress {
                                path: path.to_path_buf(),
                                file_size,
                                files_scanned,
                                bytes_scanned,
                            });
                        }
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
                })?;

            let target = allowed_target(&candidate, report.bytes_scanned, mode);
            Ok(MeasuredTarget {
                target,
                file_progress,
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
            file_progress: Vec::new(),
            scan_cache_event: None,
        }),
        CandidateDisposition::Blocked(reason) => Ok(MeasuredTarget {
            target: blocked_target(
                &candidate,
                mode,
                CleanupTargetIssueReason::SafetyPolicyBlocked,
                reason,
            ),
            file_progress: Vec::new(),
            scan_cache_event: None,
        }),
    }
}

pub(crate) fn measure_path_with_optional_scan_cache<F>(
    path: &Path,
    context: PlanBuildContext<'_>,
    mut progress: F,
) -> Result<ScanReport>
where
    F: for<'a> FnMut(PathMeasureProgressEvent<'a>),
{
    if context.cancellation().is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "scan was cancelled".to_string(),
        ));
    }

    let cacheable_target = context.scan_cache().is_some() && is_cacheable_scan_target(path);
    if cacheable_target {
        if let Some(store) = context.scan_cache() {
            match store.load_with_policy(path, context.scan_cache_policy()) {
                ScanCacheLookup::Hit(report) => {
                    progress(PathMeasureProgressEvent::ScanCacheHit { report });
                    return Ok(report);
                }
                ScanCacheLookup::Miss(outcome) => {
                    progress(PathMeasureProgressEvent::ScanCacheMiss {
                        reason: outcome.reason,
                        pruned: outcome.pruned,
                    });
                }
            }
        }
    }

    let report =
        ScanEngine::new().measure_path_with_progress(path, context.cancellation(), |event| {
            progress(PathMeasureProgressEvent::Scan(event));
        })?;

    if cacheable_target {
        if let Some(store) = context.scan_cache() {
            if let Err(err) = store.store(path, report) {
                tracing::debug!(
                    path = %path.display(),
                    error = %err,
                    "scan cache write skipped"
                );
                progress(PathMeasureProgressEvent::ScanCacheWriteSkipped);
            }
        }
    }

    Ok(report)
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
    .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
}

fn project_artifact_skipped_target(
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
    .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
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
    .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath)
    .with_modified_at_unix_seconds(artifact.modified_at_unix_seconds)
}

fn app_leftover_allowed_target(
    leftover: &AppLeftoverCandidate,
    estimated_bytes: u64,
    mode: crate::DeleteMode,
) -> CleanupTarget {
    CleanupTarget::allowed(
        app_leftover_rule_id(leftover),
        leftover.path.clone(),
        estimated_bytes,
        mode,
    )
    .with_restore_hint(Some(app_leftover_restore_hint(leftover)))
    .with_deletion_style(CleanupTargetDeletionStyle::PreserveRootContents)
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

fn app_leftover_blocked_target(
    leftover: &AppLeftoverCandidate,
    mode: crate::DeleteMode,
    reason_code: CleanupTargetIssueReason,
    reason: impl Into<String>,
) -> CleanupTarget {
    CleanupTarget::blocked_with_reason_code(
        app_leftover_rule_id(leftover),
        leftover.path.clone(),
        mode,
        reason_code,
        reason,
    )
    .with_restore_hint(Some(app_leftover_restore_hint(leftover)))
    .with_deletion_style(CleanupTargetDeletionStyle::PreserveRootContents)
    .with_modified_at_unix_seconds(leftover.modified_at_unix_seconds)
}

pub(crate) fn app_leftover_rule_id(leftover: &AppLeftoverCandidate) -> &'static str {
    match leftover.source {
        crate::app_leftovers::AppLeftoverSource::LocalAppData => "windows.app-leftover-local-cache",
        crate::app_leftovers::AppLeftoverSource::RoamingAppData => {
            "windows.app-leftover-roaming-cache"
        }
        crate::app_leftovers::AppLeftoverSource::LocalLowAppData => {
            "windows.app-leftover-local-low-cache"
        }
    }
}

fn app_leftover_restore_hint(leftover: &AppLeftoverCandidate) -> String {
    format!(
        "{} {} cache data is rebuildable.",
        leftover.app.display_name(),
        leftover.source.label()
    )
}
