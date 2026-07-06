use anyhow::Result;
use rebecca::core::plan::CleanupPlan;

use crate::clean_view::{CleanPlanProjection, ScanCacheProgressSummary};
use crate::output::{format_bytes, restore_hint_suffix};
use crate::purge_view::{
    ProjectArtifactDiscoveryDiagnosticRow, ProjectArtifactInsightReport,
    ProjectArtifactPlanProjection, ProjectArtifactRow,
};
use crate::render::{estimate_provenance_suffix, format_count};

const PROJECT_ARTIFACT_DIAGNOSTIC_LIMIT: usize = 5;

pub(crate) fn print_plan(
    plan: &CleanupPlan,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
) -> Result<()> {
    let overview = CleanPlanProjection::new(plan, scan_cache_summary);
    super::clean::print_plan_overview(&overview);
    print_project_artifact_details(plan);
    Ok(())
}

pub(crate) fn print_project_artifact_insight(
    plan: &CleanupPlan,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
) -> Result<()> {
    let insight = ProjectArtifactInsightReport::from_plan(plan);

    println!("Project artifact insight");
    println!(
        "Roots: {}",
        format_count(insight.roots.len() as u64, "root", "roots")
    );
    println!("Targets: {}", insight.summary.total_targets);
    println!(
        "Estimated bytes: {} ({})",
        insight.summary.estimated_bytes,
        format_bytes(insight.summary.estimated_bytes)
    );
    println!(
        "Diagnostics: {}",
        format_count(
            insight.discovery_diagnostics.len() as u64,
            "observation",
            "observations",
        )
    );
    if let Some(summary) = scan_cache_summary.filter(has_visible_scan_cache_summary) {
        println!(
            "Scan cache summary: {} {}, {} {}, {} {}, {} {}",
            summary.hits,
            if summary.hits == 1 { "hit" } else { "hits" },
            summary.misses,
            if summary.misses == 1 {
                "miss"
            } else {
                "misses"
            },
            summary.write_skipped,
            if summary.write_skipped == 1 {
                "skipped write"
            } else {
                "skipped writes"
            },
            summary.pruned,
            if summary.pruned == 1 {
                "pruned record"
            } else {
                "pruned records"
            }
        );
    }

    if !insight.totals_by_artifact.is_empty() {
        println!();
        println!("Artifact totals:");
        for total in &insight.totals_by_artifact {
            println!(
                "- {}: {}, {} bytes ({})",
                total.label,
                total.targets,
                total.estimated_bytes,
                format_bytes(total.estimated_bytes)
            );
        }
    }

    if !insight.top_targets.is_empty() {
        println!();
        println!("Top project artifact targets:");
        for target in &insight.top_targets {
            println!(
                "  - {} [{}] {} bytes ({}) [{}] - {}",
                target.artifact,
                target.status.label(),
                target.estimated_bytes,
                format_bytes(target.estimated_bytes),
                target.estimate_source.label(),
                target.path.display()
            );
        }
    }

    if !insight.discovery_diagnostics.is_empty() {
        println!();
        println!("Discovery diagnostics:");
        for diagnostic in &insight.discovery_diagnostics {
            println!(
                "  - {} {} - {}",
                diagnostic.kind.label(),
                diagnostic.path.display(),
                diagnostic.detail
            );
        }
    }

    Ok(())
}

fn print_project_artifact_details(plan: &CleanupPlan) {
    let projection = ProjectArtifactPlanProjection::new(plan);

    if !projection.discovery_diagnostics().is_empty() {
        println!();
        println!(
            "Project artifact discovery diagnostics: {}",
            format_count(
                projection.discovery_diagnostics().len() as u64,
                "observation",
                "observations",
            )
        );
        println!("Partial discovery may have skipped some paths.");
        for diagnostic in projection
            .discovery_diagnostics()
            .iter()
            .take(PROJECT_ARTIFACT_DIAGNOSTIC_LIMIT)
        {
            print_project_artifact_diagnostic_line(diagnostic, "  -");
        }
        let remaining = projection
            .discovery_diagnostics()
            .len()
            .saturating_sub(PROJECT_ARTIFACT_DIAGNOSTIC_LIMIT);
        if remaining > 0 {
            println!(
                "  - ... {} more",
                format_count(remaining as u64, "observation", "observations")
            );
        }
    }

    if !projection.artifact_summaries().is_empty() {
        println!();
        println!("Project artifact summary:");
        for summary in projection.artifact_summaries() {
            println!(
                "- {}: {}, {} bytes ({}) [{}]",
                summary.artifact_type,
                summary.targets_label,
                summary.estimated_bytes,
                format_bytes(summary.estimated_bytes),
                summary.status_summary_label
            );
        }
    }

    if !projection.largest_targets().is_empty() {
        println!();
        println!("Largest project artifact targets:");
        for target in projection.largest_targets() {
            print_project_artifact_line(target, "  -");
        }
    }

    if !projection.recently_modified().is_empty() {
        println!();
        println!("Recently modified artifacts:");
        for target in projection.recently_modified() {
            print_recent_project_artifact_line(target, "  -");
        }
    }

    if projection.project_groups().is_empty() {
        return;
    }

    println!();
    println!("Project artifact details:");
    for group in projection.project_groups() {
        println!(
            "{} ({}, {} bytes ({}) estimated)",
            group.project_path.display(),
            group.targets_label,
            group.estimated_bytes,
            format_bytes(group.estimated_bytes)
        );
        for target in &group.targets {
            print_project_artifact_line(target, "  -");
        }
    }
}

fn print_project_artifact_diagnostic_line(
    diagnostic: &ProjectArtifactDiscoveryDiagnosticRow<'_>,
    prefix: &str,
) {
    println!(
        "{prefix} {} {} - {}",
        diagnostic.kind_label,
        diagnostic.path.display(),
        diagnostic.detail
    );
}

fn print_recent_project_artifact_line(target: &ProjectArtifactRow<'_>, prefix: &str) {
    println!(
        "{prefix} {} [{}] {}{}{}",
        target.artifact_type,
        target.status_label,
        target.path.display(),
        target
            .modified_at_unix_seconds
            .map(|seconds| format!(" (modified at {seconds})"))
            .unwrap_or_default(),
        target
            .reason
            .map(|reason| format!(" - {reason}"))
            .unwrap_or_default(),
    );
}

fn print_project_artifact_line(target: &ProjectArtifactRow<'_>, prefix: &str) {
    println!(
        "{prefix} {} [{}] {} bytes ({}){} - {}{}{}{}",
        target.artifact_type,
        target.status_label,
        target.estimated_bytes,
        format_bytes(target.estimated_bytes),
        estimate_provenance_suffix(target.estimate_source, target.estimate_provenance),
        target.path.display(),
        target
            .modified_at_unix_seconds
            .map(|seconds| format!(" (modified at {seconds})"))
            .unwrap_or_default(),
        target
            .reason
            .map(|reason| format!(" (reason: {reason})"))
            .unwrap_or_default(),
        restore_hint_suffix(target.restore_hint)
    );
}

fn has_visible_scan_cache_summary(summary: &ScanCacheProgressSummary) -> bool {
    summary.hits > 0 || summary.misses > 0 || summary.write_skipped > 0 || summary.pruned > 0
}
