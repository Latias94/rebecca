use std::collections::BTreeMap;

use rebecca::core::cleanup_advice::{CleanupAdvice, CleanupAdviceStatus};
use rebecca::core::plan::CleanupPlan;
use rebecca::core::scan::ScanBackendKind;

use crate::workbench::CleanupWorkbenchRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupBasketItem {
    pub(crate) rule_id: String,
    pub(crate) status: CleanupAdviceStatus,
    pub(crate) reason: String,
    pub(crate) required_flags: Vec<String>,
    pub(crate) required_warnings: Vec<String>,
}

pub(crate) type CleanupBasket = BTreeMap<String, CleanupBasketItem>;

pub(crate) fn toggle_advice(basket: &mut CleanupBasket, advice: Option<&CleanupAdvice>) -> String {
    let Some(advice) = advice else {
        return "Selected entry has no cleanup advice to stage.".to_string();
    };
    if !stageable_advice(advice) {
        return format!("{} entries cannot be staged.", advice.status.label());
    }
    let Some(rule_id) = advice.rule_id.as_ref() else {
        return "This advice is not backed by a cleanup rule yet.".to_string();
    };

    if basket.remove(rule_id).is_some() {
        return format!("Unstaged rule {rule_id}.");
    }

    basket.insert(
        rule_id.clone(),
        CleanupBasketItem {
            rule_id: rule_id.clone(),
            status: advice.status,
            reason: advice.reason.clone(),
            required_flags: advice.required_flags.clone(),
            required_warnings: advice.required_warnings.clone(),
        },
    );
    format!("Staged rule {rule_id}; preview covers all matching targets.")
}

pub(crate) fn workbench_request(
    basket: &CleanupBasket,
    scan_backend: ScanBackendKind,
) -> CleanupWorkbenchRequest {
    CleanupWorkbenchRequest {
        selected_rule_ids: basket.keys().cloned().collect(),
        allow_moderate: false,
        allow_risky: false,
        allowed_warnings: Vec::new(),
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
    let mut label = format!("{} [{}]", item.rule_id, item.status.label());
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

fn stageable_advice(advice: &CleanupAdvice) -> bool {
    matches!(
        advice.status,
        CleanupAdviceStatus::Cleanable
            | CleanupAdviceStatus::MaybeCleanable
            | CleanupAdviceStatus::ContainsCleanable
    )
}

#[cfg(test)]
mod tests {
    use rebecca::core::cleanup_advice::{CleanupAdviceCommand, CleanupAdviceSource};

    use super::*;

    #[test]
    fn toggle_advice_stages_and_unstages_rule_backed_advice() {
        let mut basket = CleanupBasket::new();
        let advice = advice(CleanupAdviceStatus::Cleanable, Some("linux.user-temp"));

        assert_eq!(
            toggle_advice(&mut basket, Some(&advice)),
            "Staged rule linux.user-temp; preview covers all matching targets."
        );
        assert!(basket.contains_key("linux.user-temp"));

        assert_eq!(
            toggle_advice(&mut basket, Some(&advice)),
            "Unstaged rule linux.user-temp."
        );
        assert!(basket.is_empty());
    }

    #[test]
    fn toggle_advice_rejects_missing_unstageable_or_unbacked_advice() {
        let mut basket = CleanupBasket::new();

        assert_eq!(
            toggle_advice(&mut basket, None),
            "Selected entry has no cleanup advice to stage."
        );
        assert_eq!(
            toggle_advice(
                &mut basket,
                Some(&advice(
                    CleanupAdviceStatus::Protected,
                    Some("linux.user-temp")
                ))
            ),
            "protected entries cannot be staged."
        );
        assert_eq!(
            toggle_advice(
                &mut basket,
                Some(&advice(CleanupAdviceStatus::Cleanable, None))
            ),
            "This advice is not backed by a cleanup rule yet."
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
        };

        assert_eq!(
            label(&item),
            "linux.user-temp [maybe-cleanable] flags:--allow-moderate warnings:active-process"
        );
    }

    fn advice(status: CleanupAdviceStatus, rule_id: Option<&str>) -> CleanupAdvice {
        CleanupAdvice {
            status,
            source: Some(CleanupAdviceSource::CleanupRule),
            relation: None,
            reason: "fixture".to_string(),
            rule_id: rule_id.map(str::to_string),
            category: None,
            safety_level: None,
            required_flags: Vec::new(),
            required_warnings: Vec::new(),
            protection_kind: None,
            matched_path: None,
            app_leftover: None,
            evidence: Vec::new(),
            suggested_command: Some(CleanupAdviceCommand {
                command: "rebecca".to_string(),
                args: vec![
                    "clean".to_string(),
                    "--rule".to_string(),
                    rule_id.unwrap_or("linux.user-temp").to_string(),
                ],
            }),
        }
    }
}
