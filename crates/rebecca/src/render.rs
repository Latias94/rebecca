pub(crate) mod clean;
pub(crate) mod purge;

use rebecca::core::EstimateSource;

pub(crate) fn estimate_source_suffix(estimate_source: EstimateSource) -> String {
    match estimate_source {
        EstimateSource::ScanCache => " [estimate: scan-cache]".to_string(),
        EstimateSource::Unknown => " [estimate: unknown]".to_string(),
        EstimateSource::FreshScan | EstimateSource::NotMeasured => String::new(),
    }
}

pub(crate) fn format_count(count: u64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}
