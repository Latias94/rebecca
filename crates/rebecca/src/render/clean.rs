use anyhow::Result;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::{
    CleanupWorkflow, DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH, DEFAULT_PROJECT_ARTIFACT_MIN_AGE_DAYS,
    DeleteMode, PlanRequest,
};

use crate::clean_view::{CleanPlanProjection, CleanTargetRow, ScanCacheProgressSummary};
use crate::output::{format_bytes, format_shell_command, restore_hint_suffix};
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
    print_plan_decision(projection);
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

fn print_plan_decision(projection: &CleanPlanProjection<'_>) {
    println!();
    println!("Decision: {}", decision_label(projection));
    println!(
        "Reclaimable now: {} ({})",
        reclaimable_now_bytes(projection),
        format_bytes(reclaimable_now_bytes(projection))
    );
    println!("Execution: {}", execution_label(projection));

    if projection.mode.is_dry_run() && projection.summary.allowed_targets > 0 {
        println!(
            "Next command: {}",
            suggested_execution_command(projection.request)
        );
    }

    if !projection.warning_matrix().is_empty() {
        let warnings = projection
            .warning_matrix()
            .iter()
            .map(|warning| warning.warning.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("Warning gates in plan: {warnings}.");
    }
}

fn decision_label(projection: &CleanPlanProjection<'_>) -> &'static str {
    match projection.mode {
        DeleteMode::DryRun => "preview only; no files were deleted.",
        DeleteMode::RecycleBin if projection.summary.failed_targets > 0 => {
            "cleanup finished with failures."
        }
        DeleteMode::RecycleBin if projection.summary.completed_targets > 0 => "cleanup executed.",
        DeleteMode::RecycleBin => "no cleanup target was executed.",
    }
}

fn execution_label(projection: &CleanPlanProjection<'_>) -> &'static str {
    match projection.mode {
        DeleteMode::DryRun if projection.summary.allowed_targets > 0 => {
            "would move allowed targets to the Recycle Bin."
        }
        DeleteMode::DryRun => "no eligible target would be deleted.",
        DeleteMode::RecycleBin if projection.summary.pending_reclaim_bytes > 0 => {
            "moved allowed targets to the Recycle Bin; empty it to reclaim pending bytes."
        }
        DeleteMode::RecycleBin if projection.summary.completed_targets > 0 => {
            "moved allowed targets to the Recycle Bin."
        }
        DeleteMode::RecycleBin => "nothing was moved to the Recycle Bin.",
    }
}

fn reclaimable_now_bytes(projection: &CleanPlanProjection<'_>) -> u64 {
    match projection.mode {
        DeleteMode::DryRun => projection
            .target_groups()
            .iter()
            .find(|group| group.status_label == "allowed")
            .map(|group| {
                group
                    .targets
                    .iter()
                    .map(|target| target.estimated_bytes)
                    .sum()
            })
            .unwrap_or(0),
        DeleteMode::RecycleBin => projection
            .summary
            .freed_bytes
            .saturating_add(projection.summary.pending_reclaim_bytes),
    }
}

fn suggested_execution_command(request: &PlanRequest) -> String {
    let mut args = match request.workflow {
        CleanupWorkflow::Rules => vec!["clean".to_string(), "--yes".to_string()],
        CleanupWorkflow::AppLeftovers => {
            vec!["apps".to_string(), "clean".to_string(), "--yes".to_string()]
        }
        CleanupWorkflow::ProjectArtifacts => vec!["purge".to_string(), "--yes".to_string()],
    };

    match request.workflow {
        CleanupWorkflow::Rules => {
            push_repeated_option(&mut args, "--category", &request.selected_categories);
            push_repeated_option(&mut args, "--rule", &request.selected_rule_ids);
            push_risk_options(&mut args, request);
        }
        CleanupWorkflow::AppLeftovers => {}
        CleanupWorkflow::ProjectArtifacts => {
            for root in &request.project_artifact_roots {
                args.push("--root".to_string());
                args.push(root.display().to_string());
            }
            if request.project_artifact_max_depth != DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH {
                args.push("--max-depth".to_string());
                args.push(request.project_artifact_max_depth.to_string());
            }
            if request.project_artifact_min_age_days != DEFAULT_PROJECT_ARTIFACT_MIN_AGE_DAYS {
                args.push("--min-age-days".to_string());
                args.push(request.project_artifact_min_age_days.to_string());
            }
            if let Some(limit) = request.project_artifact_reclaim_limit_bytes {
                args.push("--reclaim-limit-bytes".to_string());
                args.push(limit.to_string());
            }
            push_repeated_option(&mut args, "--artifact", &request.project_artifact_selectors);
        }
    }

    format_shell_command("rebecca", &args)
}

fn push_repeated_option(args: &mut Vec<String>, flag: &str, values: &[String]) {
    for value in values {
        args.push(flag.to_string());
        args.push(value.clone());
    }
}

fn push_risk_options(args: &mut Vec<String>, request: &PlanRequest) {
    if request.allow_risky {
        args.push("--allow-risky".to_string());
    } else if request.allow_moderate {
        args.push("--allow-moderate".to_string());
    }

    push_repeated_option(args, "--allow-warning", &request.allowed_warnings);
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
