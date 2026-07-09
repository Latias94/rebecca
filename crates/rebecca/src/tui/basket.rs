use std::collections::BTreeMap;

use rebecca_core::cleanup_advice::{
    CleanupAdviceAction, CleanupAdviceActionKind, CleanupAdviceStatus,
};
use rebecca_core::plan::CleanupPlan;
use rebecca_core::scan::ScanBackendKind;
use rebecca_core::warnings::normalize_warning_gate;

use crate::workbench::CleanupWorkbenchRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupBasketItem {
    pub(crate) rule_id: String,
    pub(crate) status: CleanupAdviceStatus,
    pub(crate) reason: String,
    pub(crate) required_flags: Vec<String>,
    pub(crate) required_warnings: Vec<String>,
    pub(crate) source_path: std::path::PathBuf,
    pub(crate) source_logical_bytes: u64,
    pub(crate) source_files: u64,
    pub(crate) source_directories: u64,
    pub(crate) covered_path_count: u64,
}

pub(crate) type CleanupBasket = BTreeMap<String, CleanupBasketItem>;

pub(crate) fn toggle_action(
    basket: &mut CleanupBasket,
    action: Option<&CleanupAdviceAction>,
) -> String {
    let Some(action) = action else {
        return "Selected entry has no cleanup action to add.".to_string();
    };
    if action.kind != CleanupAdviceActionKind::RebeccaCommand || !stageable_status(action.status) {
        return format!("{} items cannot be added.", action.kind.label());
    }
    let Some(rule_id) = action.rule_id.as_ref() else {
        return "This cleanup action is not backed by a cleanup rule yet.".to_string();
    };

    if basket.remove(rule_id).is_some() {
        return format!("Removed {rule_id} from the Reclaim Basket.");
    }

    basket.insert(
        rule_id.clone(),
        CleanupBasketItem {
            rule_id: rule_id.clone(),
            status: action.status,
            reason: action.reason.clone(),
            required_flags: action.required_flags.clone(),
            required_warnings: action.required_warnings.clone(),
            source_path: action.owner_path.clone(),
            source_logical_bytes: action.logical_bytes,
            source_files: 0,
            source_directories: 0,
            covered_path_count: action.covered_path_count,
        },
    );
    format!("Added {rule_id} to the Reclaim Basket; preview will expand matching targets.")
}

pub(crate) fn workbench_request(
    basket: &CleanupBasket,
    scan_backend: ScanBackendKind,
) -> CleanupWorkbenchRequest {
    let mut allowed_warnings: Vec<String> = Vec::new();
    let mut allow_moderate = false;
    let mut allow_risky = false;
    for item in basket.values() {
        for flag in &item.required_flags {
            match flag.as_str() {
                "--allow-moderate" => allow_moderate = true,
                "--allow-risky" => allow_risky = true,
                _ => {}
            }
        }
        for warning in &item.required_warnings {
            let warning = normalize_warning_gate(warning);
            if warning.is_empty() {
                continue;
            }
            if allowed_warnings
                .iter()
                .all(|existing| !existing.eq_ignore_ascii_case(&warning))
            {
                allowed_warnings.push(warning);
            }
        }
    }

    CleanupWorkbenchRequest {
        selected_rule_ids: basket.keys().cloned().collect(),
        allow_moderate,
        allow_risky,
        allowed_warnings,
        scan_cache: true,
        scan_backend,
        exclude_paths: Vec::new(),
    }
}

pub(crate) fn confirmation_phrase(plan: Option<&CleanupPlan>) -> String {
    let bytes = plan.map(|plan| plan.summary.estimated_bytes).unwrap_or(0);
    format!("CLEAN {bytes}")
}

pub(crate) fn label(item: &CleanupBasketItem) -> String {
    let mut label = format!(
        "{} [{}] {}",
        item.rule_id,
        item.status.label(),
        crate::output::format_bytes(item.source_logical_bytes)
    );
    if !item.required_flags.is_empty() {
        label.push_str(" flags:");
        label.push_str(&item.required_flags.join(","));
    }
    if !item.required_warnings.is_empty() {
        label.push_str(" warnings:");
        label.push_str(&item.required_warnings.join(","));
    }
    label
}

pub(crate) fn source_summary(item: &CleanupBasketItem) -> String {
    if item.source_files == 0 && item.source_directories == 0 {
        return format!(
            "{} | {}",
            item.source_path.display(),
            crate::text::format_count(item.covered_path_count, "measured path", "measured paths")
        );
    }
    format!(
        "{} | {}, {}",
        item.source_path.display(),
        crate::text::format_count(item.source_files, "file", "files"),
        crate::text::format_count(item.source_directories, "dir", "dirs")
    )
}

pub(crate) fn total_source_logical_bytes(basket: &CleanupBasket) -> u64 {
    basket.values().map(|item| item.source_logical_bytes).sum()
}

