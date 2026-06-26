use anyhow::Result;
use rebecca_core::plan::{CleanupIssueSummary, CleanupPlan};
use rebecca_core::{CleanupWorkflow, RuleDefinition};

use crate::clean_view::{CleanPlanProjection, CleanTargetRow, ScanCacheProgressSummary};
use crate::purge_view::{ProjectArtifactPlanProjection, ProjectArtifactRow};

pub fn print_rule_catalog(rules: &[&RuleDefinition]) {
    println!("Rebecca rules: {}", rules.len());

    if rules.is_empty() {
        println!("No built-in rules match the current selection.");
        return;
    }

    let mut grouped: std::collections::BTreeMap<String, Vec<&RuleDefinition>> =
        std::collections::BTreeMap::new();
    for rule in rules {
        grouped
            .entry(rule.category.clone())
            .or_default()
            .push(*rule);
    }

    for rules in grouped.values_mut() {
        rules.sort_by(|left, right| left.id.cmp(&right.id));
    }

    for (category, rules) in grouped {
        println!("- {} ({})", category, rules.len());
        for rule in rules {
            println!(
                "  - {} [{}] {}{}",
                rule.id,
                rule.safety_level.label(),
                rule.name,
                restore_hint_suffix(rule.restore_hint.as_deref())
            );
        }
    }
}

pub(crate) fn restore_hint_suffix<I, S>(restore_hints: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut unique_hints = Vec::new();

    for hint in restore_hints {
        let hint = hint.as_ref();
        if !unique_hints.iter().any(|existing| existing == hint) {
            unique_hints.push(hint.to_string());
        }
    }

    if unique_hints.is_empty() {
        String::new()
    } else {
        format!(" [restore: {}]", unique_hints.join("; "))
    }
}

pub(crate) fn print_plan(
    plan: &CleanupPlan,
    json: bool,
    scan_cache_summary: Option<ScanCacheProgressSummary>,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
        return Ok(());
    }

    let projection = CleanPlanProjection::new(plan, scan_cache_summary);

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
    if let Some(summary) = projection.scan_cache_summary() {
        println!(
            "Scan cache summary: {}, {}, {}",
            summary.hits_label, summary.misses_label, summary.write_skipped_label
        );
    }

    if plan.request.workflow == CleanupWorkflow::ProjectArtifacts {
        print_project_artifact_details(plan);
        return Ok(());
    }

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

fn print_project_artifact_details(plan: &CleanupPlan) {
    let projection = ProjectArtifactPlanProjection::new(plan);

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

fn print_recent_project_artifact_line(target: &ProjectArtifactRow<'_>, prefix: &str) {
    println!(
        "{prefix} {} [{}] {}{}",
        target.artifact_type,
        target.status_label,
        target.path.display(),
        target
            .reason
            .map(|reason| format!(" - {reason}"))
            .unwrap_or_default(),
    );
}

fn print_project_artifact_line(target: &ProjectArtifactRow<'_>, prefix: &str) {
    println!(
        "{prefix} {} [{}] {} bytes ({}) - {}{}{}",
        target.artifact_type,
        target.status_label,
        target.estimated_bytes,
        format_bytes(target.estimated_bytes),
        target.path.display(),
        target
            .reason
            .map(|reason| format!(" (reason: {reason})"))
            .unwrap_or_default(),
        restore_hint_suffix(target.restore_hint)
    );
}

fn print_target_line(target: &CleanTargetRow<'_>, prefix: &str) {
    println!(
        "{prefix} {} [{}] {} bytes ({}){}{}",
        target.rule_id,
        target.path.display(),
        target.estimated_bytes,
        format_bytes(target.estimated_bytes),
        target
            .reason
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default(),
        restore_hint_suffix(target.restore_hint)
    );
}

pub(crate) fn format_issue_matrix_entry(issue: &CleanupIssueSummary) -> String {
    format!(
        "{} {}: {}, {} ({})",
        issue.status.label(),
        issue.reason_code.label(),
        format_count(issue.targets as u64, "target", "targets"),
        issue.estimated_bytes,
        format_bytes(issue.estimated_bytes)
    )
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    format!("{value:.2} {}", UNITS[unit_index])
}

fn format_count(count: u64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

#[cfg(test)]
mod tests {
    use super::restore_hint_suffix;

    #[test]
    fn restore_hint_suffix_deduplicates_and_formats_hints() {
        assert_eq!(
            restore_hint_suffix([
                "Steam web caches will be rebuilt on launch.",
                "Steam web caches will be rebuilt on launch.",
                "Steam download staging data will be recreated if needed.",
            ]),
            " [restore: Steam web caches will be rebuilt on launch.; Steam download staging data will be recreated if needed.]"
        );
    }
}
