use std::collections::BTreeSet;

use anyhow::Result;
use rebecca_core::plan::CleanupPlan;
use rebecca_core::{
    CleanupWorkflow, DEFAULT_PROJECT_ARTIFACT_MAX_DEPTH, DEFAULT_PROJECT_ARTIFACT_MIN_AGE_DAYS,
    DeleteMode, PlanRequest, Platform,
};

use crate::clean_view::{CleanPlanProjection, CleanTargetRow, ScanCacheProgressSummary};
use crate::output::{format_bytes, format_shell_command, restore_hint_suffix};
use crate::render::{estimate_provenance_suffix, format_count};

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
    println!("{}", user_summary_line(projection));
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
            suggested_execution_command(projection.request, false)
        );
        println!(
            "Default delete: moves files to {}.",
            recoverable_destination_label(projection.request.platform)
        );
        println!(
            "Skip the trash: {}",
            suggested_execution_command(projection.request, true)
        );
        let opt_ins = required_opt_in_flags(projection.request);
        if !opt_ins.is_empty() {
            println!("Required opt-ins in next command: {}.", opt_ins.join(", "));
        }
    }

    if projection.mode == DeleteMode::RecoverableDelete
        && projection.summary.pending_reclaim_bytes > 0
    {
        println!("Preview pending trash space: rebecca trash empty");
        println!("Empty after review: rebecca trash empty --yes");
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

    if projection.mode.is_dry_run() {
        let guidance = pre_execution_guidance(projection);
        if !guidance.is_empty() {
            println!("Resolve before execution:");
            for line in guidance {
                println!("- {line}");
            }
        }

        for hint in doctor_hints(projection) {
            println!("Doctor hint: {hint}");
        }
    }
}

