use std::fmt::Write as _;

use anyhow::{Result, anyhow};
use rebecca::core::cache::{
    CacheDoctorReport, CacheInventory, CacheInventoryEntryStatus, CacheNamespace, CachePruneReport,
    CachePurgeMode, CachePurgeReport, doctor_app_cache, inspect_app_cache,
    prune_app_cache_inventory, purge_app_cache, purge_app_cache_with_backend,
};
use rebecca::core::config::{load_app_paths, load_runtime_config};

use crate::cache_view::CachePurgeProjection;
use crate::cli::OutputMode;
use crate::output::{format_bytes, format_shell_command};

#[derive(Debug)]
pub struct CachePurgeOptions {
    pub dry_run: bool,
    pub output_mode: OutputMode,
    pub yes: bool,
    pub permanent: bool,
}

#[derive(Debug)]
pub struct CacheInspectOptions {
    pub output_mode: OutputMode,
    pub namespace: CacheNamespace,
}

#[derive(Debug)]
pub struct CacheDoctorOptions {
    pub output_mode: OutputMode,
}

#[derive(Debug)]
pub struct CachePruneOptions {
    pub output_mode: OutputMode,
    pub namespace: CacheNamespace,
    pub stale_only: bool,
    pub limit: Option<std::num::NonZeroUsize>,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn inspect(options: CacheInspectOptions) -> Result<()> {
    let runtime_config = load_runtime_config()?;
    let inventory = inspect_app_cache(
        &runtime_config.app_paths,
        options.namespace,
        runtime_config.scan_cache_policy,
    );
    crate::output::print_command_success(
        "cache inspect",
        "cache-inventory",
        options.output_mode,
        || &inventory,
        || {
            print!("{}", render_cache_inventory(&inventory)?);
            Ok(())
        },
    )
}

pub fn doctor(options: CacheDoctorOptions) -> Result<()> {
    let runtime_config = load_runtime_config()?;
    let report = doctor_app_cache(&runtime_config.app_paths, runtime_config.scan_cache_policy);
    crate::output::print_command_success(
        "cache doctor",
        "cache-doctor",
        options.output_mode,
        || &report,
        || {
            print!("{}", render_cache_doctor_report(&report)?);
            Ok(())
        },
    )
}

pub fn prune(options: CachePruneOptions) -> Result<()> {
    let runtime_config = load_runtime_config()?;
    if options.dry_run && options.yes {
        return Err(anyhow!("--dry-run cannot be combined with --yes"));
    }
    let dry_run = !options.yes || options.dry_run;
    let report = prune_app_cache_inventory(
        &runtime_config.app_paths,
        options.namespace,
        runtime_config.scan_cache_policy,
        options.stale_only,
        options.limit.map(std::num::NonZeroUsize::get),
        dry_run,
    );

    crate::output::print_command_success(
        "cache prune",
        "cache-prune-report",
        options.output_mode,
        || &report,
        || {
            print!("{}", render_cache_prune_report(&report)?);
            Ok(())
        },
    )
}

pub fn purge(options: CachePurgeOptions) -> Result<()> {
    let paths = load_app_paths()?;
    if options.permanent && (options.dry_run || !options.yes) {
        return Err(anyhow!(
            "--permanent requires --yes and cannot be combined with --dry-run"
        ));
    }

    let mode = if options.yes && !options.dry_run {
        if options.permanent {
            CachePurgeMode::PermanentDelete
        } else {
            CachePurgeMode::RecoverableDelete
        }
    } else {
        CachePurgeMode::DryRun
    };
    let report = match mode {
        CachePurgeMode::DryRun | CachePurgeMode::PermanentDelete => purge_app_cache(&paths, mode)?,
        CachePurgeMode::RecoverableDelete => purge_app_cache_recoverably(&paths)?,
    };

    crate::output::print_command_success(
        "cache purge",
        "cache-purge-report",
        options.output_mode,
        || &report,
        || {
            print!("{}", render_cache_purge_report(&report)?);
            Ok(())
        },
    )
}

#[cfg(feature = "windows")]
fn purge_app_cache_recoverably(
    paths: &rebecca::core::config::AppPaths,
) -> Result<CachePurgeReport> {
    let backend = rebecca::windows::WindowsRecycleBinBackend::new();
    Ok(purge_app_cache_with_backend(
        paths,
        CachePurgeMode::RecoverableDelete,
        &backend,
    )?)
}

#[cfg(not(feature = "windows"))]
fn purge_app_cache_recoverably(
    _paths: &rebecca::core::config::AppPaths,
) -> Result<CachePurgeReport> {
    Err(anyhow!(
        "recoverable cache purge requires a platform recycle-bin backend; rerun with --yes --permanent to delete permanently"
    ))
}

fn render_cache_purge_report(report: &CachePurgeReport) -> Result<String> {
    let projection = CachePurgeProjection::new(report);
    let mut output = String::new();
    writeln!(output, "Rebecca cache: {}", projection.cache_dir.display())?;
    writeln!(output, "Mode: {}", projection.mode_label)?;
    writeln!(
        output,
        "Lifecycle: {} ({})",
        projection.lifecycle_label, projection.retention_label
    )?;
    writeln!(
        output,
        "Cache directory exists: {}",
        projection.cache_dir_exists_label
    )?;
    writeln!(
        output,
        "Preserves cache directory: {}",
        projection.preserves_cache_dir_label
    )?;
    writeln!(
        output,
        "Entries: {}, files: {}, directories: {}",
        projection.summary.total_entries, projection.summary.files, projection.summary.directories
    )?;
    writeln!(
        output,
        "Entry status: {} would delete, {} recoverably deleted, {} permanently deleted, {} skipped, {} failed",
        projection.summary.would_delete_entries,
        projection.summary.recoverably_deleted_entries,
        projection.summary.permanently_deleted_entries,
        projection.summary.skipped_entries,
        projection.summary.failed_entries
    )?;
    writeln!(
        output,
        "Estimated bytes: {} ({})",
        projection.summary.estimated_bytes,
        format_bytes(projection.summary.estimated_bytes)
    )?;
    writeln!(
        output,
        "Reclaimed bytes: {} ({})",
        projection.summary.reclaimed_bytes,
        format_bytes(projection.summary.reclaimed_bytes)
    )?;
    writeln!(
        output,
        "Pending reclaim bytes: {} ({})",
        projection.summary.pending_reclaim_bytes,
        format_bytes(projection.summary.pending_reclaim_bytes)
    )?;

    if !projection.issue_matrix().is_empty() {
        writeln!(output, "Issue matrix:")?;
        for issue in projection.issue_matrix() {
            writeln!(
                output,
                "- {} {}: {}, {} ({})",
                issue.status_label,
                issue.reason_label,
                issue.entries_label,
                issue.estimated_bytes,
                format_bytes(issue.estimated_bytes)
            )?;
        }
    }

    if projection.is_empty() {
        writeln!(output, "No cache entries found.")?;
        return Ok(output);
    }

    if projection.show_delete_hint() {
        writeln!(
            output,
            "Run with --yes to move these rebuildable cache entries to the Recycle Bin, or --yes --permanent to delete them permanently."
        )?;
    }

    writeln!(output, "Cache entries:")?;
    for entry in projection.entries() {
        writeln!(
            output,
            "- {}: {} ({}; {} file(s), {} dir(s)){}",
            entry.status_label,
            entry.path.display(),
            format_bytes(entry.estimated_bytes),
            entry.files,
            entry.directories,
            entry.reason_suffix
        )?;
    }

    Ok(output)
}

fn render_cache_inventory(inventory: &CacheInventory) -> Result<String> {
    let mut output = String::new();
    writeln!(output, "Rebecca cache: {}", inventory.cache_dir.display())?;
    writeln!(output, "Namespace: {}", inventory.namespace.label())?;
    writeln!(
        output,
        "Entries: {}, valid: {}, stale: {}, corrupt: {}, missing payloads: {}, prunable: {}",
        inventory.summary.total_entries,
        inventory.summary.valid_entries,
        inventory.summary.stale_entries,
        inventory.summary.corrupt_entries,
        inventory.summary.missing_payloads,
        inventory.summary.prunable_entries
    )?;
    writeln!(
        output,
        "Cache bytes: {} ({})",
        inventory.summary.bytes,
        format_bytes(inventory.summary.bytes)
    )?;

    if inventory.entries.is_empty() {
        writeln!(output, "No cache records found.")?;
    } else {
        writeln!(output, "Cache records:")?;
        for entry in &inventory.entries {
            let reason = entry
                .reason_code
                .as_deref()
                .map(|reason| format!("; {reason}"))
                .unwrap_or_default();
            writeln!(
                output,
                "- {} {}: {} ({}; {}){}",
                entry.namespace.label(),
                cache_status_label(entry.status),
                entry.display_path.display(),
                entry.bytes,
                format_bytes(entry.bytes),
                reason
            )?;
        }
    }

    if !inventory.diagnostics.is_empty() {
        writeln!(output, "Diagnostics:")?;
        for diagnostic in &inventory.diagnostics {
            writeln!(
                output,
                "- {}: {} ({})",
                diagnostic.reason_code,
                diagnostic.display_path.display(),
                diagnostic.message
            )?;
        }
    }

    Ok(output)
}

fn render_cache_doctor_report(report: &CacheDoctorReport) -> Result<String> {
    let mut output = String::new();
    writeln!(output, "Cache health: {}", cache_health_label(report))?;
    writeln!(
        output,
        "Prunable records: {}",
        report.inventory.summary.prunable_entries
    )?;
    if let Some(parts) = report
        .recommendations
        .iter()
        .filter_map(|recommendation| recommendation.suggested_command.as_ref())
        .next()
    {
        writeln!(
            output,
            "Recommended next command: {}",
            format_shell_command("rebecca", parts)
        )?;
    }
    writeln!(output)?;
    output.push_str(&render_cache_inventory(&report.inventory)?);
    if report.recommendations.is_empty() {
        writeln!(output, "Recommendations: none")?;
    } else {
        writeln!(output, "Recommendations:")?;
        for recommendation in &report.recommendations {
            let command = recommendation
                .suggested_command
                .as_ref()
                .map(|parts| format!(" [{}]", parts.join(" ")))
                .unwrap_or_default();
            writeln!(
                output,
                "- {:?}: {}{}",
                recommendation.severity, recommendation.message, command
            )?;
        }
    }
    Ok(output)
}

fn cache_health_label(report: &CacheDoctorReport) -> &'static str {
    if report.inventory.summary.prunable_entries > 0 {
        "needs pruning"
    } else if report.recommendations.is_empty() {
        "healthy"
    } else {
        "review recommended"
    }
}

