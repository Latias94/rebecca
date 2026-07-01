use anyhow::Result;
use rebecca::core::plan::CleanupPlan;

use crate::clean_view::{CleanPlanProjection, CleanTargetRow, ScanCacheProgressSummary};
use crate::output::{format_bytes, restore_hint_suffix};
use crate::render::estimate_provenance_suffix;

pub(crate) fn print_plan(
    plan: &CleanupPlan,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
) -> Result<()> {
    let projection = CleanPlanProjection::new(plan, scan_cache_summary);

    print_plan_overview(&projection);

    if !projection.largest_targets().is_empty() {
        println!();
        println!("Largest estimated targets:");
        for target in projection.largest_targets() {
            print_target_line(target, "  -");
        }
    }

    if !projection.target_groups().is_empty() {
        println!();
        println!("Target details:");
        for group in projection.target_groups() {
            println!("{} ({})", group.status_label, group.targets.len());
            for target in &group.targets {
                print_target_line(target, "  -");
            }
        }
    }

    Ok(())
}

pub(super) fn print_plan_overview(projection: &CleanPlanProjection<'_>) {
    println!("Workflow: {}", projection.workflow_title);
    println!("Cleanup mode: {}", projection.mode_label);
    println!("Targets: {}", projection.summary.total_targets);
    println!("Allowed: {}", projection.summary.allowed_targets);
    println!("Skipped: {}", projection.summary.skipped_targets);
    println!("Blocked: {}", projection.summary.blocked_targets);
    println!("Failed: {}", projection.summary.failed_targets);
    println!("Completed: {}", projection.summary.completed_targets);
    println!(
        "Estimated bytes: {} ({})",
        projection.summary.estimated_bytes,
        format_bytes(projection.summary.estimated_bytes)
    );
    println!(
        "Freed bytes: {} ({})",
        projection.summary.freed_bytes,
        format_bytes(projection.summary.freed_bytes)
    );
    println!(
        "Pending reclaim bytes: {} ({})",
        projection.summary.pending_reclaim_bytes,
        format_bytes(projection.summary.pending_reclaim_bytes)
    );
    if !projection.issue_matrix().is_empty() {
        println!();
        println!("Issue matrix:");
        for issue in projection.issue_matrix() {
            println!(
                "- {} {}: {}, {} ({})",
                issue.status_label,
                issue.reason_label,
                issue.targets_label,
                issue.estimated_bytes,
                format_bytes(issue.estimated_bytes)
            );
        }
    }
    if !projection.warning_matrix().is_empty() {
        println!();
        println!("Warning matrix:");
        for warning in projection.warning_matrix() {
            println!(
                "- {}: {}, {} ({})",
                warning.warning,
                warning.targets_label,
                warning.estimated_bytes,
                format_bytes(warning.estimated_bytes)
            );
        }
    }
    if let Some(summary) = projection.scan_cache_summary() {
        println!(
            "Scan cache summary: {}, {}, {}, {}",
            summary.hits_label,
            summary.misses_label,
            summary.write_skipped_label,
            summary.pruned_label
        );
    }
}

fn print_target_line(target: &CleanTargetRow<'_>, prefix: &str) {
    println!(
        "{prefix} {} [{}] {} bytes ({}){}{}{}{}",
        target.rule_id,
        target.path.display(),
        target.estimated_bytes,
        format_bytes(target.estimated_bytes),
        estimate_provenance_suffix(target.estimate_source, target.estimate_provenance),
        target
            .reason
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default(),
        warning_suffix(target.warnings),
        restore_hint_suffix(target.restore_hint)
    );
}

fn warning_suffix(warnings: &[String]) -> String {
    if warnings.is_empty() {
        String::new()
    } else {
        format!(" [warnings: {}]", warnings.join(", "))
    }
}