fn stageable_status(status: CleanupAdviceStatus) -> bool {
    matches!(
        status,
        CleanupAdviceStatus::Cleanable
            | CleanupAdviceStatus::MaybeCleanable
            | CleanupAdviceStatus::ContainsCleanable
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca_core::cleanup_advice::{CleanupAdviceCommand, CleanupAdviceSource};

    use super::*;

    #[test]
    fn toggle_action_stages_and_unstages_rule_backed_action() {
        let mut basket = CleanupBasket::new();
        let action = action(
            CleanupAdviceActionKind::RebeccaCommand,
            CleanupAdviceStatus::Cleanable,
            Some("linux.user-temp"),
        );

        assert_eq!(
            toggle_action(&mut basket, Some(&action)),
            "Added linux.user-temp to the Reclaim Basket; preview will expand matching targets."
        );
        assert!(basket.contains_key("linux.user-temp"));

        assert_eq!(
            toggle_action(&mut basket, Some(&action)),
            "Removed linux.user-temp from the Reclaim Basket."
        );
        assert!(basket.is_empty());
    }

    #[test]
    fn toggle_action_rejects_missing_unstageable_or_unbacked_actions() {
        let mut basket = CleanupBasket::new();

        assert_eq!(
            toggle_action(&mut basket, None),
            "Selected entry has no cleanup action to add."
        );
        assert_eq!(
            toggle_action(
                &mut basket,
                Some(&action(
                    CleanupAdviceActionKind::Protected,
                    CleanupAdviceStatus::Protected,
                    Some("linux.user-temp")
                ))
            ),
            "protected items cannot be added."
        );
        assert_eq!(
            toggle_action(
                &mut basket,
                Some(&action(
                    CleanupAdviceActionKind::RebeccaCommand,
                    CleanupAdviceStatus::Cleanable,
                    None
                ))
            ),
            "This cleanup action is not backed by a cleanup rule yet."
        );
        assert!(basket.is_empty());
    }

    #[test]
    fn toggle_action_rejects_manual_review_even_when_rule_backed() {
        let mut basket = CleanupBasket::new();
        let action = action(
            CleanupAdviceActionKind::ManualReview,
            CleanupAdviceStatus::ReviewOnly,
            Some("workspace.git"),
        );

        assert_eq!(
            toggle_action(&mut basket, Some(&action)),
            "manual-review items cannot be added."
        );

        assert!(basket.is_empty());
    }

    #[test]
    fn label_includes_flags_and_warnings_when_present() {
        let item = CleanupBasketItem {
            rule_id: "linux.user-temp".to_string(),
            status: CleanupAdviceStatus::MaybeCleanable,
            reason: "temporary files".to_string(),
            required_flags: vec!["--allow-moderate".to_string()],
            required_warnings: vec!["active-process".to_string()],
            source_path: PathBuf::from("/tmp/cache"),
            source_logical_bytes: 42,
            source_files: 2,
            source_directories: 1,
            covered_path_count: 1,
        };

        assert_eq!(
            label(&item),
            "linux.user-temp [maybe-cleanable] 42 B flags:--allow-moderate warnings:active-process"
        );
        assert_eq!(source_summary(&item), "/tmp/cache | 2 files, 1 dir");
        let mut basket = CleanupBasket::new();
        basket.insert(item.rule_id.clone(), item);
        assert_eq!(total_source_logical_bytes(&basket), 42);
    }

    #[test]
    fn workbench_request_carries_supported_safety_gates_from_basket() {
        let mut basket = CleanupBasket::new();
        basket.insert(
            "linux.user-temp".to_string(),
            CleanupBasketItem {
                rule_id: "linux.user-temp".to_string(),
                status: CleanupAdviceStatus::MaybeCleanable,
                reason: "temporary files".to_string(),
                required_flags: vec![
                    "--allow-moderate".to_string(),
                    "--allow-risky".to_string(),
                    "--min-age-days 0".to_string(),
                ],
                required_warnings: vec![
                    " Active-Process ".to_string(),
                    "browser-profile".to_string(),
                    "active-process".to_string(),
                ],
                source_path: PathBuf::from("/tmp/cache"),
                source_logical_bytes: 42,
                source_files: 2,
                source_directories: 1,
                covered_path_count: 1,
            },
        );

        let request = workbench_request(&basket, ScanBackendKind::PortableRecursive);

        assert!(request.allow_moderate);
        assert!(request.allow_risky);
        assert_eq!(
            request.allowed_warnings,
            ["active-process".to_string(), "browser-profile".to_string()]
        );
    }

    fn action(
        kind: CleanupAdviceActionKind,
        status: CleanupAdviceStatus,
        rule_id: Option<&str>,
    ) -> CleanupAdviceAction {
        CleanupAdviceAction {
            id: "fixture-action".to_string(),
            kind,
            status,
            source: Some(CleanupAdviceSource::CleanupRule),
            rule_id: rule_id.map(str::to_string),
            category: None,
            owner_path: PathBuf::from("/tmp/cache"),
            sample_paths: vec![PathBuf::from("/tmp/cache")],
            sample_path_count: 1,
            omitted_sample_path_count: 0,
            covered_path_count: 1,
            logical_bytes: 42,
            allocated_bytes: None,
            unique_logical_bytes: None,
            unique_allocated_bytes: None,
            required_flags: Vec::new(),
            required_warnings: Vec::new(),
            suggested_command: (kind == CleanupAdviceActionKind::RebeccaCommand).then(|| {
                CleanupAdviceCommand {
                    command: "rebecca".to_string(),
                    args: vec![
                        "clean".to_string(),
                        "--rule".to_string(),
                        rule_id.unwrap_or("linux.user-temp").to_string(),
                    ],
                }
            }),
            manual_guidance: None,
            reason: "fixture".to_string(),
        }
    }
}