fn render_cache_prune_report(report: &CachePruneReport) -> Result<String> {
    let mut output = String::new();
    writeln!(output, "Rebecca cache: {}", report.cache_dir.display())?;
    writeln!(output, "Namespace: {}", report.namespace.label())?;
    writeln!(
        output,
        "Mode: {}",
        if report.dry_run { "dry-run" } else { "delete" }
    )?;
    writeln!(
        output,
        "Stale only: {}",
        if report.stale_only { "yes" } else { "no" }
    )?;
    writeln!(
        output,
        "Selected records: {}",
        report.selected_entries.len()
    )?;
    writeln!(
        output,
        "Execution: {} completed, {} skipped, {} failed, {} bytes reclaimed",
        report.execution_report.summary.completed_actions,
        report.execution_report.summary.skipped_actions,
        report.execution_report.summary.failed_actions,
        report.execution_report.summary.confirmed_reclaimed_bytes
    )?;
    if report.selected_entries.is_empty() {
        writeln!(output, "No cache records selected.")?;
    } else {
        writeln!(output, "Selected cache records:")?;
        for entry in &report.selected_entries {
            writeln!(
                output,
                "- {} {}: {} ({})",
                entry.namespace.label(),
                cache_status_label(entry.status),
                entry.display_path.display(),
                format_bytes(entry.bytes)
            )?;
        }
    }
    if report.dry_run && !report.selected_entries.is_empty() {
        writeln!(
            output,
            "Run with --yes to delete the selected cache metadata records."
        )?;
    }
    Ok(output)
}

