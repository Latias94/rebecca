use std::collections::BTreeMap;
use std::path::PathBuf;

use rebecca::core::cleanup_advice::{CleanupAdvice, CleanupAdviceStatus};
use rebecca::core::plan::CleanupPlan;
use rebecca::core::scan::ScanBackendKind;
use rebecca::core::warnings::normalize_warning_gate;

use crate::workbench::CleanupWorkbenchRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupBasketItem {
    pub(crate) rule_id: String,
    pub(crate) status: CleanupAdviceStatus,
    pub(crate) reason: String,
    pub(crate) required_flags: Vec<String>,
    pub(crate) required_warnings: Vec<String>,
    pub(crate) source_path: PathBuf,
    pub(crate) source_logical_bytes: u64,
    pub(crate) source_files: u64,
    pub(crate) source_directories: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupBasketSource {
    pub(crate) path: PathBuf,
    pub(crate) logical_bytes: u64,
    pub(crate) files: u64,
    pub(crate) directories: u64,
}

pub(crate) type CleanupBasket = BTreeMap<String, CleanupBasketItem>;

pub(crate) fn toggle_advice(
    basket: &mut CleanupBasket,
    advice: Option<&CleanupAdvice>,
    source: CleanupBasketSource,
) -> String {
    let Some(advice) = advice else {
        return "Selected entry has no cleanup advice to add.".to_string();
    };
    if !stageable_advice(advice) {
        return format!("{} entries cannot be added.", advice.status.label());
    }
    let Some(rule_id) = advice.rule_id.as_ref() else {
        return "This advice is not backed by a cleanup rule yet.".to_string();
    };

    if basket.remove(rule_id).is_some() {
        return format!("Removed {rule_id} from the Reclaim Basket.");
    }

    basket.insert(
        rule_id.clone(),
        CleanupBasketItem {
            rule_id: rule_id.clone(),
            status: advice.status,
            reason: advice.reason.clone(),
            required_flags: advice.required_flags.clone(),
            required_warnings: advice.required_warnings.clone(),
            source_path: source.path,
            source_logical_bytes: source.logical_bytes,
            source_files: source.files,
            source_directories: source.directories,
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
    use std::path::PathBuf;

    use rebecca::core::cleanup_advice::{CleanupAdviceCommand, CleanupAdviceSource};

    use super::*;

    #[test]
    fn toggle_advice_stages_and_unstages_rule_backed_advice() {
        let mut basket = CleanupBasket::new();
        let advice = advice(CleanupAdviceStatus::Cleanable, Some("linux.user-temp"));

        assert_eq!(
            toggle_advice(&mut basket, Some(&advice), source()),
            "Added linux.user-temp to the Reclaim Basket; preview will expand matching targets."
        );
        assert!(basket.contains_key("linux.user-temp"));

        assert_eq!(
            toggle_advice(&mut basket, Some(&advice), source()),
            "Removed linux.user-temp from the Reclaim Basket."
        );
        assert!(basket.is_empty());
    }

    #[test]
    fn toggle_advice_rejects_missing_unstageable_or_unbacked_advice() {
        let mut basket = CleanupBasket::new();

        assert_eq!(
            toggle_advice(&mut basket, None, source()),
            "Selected entry has no cleanup advice to add."
        );
        assert_eq!(
            toggle_advice(
                &mut basket,
                Some(&advice(
                    CleanupAdviceStatus::Protected,
                    Some("linux.user-temp")
                )),
                source()
            ),
            "protected entries cannot be added."
        );
        assert_eq!(
            toggle_advice(
                &mut basket,
                Some(&advice(CleanupAdviceStatus::Cleanable, None)),
                source()
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
            source_path: PathBuf::from("/tmp/cache"),
            source_logical_bytes: 42,
            source_files: 2,
            source_directories: 1,
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

    fn source() -> CleanupBasketSource {
        CleanupBasketSource {
            path: PathBuf::from("/tmp/cache"),
            logical_bytes: 42,
            files: 2,
            directories: 1,
        }
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
