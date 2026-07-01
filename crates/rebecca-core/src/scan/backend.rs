use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

use super::{ScanCancellationToken, ScanProgressEvent, ScanReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanBackendKind {
    PortableRecursive,
    WindowsNative,
}

impl ScanBackendKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PortableRecursive => "portable-recursive",
            Self::WindowsNative => "windows-native",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasuredScan {
    pub report: ScanReport,
    pub backend: ScanBackendKind,
    pub confidence: ScanEstimateConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caveats: Vec<ScanEstimateCaveat>,
}

impl MeasuredScan {
    pub(crate) fn exact(report: ScanReport, backend: ScanBackendKind) -> Self {
        Self {
            report,
            backend,
            confidence: ScanEstimateConfidence::Exact,
            fallback_reason: None,
            caveats: Vec::new(),
        }
    }

    pub(crate) fn with_fallback_reason(mut self, reason: impl Into<String>) -> Self {
        self.fallback_reason = Some(reason.into());
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
