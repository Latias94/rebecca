use anyhow::Result;
use rebecca_core::plan::{CleanupIssueSummary, CleanupPlan, CleanupTarget};
use rebecca_core::{DeleteMode, RuleDefinition, TargetStatus};

const LARGEST_TARGET_LIMIT: usize = 5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ScanCacheProgressSummary {
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) write_skipped: u64,
}

impl ScanCacheProgressSummary {
    fn has_activity(self) -> bool {
        self.hits > 0 || self.misses > 0 || self.write_skipped > 0
    }
}

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

    println!("Cleanup mode: {}", cleanup_mode_label(plan.request.mode));
    println!("Targets: {}", plan.summary.total_targets);
    println!("Allowed: {}", plan.summary.allowed_targets);
    println!("Skipped: {}", plan.summary.skipped_targets);
    println!("Blocked: {}", plan.summary.blocked_targets);
    println!("Failed: {}", plan.summary.failed_targets);
    println!("Completed: {}", plan.summary.completed_targets);
    println!(
        "Estimated bytes: {} ({})",
        plan.summary.estimated_bytes,
        format_bytes(plan.summary.estimated_bytes)
    );
    println!(
        "Freed bytes: {} ({})",
        plan.summary.freed_bytes,
        format_bytes(plan.summary.freed_bytes)
    );
    println!(
        "Pending reclaim bytes: {} ({})",
        plan.summary.pending_reclaim_bytes,
        format_bytes(plan.summary.pending_reclaim_bytes)
    );
    print_issue_matrix(&plan.summary.issue_matrix);
    if let Some(summary) = scan_cache_summary.filter(|summary| summary.has_activity()) {
        println!(
            "Scan cache summary: {}, {}, {}",
            format_count(summary.hits, "hit", "hits"),
            format_count(summary.misses, "miss", "misses"),
            format_count(summary.write_skipped, "skipped write", "skipped writes")
        );
    }

    print_largest_targets(plan);
    print_targets_by_status(plan);

    Ok(())
}

fn print_largest_targets(plan: &CleanupPlan) {
    let mut targets = plan
        .targets
        .iter()
        .filter(|target| target.estimated_bytes > 0)
        .collect::<Vec<_>>();

    targets.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.rule_id.cmp(&right.rule_id))
            .then_with(|| left.path.cmp(&right.path))
    });

    if targets.is_empty() {
        return;
    }

    println!();
    println!("Largest estimated targets:");
    for target in targets.into_iter().take(LARGEST_TARGET_LIMIT) {
        print_target_line(target, "  -");
    }
}

fn print_targets_by_status(plan: &CleanupPlan) {
    if plan.targets.is_empty() {
        return;
    }

    println!();
    println!("Target details:");

    for status in [
        TargetStatus::Allowed,
        TargetStatus::Completed,
        TargetStatus::Failed,
        TargetStatus::Blocked,
        TargetStatus::Skipped,
    ] {
        let targets = plan
            .targets
            .iter()
            .filter(|target| target.status == status)
            .collect::<Vec<_>>();

        if targets.is_empty() {
            continue;
        }

        println!("{} ({})", status.label(), targets.len());
        for target in targets {
            print_target_line(target, "  -");
        }
    }
}

fn print_target_line(target: &CleanupTarget, prefix: &str) {
    println!(
        "{prefix} {} [{}] {} bytes ({}){}{}",
        target.rule_id,
        target.path.display(),
        target.estimated_bytes,
        format_bytes(target.estimated_bytes),
        target
            .reason
            .as_ref()
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default(),
        restore_hint_suffix(target.restore_hint.as_deref())
    );
}

fn print_issue_matrix(issue_matrix: &[CleanupIssueSummary]) {
    if issue_matrix.is_empty() {
        return;
    }

    println!();
    println!("Issue matrix:");
    for issue in issue_matrix {
        println!(
            "- {} {}: {}, {} ({})",
            issue.status.label(),
            issue.reason_code.label(),
            format_count(issue.targets as u64, "target", "targets"),
            issue.estimated_bytes,
            format_bytes(issue.estimated_bytes)
        );
    }
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

fn cleanup_mode_label(mode: DeleteMode) -> &'static str {
    match mode {
        DeleteMode::DryRun => "dry-run",
        DeleteMode::RecycleBin => "recycle-bin",
        DeleteMode::Permanent => "permanent",
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
