pub(crate) mod clean;
pub(crate) mod inspect;
pub(crate) mod purge;

use rebecca::core::{EstimateProvenance, EstimateSource};

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
        EstimateSource::ScanCache | EstimateSource::Unknown => parts.push(estimate_source.label()),
        EstimateSource::FreshScan | EstimateSource::NotMeasured => {}
    }
    if let Some(backend) = provenance.estimate_backend {
        parts.push(backend.label());
    }
    if let Some(confidence) = provenance.estimate_confidence {
        parts.push(confidence.label());
    }
    if let Some(reason) = &provenance.estimate_fallback_reason {
        parts.push(reason.as_str());
    }
    if !provenance.estimate_caveats.is_empty() {
        parts.extend(
            provenance
                .estimate_caveats
                .iter()
                .map(|caveat| caveat.code.as_str()),
        );
    }

    format!(" [estimate: {}]", parts.join(", "))
}
