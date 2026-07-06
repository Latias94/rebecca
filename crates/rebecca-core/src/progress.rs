use std::path::Path;

use crate::error::Result;
use crate::scan::ScanBackendKind;

pub type InspectProgressResult = Result<()>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InspectProgressDetail {
    #[default]
    Target,
    File,
}

impl InspectProgressDetail {
    pub const fn includes_file_events(self) -> bool {
        matches!(self, Self::File)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InspectProgressOptions {
    detail: InspectProgressDetail,
}

impl InspectProgressOptions {
    pub const fn target() -> Self {
        Self {
            detail: InspectProgressDetail::Target,
        }
    }

    pub const fn file() -> Self {
        Self {
            detail: InspectProgressDetail::File,
        }
    }

    pub const fn detail(self) -> InspectProgressDetail {
        self.detail
    }

    pub const fn includes_file_events(self) -> bool {
        self.detail.includes_file_events()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectProgressRootStatus {
    Scanned,
    Skipped,
}

impl InspectProgressRootStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Scanned => "scanned",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectProgressCacheEvent {
    Hit,
    Miss,
    WriteSkipped,
}

impl InspectProgressCacheEvent {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::WriteSkipped => "write-skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectProgressCounterKind {
    Files,
    Directories,
    Bytes,
    Records,
}

impl InspectProgressCounterKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Directories => "directories",
            Self::Bytes => "bytes",
            Self::Records => "records",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum InspectProgressEvent<'a> {
    RootStarted {
        root_index: usize,
        root_count: usize,
        root: &'a Path,
        backend: ScanBackendKind,
    },
    RootFinished {
        root_index: usize,
        root_count: usize,
        root: &'a Path,
        status: InspectProgressRootStatus,
        logical_bytes: u64,
        files: u64,
        directories: u64,
    },
    EntryStarted {
        root: &'a Path,
        path: &'a Path,
        entry_index: u64,
        backend: ScanBackendKind,
    },
    EntryMeasured {
        root: &'a Path,
        path: &'a Path,
        entry_index: u64,
        logical_bytes: u64,
        files: u64,
        directories: u64,
    },
    FileMeasured {
        root: &'a Path,
        target_path: &'a Path,
        path: &'a Path,
        file_size: u64,
        files_scanned: u64,
        bytes_scanned: u64,
    },
    TraversalProgress {
        root: &'a Path,
        counter: InspectProgressCounterKind,
        value: u64,
        logical_bytes: u64,
        files: u64,
        directories: u64,
    },
    BackendFallback {
        root: &'a Path,
        backend: ScanBackendKind,
        reason: &'a str,
    },
    BackendStageStarted {
        root: &'a Path,
        backend: ScanBackendKind,
        stage: &'static str,
    },
    BackendStageFinished {
        root: &'a Path,
        backend: ScanBackendKind,
        stage: &'static str,
    },
    BackendMetric {
        root: &'a Path,
        backend: ScanBackendKind,
        metric: &'static str,
        value: u64,
    },
    CacheEvent {
        path: &'a Path,
        event: InspectProgressCacheEvent,
        reason: Option<&'static str>,
        estimated_bytes: Option<u64>,
    },
    Finalizing {
        roots: usize,
        logical_bytes: u64,
        files: u64,
        directories: u64,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PowerOfTwoProgressSampler {
    next_files: u64,
    next_directories: u64,
    next_bytes: u64,
    next_records: u64,
}

impl Default for PowerOfTwoProgressSampler {
    fn default() -> Self {
        Self {
            next_files: 1,
            next_directories: 1,
            next_bytes: 1,
            next_records: 1,
        }
    }
}

impl PowerOfTwoProgressSampler {
    pub(crate) fn should_emit(&mut self, counter: InspectProgressCounterKind, value: u64) -> bool {
        let next = match counter {
            InspectProgressCounterKind::Files => &mut self.next_files,
            InspectProgressCounterKind::Directories => &mut self.next_directories,
            InspectProgressCounterKind::Bytes => &mut self.next_bytes,
            InspectProgressCounterKind::Records => &mut self.next_records,
        };
        if value < *next {
            return false;
        }

        while *next <= value {
            *next = next.saturating_mul(2).max(next.saturating_add(1));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_of_two_sampler_emits_at_milestones() {
        let mut sampler = PowerOfTwoProgressSampler::default();

        assert!(sampler.should_emit(InspectProgressCounterKind::Files, 1));
        assert!(sampler.should_emit(InspectProgressCounterKind::Files, 2));
        assert!(!sampler.should_emit(InspectProgressCounterKind::Files, 3));
        assert!(sampler.should_emit(InspectProgressCounterKind::Files, 4));
        assert!(sampler.should_emit(InspectProgressCounterKind::Files, 9));
        assert!(!sampler.should_emit(InspectProgressCounterKind::Files, 10));
    }
}
