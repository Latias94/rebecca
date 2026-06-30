use serde::{Deserialize, Serialize};

pub const ACTIVE_PROCESS_WARNING: &str = "active-process";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WarningSummary {
    pub warning: String,
    pub targets: usize,
    pub estimated_bytes: u64,
}

pub fn normalize_warning_gate(warning: &str) -> String {
    warning.trim().to_ascii_lowercase()
}

pub fn missing_warning_gates<'a>(
    warnings: impl IntoIterator<Item = &'a String>,
    allowed_warnings: &[String],
) -> Vec<String> {
    let mut missing = Vec::new();

    for warning in warnings {
        if allowed_warnings
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(warning))
        {
            continue;
        }

        if !missing
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(warning))
        {
            missing.push(warning.clone());
        }
    }

    missing
}

pub fn warning_gate_required_reason(warnings: &[String]) -> String {
    match warnings {
        [] => "warning gate required".to_string(),
        [warning] => format!("warning gate requires --allow-warning {warning}"),
        _ => format!(
            "warning gates require --allow-warning for: {}",
            warnings.join(", ")
        ),
    }
}