fn decision_label(projection: &CleanPlanProjection<'_>) -> &'static str {
    match projection.mode {
        DeleteMode::DryRun => "preview only; no files were deleted.",
        DeleteMode::RecoverableDelete if projection.summary.failed_targets > 0 => {
            "cleanup finished with failures."
        }
        DeleteMode::RecoverableDelete if projection.summary.completed_targets > 0 => {
            "cleanup executed."
        }
        DeleteMode::PermanentDelete if projection.summary.failed_targets > 0 => {
            "permanent cleanup finished with failures."
        }
        DeleteMode::PermanentDelete if projection.summary.completed_targets > 0 => {
            "permanent cleanup executed."
        }
        DeleteMode::RecoverableDelete => "no cleanup target was executed.",
        DeleteMode::PermanentDelete => "no cleanup target was executed.",
    }
}

fn user_summary_line(projection: &CleanPlanProjection<'_>) -> String {
    let target_label = format_usize_count(
        projection.summary.allowed_targets,
        "eligible target",
        "eligible targets",
    );
    let destination = recoverable_destination_label(projection.request.platform);
    match projection.mode {
        DeleteMode::DryRun if projection.summary.allowed_targets > 0 => format!(
            "Summary: {} can reclaim {} ({}); default execution moves them to {}.",
            target_label,
            reclaimable_now_bytes(projection),
            format_bytes(reclaimable_now_bytes(projection)),
            destination
        ),
        DeleteMode::DryRun => {
            "Summary: no eligible cleanup targets matched this request.".to_string()
        }
        DeleteMode::RecoverableDelete if projection.summary.completed_targets > 0 => format!(
            "Summary: moved {} to {}; {} ({}) is pending trash reclaim.",
            format_usize_count(projection.summary.completed_targets, "target", "targets"),
            destination,
            projection.summary.pending_reclaim_bytes,
            format_bytes(projection.summary.pending_reclaim_bytes)
        ),
        DeleteMode::RecoverableDelete => {
            format!("Summary: no target was moved to {destination}.")
        }
        DeleteMode::PermanentDelete if projection.summary.completed_targets > 0 => format!(
            "Summary: permanently deleted {}; freed {} ({}).",
            format_usize_count(projection.summary.completed_targets, "target", "targets"),
            projection.summary.freed_bytes,
            format_bytes(projection.summary.freed_bytes)
        ),
        DeleteMode::PermanentDelete => "Summary: no target was permanently deleted.".to_string(),
    }
}

fn format_usize_count(count: usize, singular: &str, plural: &str) -> String {
    format_count(count as u64, singular, plural)
}

fn execution_label(projection: &CleanPlanProjection<'_>) -> String {
    let destination = recoverable_destination_label(projection.request.platform);
    match projection.mode {
        DeleteMode::DryRun if projection.summary.allowed_targets > 0 => {
            format!("would move allowed targets to {destination}.")
        }
        DeleteMode::DryRun => "no eligible target would be deleted.".to_string(),
        DeleteMode::RecoverableDelete if projection.summary.pending_reclaim_bytes > 0 => {
            format!(
                "moved allowed targets to {destination}; preview trash before emptying pending space."
            )
        }
        DeleteMode::RecoverableDelete if projection.summary.completed_targets > 0 => {
            format!("moved allowed targets to {destination}.")
        }
        DeleteMode::RecoverableDelete => format!("nothing was moved to {destination}."),
        DeleteMode::PermanentDelete if projection.summary.completed_targets > 0 => {
            format!("deleted allowed targets permanently; this bypassed {destination}.")
        }
        DeleteMode::PermanentDelete => "nothing was deleted permanently.".to_string(),
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
        DeleteMode::RecoverableDelete => projection
            .summary
            .freed_bytes
            .saturating_add(projection.summary.pending_reclaim_bytes),
        DeleteMode::PermanentDelete => projection.summary.freed_bytes,
    }
}

fn suggested_execution_command(request: &PlanRequest, permanent: bool) -> String {
    let mut args = match request.workflow {
        CleanupWorkflow::Rules => vec!["clean".to_string(), "--yes".to_string()],
        CleanupWorkflow::AppLeftovers => {
            vec!["apps".to_string(), "clean".to_string(), "--yes".to_string()]
        }
        CleanupWorkflow::ProjectArtifacts => vec!["purge".to_string(), "--yes".to_string()],
    };
    if permanent {
        args.push("--permanent".to_string());
    }

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

fn recoverable_destination_label(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "the Windows Recycle Bin",
        Platform::Linux | Platform::Macos => "the system Trash",
        Platform::Unknown => "the system trash",
    }
}

fn required_opt_in_flags(request: &PlanRequest) -> Vec<String> {
    let mut flags = Vec::new();
    if request.allow_risky {
        flags.push("--allow-risky".to_string());
    } else if request.allow_moderate {
        flags.push("--allow-moderate".to_string());
    }

    flags.extend(
        request
            .allowed_warnings
            .iter()
            .map(|warning| format!("--allow-warning {warning}")),
    );
    flags
}

fn pre_execution_guidance(projection: &CleanPlanProjection<'_>) -> Vec<String> {
    let mut guidance = BTreeSet::new();
    for issue in projection.issue_matrix() {
        if let Some(message) = issue_resolution_hint(issue.reason_label) {
            guidance.insert(format!(
                "{} {}: {message}",
                issue.status_label, issue.reason_label
            ));
        }
    }
    guidance.into_iter().collect()
}

fn issue_resolution_hint(reason_label: &str) -> Option<&'static str> {
    match reason_label {
        "safety-opt-in-required" => {
            Some("add --allow-moderate or --allow-risky after reviewing the rule.")
        }
        "warning-gate-required" => Some(
            "add the named --allow-warning flag after checking active processes and app state.",
        ),
        "safety-policy-blocked" => {
            Some("blocked by protection policy; adjust --exclude or config only if intentional.")
        }
        "safety-policy-skipped" => Some(
            "excluded by protection policy; remove the matching exclude or protection only if intentional.",
        ),
        "duplicate-target-path" => {
            Some("already covered by another target; execute the allowed parent target instead.")
        }
        "target-discovery-skipped" => {
            Some("narrow the selection or fix discovery prerequisites before executing.")
        }
        "target-discovery-failed" | "scan-failed" => {
            Some("fix the scan error or narrow the selected rules before executing.")
        }
        "scan-permission-denied" => Some(
            "grant the required OS privacy access or run rebecca doctor permissions before retrying.",
        ),
        "execution-target-missing" => {
            Some("the path disappeared; rerun the preview before executing.")
        }
        "execution-target-shadowed" => {
            Some("the path changed during validation; rerun the preview before executing.")
        }
        "saved-plan-target-changed" => {
            Some("the saved plan is stale for this path; rerun the preview and save a fresh plan.")
        }
        "project-artifact-recently-modified" => {
            Some("lower --min-age-days only if the artifact is inactive.")
        }
        "reclaim-limit-satisfied" => {
            Some("raise --reclaim-limit-bytes if more artifacts should be considered.")
        }
        "execution-failed" => Some("review the failure, fix the path or permissions, and rerun."),
        "execution-permission-denied" => {
            Some("run rebecca doctor permissions and grant OS privacy access before retrying.")
        }
        "unclassified" => Some("review target details before executing."),
        _ => None,
    }
}

fn doctor_hints(projection: &CleanPlanProjection<'_>) -> Vec<&'static str> {
    if projection
        .warning_matrix()
        .iter()
        .any(|warning| warning.warning == "active-process")
    {
        vec!["rebecca doctor active-processes"]
    } else {
        Vec::new()
    }
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
