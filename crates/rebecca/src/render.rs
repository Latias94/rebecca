pub(crate) mod clean;
pub(crate) mod inspect;
pub(crate) mod purge;

use rebecca_core::{EstimateProvenance, EstimateSource};

pub(crate) use crate::text::format_count;

pub(crate) fn estimate_provenance_suffix(
    estimate_source: EstimateSource,
    provenance: &EstimateProvenance,
) -> String {
    if !provenance.has_human_visible_detail(estimate_source) {
        return String::new();
    }

    let mut parts = Vec::new();
    match estimate_source {
        EstimateSource::ScanCache | EstimateSource::Unknown => {
            parts.push(estimate_source.label().to_string())
        }
        EstimateSource::FreshScan | EstimateSource::NotMeasured => {}
    }
    if let Some(backend) = provenance.estimate_backend {
        parts.push(backend.label().to_string());
    }
    if let Some(source) = &provenance.estimate_backend_source {
        parts.push(format!("source={source}"));
    }
    if let Some(confidence) = provenance.estimate_confidence {
        parts.push(confidence.label().to_string());
    }
    if let Some(reason) = &provenance.estimate_fallback_reason {
        parts.push(reason.clone());
    }
    if !provenance.estimate_caveats.is_empty() {
        parts.extend(
            provenance
                .estimate_caveats
                .iter()
                .map(|caveat| caveat.code.clone()),
        );
    }

    format!(" [estimate: {}]", parts.join(", "))
}
