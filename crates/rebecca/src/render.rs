pub(crate) mod clean;
pub(crate) mod purge;

use rebecca::core::EstimateSource;

pub(crate) use crate::text::format_count;

pub(crate) fn estimate_source_suffix(estimate_source: EstimateSource) -> String {
    match estimate_source {
        EstimateSource::ScanCache => " [estimate: scan-cache]".to_string(),
        EstimateSource::Unknown => " [estimate: unknown]".to_string(),
        EstimateSource::FreshScan | EstimateSource::NotMeasured => String::new(),
    }
}
