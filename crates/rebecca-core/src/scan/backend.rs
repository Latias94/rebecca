use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

use super::{ScanCancellationToken, ScanProgressEvent, ScanReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanBackendKind {
    PortableRecursive,
    WindowsNative,
    WindowsNtfsMftExperimental,
}

impl ScanBackendKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PortableRecursive => "portable-recursive",
            Self::WindowsNative => "windows-native",
            Self::WindowsNtfsMftExperimental => "windows-ntfs-mft-experimental",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanEstimateConfidence {
    Exact,
}

impl ScanEstimateConfidence {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanEstimateCaveat {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanBackendEvidence {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub timings_ms: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cache_events: Vec<ScanCacheEvidenceEvent>,
}

impl ScanBackendEvidence {
    pub fn is_empty(&self) -> bool {
        self.timings_ms.is_empty() && self.counters.is_empty() && self.cache_events.is_empty()
    }

    pub fn merge(&mut self, other: Self) {
        self.timings_ms.extend(other.timings_ms);
        self.counters.extend(other.counters);
        self.cache_events.extend(other.cache_events);
    }

    pub fn record_cache_event(
        &mut self,
        cache: impl Into<String>,
        outcome: impl Into<String>,
        reason: Option<String>,
    ) {
        self.cache_events.push(ScanCacheEvidenceEvent {
            cache: cache.into(),
            outcome: outcome.into(),
            reason,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanCacheEvidenceEvent {
    pub cache: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasuredScan {
    pub report: ScanReport,
    pub backend: ScanBackendKind,
    pub confidence: ScanEstimateConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caveats: Vec<ScanEstimateCaveat>,
    #[serde(default, skip_serializing_if = "ScanBackendEvidence::is_empty")]
    pub backend_evidence: ScanBackendEvidence,
}

impl MeasuredScan {
    pub(crate) fn exact(report: ScanReport, backend: ScanBackendKind) -> Self {
        Self {
            report,
            backend,
            confidence: ScanEstimateConfidence::Exact,
            backend_source: None,
            fallback_reason: None,
            caveats: Vec::new(),
            backend_evidence: ScanBackendEvidence::default(),
        }
    }

    #[cfg(windows)]
    pub(crate) fn with_backend_source(mut self, source: impl Into<String>) -> Self {
        self.backend_source = Some(source.into());
        self
    }

    pub(crate) fn with_fallback_reason(mut self, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        self.fallback_reason = Some(match self.fallback_reason.take() {
            Some(existing) => format!("{reason}; {existing}"),
            None => reason,
        });
        self
    }

    pub(crate) fn with_caveat(
        mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        self.caveats.push(ScanEstimateCaveat {
            code: code.into(),
            message: message.into(),
        });
        self
    }

    #[cfg(windows)]
    pub(crate) fn with_backend_evidence(mut self, evidence: ScanBackendEvidence) -> Self {
        self.backend_evidence.merge(evidence);
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScanRequest<'a> {
    pub path: &'a Path,
    pub cancellation: &'a ScanCancellationToken,
}

impl<'a> ScanRequest<'a> {
    pub const fn new(path: &'a Path, cancellation: &'a ScanCancellationToken) -> Self {
        Self { path, cancellation }
    }
}

pub trait ScanBackend {
    fn kind(&self) -> ScanBackendKind;

    fn measure_path_with_progress<F>(
        &self,
        request: ScanRequest<'_>,
        progress: F,
    ) -> Result<MeasuredScan>
    where
        F: for<'a> FnMut(ScanProgressEvent<'a>);
}
