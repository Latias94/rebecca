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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanMetricSemantics {
    #[default]
    LogicalBytes,
    AllocatedBytes,
}

impl ScanMetricSemantics {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LogicalBytes => "logical-bytes",
            Self::AllocatedBytes => "allocated-bytes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanBackendFallbackKind {
    FeatureDisabled,
    UnsupportedPlatform,
    DisabledByEnvironment,
    PermissionDenied,
    NonLocalVolume,
    NonNtfsVolume,
    Timeout,
    ScanFailed,
    SafetyBlocked,
    BackendUnavailable,
    Unknown,
}

impl ScanBackendFallbackKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::FeatureDisabled => "feature-disabled",
            Self::UnsupportedPlatform => "unsupported-platform",
            Self::DisabledByEnvironment => "disabled-by-environment",
            Self::PermissionDenied => "permission-denied",
            Self::NonLocalVolume => "non-local-volume",
            Self::NonNtfsVolume => "non-ntfs-volume",
            Self::Timeout => "timeout",
            Self::ScanFailed => "scan-failed",
            Self::SafetyBlocked => "safety-blocked",
            Self::BackendUnavailable => "backend-unavailable",
            Self::Unknown => "unknown",
        }
    }

    pub const fn guidance(self) -> Option<&'static str> {
        match self {
            Self::FeatureDisabled => {
                Some("Use a Rebecca build with NTFS support, or select a safe fallback backend.")
            }
            Self::UnsupportedPlatform => Some("Use a platform-supported backend for this host."),
            Self::DisabledByEnvironment => {
                Some("Unset the test override or select a safe fallback backend.")
            }
            Self::PermissionDenied => Some(
                "Run from an elevated shell to use the NTFS/MFT backend, or use a safe fallback backend.",
            ),
            Self::NonLocalVolume => {
                Some("Select a local fixed NTFS volume or use a fallback backend.")
            }
            Self::NonNtfsVolume => Some("Select an NTFS volume or use a fallback backend."),
            Self::Timeout => {
                Some("Increase the NTFS/MFT timeout or use a safe fallback backend for this scan.")
            }
            Self::ScanFailed => {
                Some("Rebecca used a safe fallback backend after the selected scanner failed.")
            }
            Self::SafetyBlocked => {
                Some("Rebecca used a safe fallback path because the selected scanner was blocked.")
            }
            Self::BackendUnavailable => Some(
                "Rebecca used a safe fallback backend because the selected backend was unavailable.",
            ),
            Self::Unknown => None,
        }
    }

    pub fn from_reason(reason: &str) -> Self {
        let reason = reason.to_ascii_lowercase();
        if reason.contains("ntfs feature is disabled") || reason.contains("feature is disabled") {
            Self::FeatureDisabled
        } else if reason.contains("disabled by") {
            Self::DisabledByEnvironment
        } else if reason.contains("requires windows")
            || reason.contains("only available on windows")
            || reason.contains("not available on this platform")
        {
            Self::UnsupportedPlatform
        } else if reason.contains("permission denied") || reason.contains("access is denied") {
            Self::PermissionDenied
        } else if reason.contains("only indexes local fixed") {
            Self::NonLocalVolume
        } else if reason.contains("expected ntfs") || reason.contains("not ntfs") {
            Self::NonNtfsVolume
        } else if reason.contains("timed out") || reason.contains("timeout") {
            Self::Timeout
        } else if reason.contains("safety") && reason.contains("blocked") {
            Self::SafetyBlocked
        } else if reason.contains("scan failed") {
            Self::ScanFailed
        } else if reason.contains("unavailable") || reason.contains("not enabled") {
            Self::BackendUnavailable
        } else {
            Self::Unknown
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
    #[serde(default)]
    pub metric_semantics: ScanMetricSemantics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_kind: Option<ScanBackendFallbackKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_guidance: Option<String>,
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
            metric_semantics: ScanMetricSemantics::LogicalBytes,
            backend_source: None,
            fallback_reason: None,
            fallback_kind: None,
            fallback_guidance: None,
            caveats: Vec::new(),
            backend_evidence: ScanBackendEvidence::default(),
        }
    }

    pub fn with_metric_semantics(mut self, semantics: ScanMetricSemantics) -> Self {
        self.metric_semantics = semantics;
        self
    }

    #[cfg(all(windows, feature = "ntfs"))]
    pub(crate) fn with_backend_source(mut self, source: impl Into<String>) -> Self {
        self.backend_source = Some(source.into());
        self
    }

    pub(crate) fn with_fallback_reason(mut self, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let kind = ScanBackendFallbackKind::from_reason(&reason);
        self.fallback_kind.get_or_insert(kind);
        if self.fallback_guidance.is_none() {
            self.fallback_guidance = kind.guidance().map(str::to_string);
        }
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

    #[cfg(all(windows, feature = "ntfs"))]
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
