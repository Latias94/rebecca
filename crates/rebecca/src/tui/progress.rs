use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiTaskProgressEvent {
    RootStarted {
        root_index: usize,
        root_count: usize,
        root: PathBuf,
        backend: String,
    },
    RootFinished {
        root_index: usize,
        root_count: usize,
        root: PathBuf,
        status: String,
        logical_bytes: u64,
        files: u64,
        directories: u64,
    },
    Traversal {
        root: PathBuf,
        counter: String,
        value: u64,
        logical_bytes: u64,
        files: u64,
        directories: u64,
    },
    FileMeasured {
        target_path: PathBuf,
        path: PathBuf,
        file_size: u64,
        files_scanned: u64,
        bytes_scanned: u64,
    },
    BackendFallback {
        root: PathBuf,
        backend: String,
        reason: String,
    },
    BackendStage {
        root: PathBuf,
        backend: String,
        stage: &'static str,
        finished: bool,
    },
    BackendMetric {
        metric: &'static str,
        value: u64,
    },
    Finalizing {
        roots: usize,
        logical_bytes: u64,
        files: u64,
        directories: u64,
    },
    CleanupTargetScanning {
        rule_id: String,
        path: PathBuf,
    },
    CleanupTargetFinished {
        rule_id: String,
        path: PathBuf,
        status: String,
        estimated_bytes: u64,
    },
    CleanupFileMeasured {
        rule_id: String,
        target_path: PathBuf,
        path: PathBuf,
        file_size: u64,
        files_scanned: u64,
        bytes_scanned: u64,
    },
    CleanupCacheHit {
        rule_id: String,
        path: PathBuf,
        estimated_bytes: u64,
    },
    CleanupCacheMiss {
        rule_id: String,
        path: PathBuf,
        reason: String,
        pruned: bool,
    },
    CleanupCacheWriteSkipped {
        rule_id: String,
        path: PathBuf,
    },
    CleanupCachePruned {
        inspected: usize,
        pruned: usize,
        retained: usize,
    },
    ExecutionFinished {
        completed_targets: u64,
        freed_bytes: u64,
        pending_reclaim_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiTaskStatus {
    pub(crate) label: String,
    pub(crate) phase: String,
    pub(crate) backend: Option<String>,
    pub(crate) current_root: Option<PathBuf>,
    pub(crate) current_path: Option<PathBuf>,
    pub(crate) current_rule_id: Option<String>,
    pub(crate) roots_finished: usize,
    pub(crate) root_count: usize,
    pub(crate) logical_bytes: u64,
    pub(crate) files: u64,
    pub(crate) directories: u64,
    pub(crate) targets_started: u64,
    pub(crate) targets_finished: u64,
    pub(crate) estimated_bytes: u64,
    pub(crate) cache_hits: u64,
    pub(crate) cache_misses: u64,
    pub(crate) cache_write_skipped: u64,
    pub(crate) cache_pruned: usize,
    pub(crate) last_event: String,
    pub(crate) cancel_requested: bool,
}

impl TuiTaskStatus {
    pub(crate) fn started(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            phase: label.clone(),
            label,
            backend: None,
            current_root: None,
            current_path: None,
            current_rule_id: None,
            roots_finished: 0,
            root_count: 0,
            logical_bytes: 0,
            files: 0,
            directories: 0,
            targets_started: 0,
            targets_finished: 0,
            estimated_bytes: 0,
            cache_hits: 0,
            cache_misses: 0,
            cache_write_skipped: 0,
            cache_pruned: 0,
            last_event: String::new(),
            cancel_requested: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiTaskId(pub(crate) u64);