fn cache_status_label(status: CacheInventoryEntryStatus) -> &'static str {
    match status {
        CacheInventoryEntryStatus::Valid => "valid",
        CacheInventoryEntryStatus::Stale => "stale",
        CacheInventoryEntryStatus::Corrupt => "corrupt",
        CacheInventoryEntryStatus::Unreadable => "unreadable",
        CacheInventoryEntryStatus::MissingPayload => "missing-payload",
        CacheInventoryEntryStatus::Payload => "payload",
        CacheInventoryEntryStatus::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca::core::cache::{
        CachePurgeEntry, CachePurgeEntryKind, CachePurgeEntryReason, CachePurgeEntryStatus,
        CachePurgeIssueSummary, CachePurgeSummary,
    };
    use rebecca::core::config::{AppStorageLifecycle, AppStorageRetention};

    use super::{CachePurgeMode, CachePurgeReport, render_cache_purge_report};

    #[test]
    fn render_cache_purge_report_includes_issue_matrix_when_present() {
        let report = CachePurgeReport {
            cache_dir: PathBuf::from("cache"),
            cache_dir_lifecycle: AppStorageLifecycle::RebuildableCache,
            cache_dir_retention: AppStorageRetention::Rebuildable,
            cache_dir_exists: true,
            preserves_cache_dir: true,
            mode: CachePurgeMode::DryRun,
            deleted: false,
            summary: CachePurgeSummary {
                total_entries: 1,
                would_delete_entries: 0,
                deleted_entries: 0,
                skipped_entries: 1,
                failed_entries: 0,
                files: 0,
                directories: 0,
                estimated_bytes: 0,
                reclaimed_bytes: 0,
                pending_reclaim_bytes: 0,
                recoverably_deleted_entries: 0,
                permanently_deleted_entries: 0,
                issue_matrix: vec![CachePurgeIssueSummary {
                    status: CachePurgeEntryStatus::Skipped,
                    reason_code: CachePurgeEntryReason::SymlinkSkipped,
                    entries: 1,
                    estimated_bytes: 0,
                }],
            },
            entries: vec![CachePurgeEntry {
                path: PathBuf::from("cache/link"),
                kind: CachePurgeEntryKind::Symlink,
                status: CachePurgeEntryStatus::Skipped,
                estimated_bytes: 0,
                reclaimed_bytes: 0,
                pending_reclaim_bytes: 0,
                files: 0,
                directories: 0,
                reason: Some("symlink entries are skipped".to_string()),
                reason_code: Some(CachePurgeEntryReason::SymlinkSkipped),
            }],
            execution_report: Default::default(),
        };

        let rendered = render_cache_purge_report(&report).unwrap();

        assert!(rendered.contains("Issue matrix:"));
        assert!(rendered.contains("- skipped symlink-skipped: 1 entry, 0 (0 B)"));
        assert!(
            rendered.contains(
                "Run with --yes to move these rebuildable cache entries to the Recycle Bin"
            )
        );
    }

    #[test]
    fn render_cache_purge_report_omits_empty_issue_matrix() {
        let report = CachePurgeReport {
            cache_dir: PathBuf::from("cache"),
            cache_dir_lifecycle: AppStorageLifecycle::RebuildableCache,
            cache_dir_retention: AppStorageRetention::Rebuildable,
            cache_dir_exists: false,
            preserves_cache_dir: true,
            mode: CachePurgeMode::DryRun,
            deleted: false,
            summary: CachePurgeSummary::default(),
            entries: Vec::new(),
            execution_report: Default::default(),
        };

        let rendered = render_cache_purge_report(&report).unwrap();

        assert!(!rendered.contains("Issue matrix:"));
        assert!(rendered.contains("No cache entries found."));
    }
}
