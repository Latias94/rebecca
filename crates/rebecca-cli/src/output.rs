use anyhow::Result;
use rebecca_core::plan::{CleanupPlan, CleanupTarget};
use rebecca_core::{DeleteMode, RuleDefinition, TargetStatus};

const LARGEST_TARGET_LIMIT: usize = 5;

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
                "  - {} [{}] {}",
                rule.id,
                rule.safety_level.label(),
                rule.name
            );
        }
    }
}

pub fn print_plan(plan: &CleanupPlan, json: bool) -> Result<()> {
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

        println!("{} ({})", status_label(status), targets.len());
        for target in targets {
            print_target_line(target, "  -");
        }
    }
}

fn print_target_line(target: &CleanupTarget, prefix: &str) {
    println!(
        "{prefix} {} [{}] {} bytes ({}){}",
        target.rule_id,
        target.path.display(),
        target.estimated_bytes,
        format_bytes(target.estimated_bytes),
        target
            .reason
            .as_ref()
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default()
    );
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

fn cleanup_mode_label(mode: DeleteMode) -> &'static str {
    match mode {
        DeleteMode::DryRun => "dry-run",
        DeleteMode::RecycleBin => "recycle-bin",
        DeleteMode::Permanent => "permanent",
    }
}

fn status_label(status: TargetStatus) -> &'static str {
    match status {
        TargetStatus::Allowed => "allowed",
        TargetStatus::Skipped => "skipped",
        TargetStatus::Blocked => "blocked",
        TargetStatus::Failed => "failed",
        TargetStatus::Completed => "completed",
    }
}
