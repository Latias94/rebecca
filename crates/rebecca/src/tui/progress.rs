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
    ExecutionStarted {
        executable_targets: usize,
        estimated_bytes: u64,
        mode: String,
    },
    ExecutionTargetStarted {
        rule_id: String,
        path: PathBuf,
        estimated_bytes: u64,
    },
    ExecutionTargetFinished {
        rule_id: String,
        path: PathBuf,
        status: String,
        freed_bytes: u64,
        pending_reclaim_bytes: u64,
    },
    ExecutionFinished {
        completed_targets: u64,
        freed_bytes: u64,
        pending_reclaim_bytes: u64,
    },
}

impl TuiTaskProgressEvent {
    pub(crate) fn is_coalescible(&self) -> bool {
        matches!(
            self,
            Self::Traversal { .. } | Self::FileMeasured { .. } | Self::CleanupFileMeasured { .. }
        )
    }
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

    pub(crate) fn apply_event(&mut self, event: TuiTaskProgressEvent) {
        match event {
            TuiTaskProgressEvent::RootStarted {
                root_index,
                root_count,
                root,
                backend,
            } => {
                self.phase = format!("Scanning root {}/{}", root_index + 1, root_count);
                self.backend = Some(backend);
                self.current_root = Some(root.clone());
                self.current_path = Some(root.clone());
                self.root_count = root_count;
                self.last_event = format!("Started {}", root.display());
            }
            TuiTaskProgressEvent::RootFinished {
                root_index,
                root_count,
                root,
                status: root_status,
                logical_bytes,
                files,
                directories,
            } => {
                self.phase = format!("Finished root {}/{}", root_index + 1, root_count);
                self.current_root = Some(root.clone());
                self.current_path = Some(root.clone());
                self.roots_finished = self.roots_finished.max(root_index + 1);
                self.root_count = root_count;
                self.logical_bytes = logical_bytes;
                self.files = files;
                self.directories = directories;
                self.last_event = format!("{root_status}: {}", root.display());
            }
            TuiTaskProgressEvent::Traversal {
                root,
                counter,
                value,
                logical_bytes,
                files,
                directories,
            } => {
                self.phase = format!("Walking {counter} {value}");
                self.current_root = Some(root);
                self.logical_bytes = logical_bytes;
                self.files = files;
                self.directories = directories;
                self.last_event = format!("{counter}: {value}");
            }
            TuiTaskProgressEvent::FileMeasured {
                target_path,
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            } => {
                self.phase = "Measuring files".to_string();
                self.current_root = Some(target_path.clone());
                self.current_path = Some(path.clone());
                self.files = files_scanned;
                self.logical_bytes = bytes_scanned;
                self.last_event = format!("{} ({file_size} bytes)", path.display());
            }
            TuiTaskProgressEvent::BackendFallback {
                root,
                backend,
                reason,
            } => {
                self.phase = "Backend fallback".to_string();
                self.backend = Some(backend.clone());
                self.current_root = Some(root.clone());
                self.current_path = Some(root.clone());
                self.last_event = format!("{backend}: {reason}");
            }
            TuiTaskProgressEvent::BackendStage {
                root,
                backend,
                stage,
                finished,
            } => {
                self.phase = if finished {
                    format!("{stage} finished")
                } else {
                    format!("{stage} started")
                };
                self.backend = Some(backend);
                self.current_root = Some(root.clone());
                self.current_path = Some(root);
                self.last_event = self.phase.clone();
            }
            TuiTaskProgressEvent::BackendMetric { metric, value } => {
                self.phase = "Reading backend metrics".to_string();
                self.last_event = format!("{metric}: {value}");
            }
            TuiTaskProgressEvent::Finalizing {
                roots,
                logical_bytes,
                files,
                directories,
            } => {
                self.phase = "Finalizing map".to_string();
                self.root_count = roots;
                self.roots_finished = roots;
                self.logical_bytes = logical_bytes;
                self.files = files;
                self.directories = directories;
                self.last_event = "Building ranked tree".to_string();
            }
            TuiTaskProgressEvent::CleanupTargetScanning { rule_id, path } => {
                self.phase = "Scanning cleanup target".to_string();
                self.current_rule_id = Some(rule_id.clone());
                self.current_path = Some(path.clone());
                self.targets_started = self.targets_started.saturating_add(1);
                self.last_event = format!("{rule_id}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupTargetFinished {
                rule_id,
                path,
                status: target_status,
                estimated_bytes,
            } => {
                self.phase = "Measured cleanup target".to_string();
                self.current_rule_id = Some(rule_id.clone());
                self.current_path = Some(path.clone());
                self.targets_finished = self.targets_finished.saturating_add(1);
                self.estimated_bytes = self.estimated_bytes.saturating_add(estimated_bytes);
                self.last_event = format!("{rule_id} {target_status}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupFileMeasured {
                rule_id,
                target_path,
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            } => {
                self.phase = "Measuring cleanup files".to_string();
                self.current_rule_id = Some(rule_id);
                self.current_root = Some(target_path);
                self.current_path = Some(path.clone());
                self.files = files_scanned;
                self.logical_bytes = bytes_scanned;
                self.last_event = format!("{} ({file_size} bytes)", path.display());
            }
            TuiTaskProgressEvent::CleanupCacheHit {
                rule_id,
                path,
                estimated_bytes,
            } => {
                self.phase = "Using scan cache".to_string();
                self.current_rule_id = Some(rule_id.clone());
                self.current_path = Some(path.clone());
                self.cache_hits = self.cache_hits.saturating_add(1);
                self.estimated_bytes = self.estimated_bytes.saturating_add(estimated_bytes);
                self.last_event = format!("{rule_id} cache hit: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupCacheMiss {
                rule_id,
                path,
                reason,
                pruned,
            } => {
                self.phase = "Refreshing scan cache".to_string();
                self.current_rule_id = Some(rule_id.clone());
                self.current_path = Some(path.clone());
                self.cache_misses = self.cache_misses.saturating_add(1);
                self.cache_pruned += usize::from(pruned);
                self.last_event = format!("{rule_id} cache miss {reason}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupCacheWriteSkipped { rule_id, path } => {
                self.phase = "Scan cache write skipped".to_string();
                self.current_rule_id = Some(rule_id.clone());
                self.current_path = Some(path.clone());
                self.cache_write_skipped = self.cache_write_skipped.saturating_add(1);
                self.last_event = format!("{rule_id}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupCachePruned {
                inspected,
                pruned,
                retained,
            } => {
                self.phase = "Pruning scan cache".to_string();
                self.cache_pruned = self.cache_pruned.saturating_add(pruned);
                self.last_event =
                    format!("cache inspected {inspected}, pruned {pruned}, retained {retained}");
            }
            TuiTaskProgressEvent::ExecutionStarted {
                executable_targets,
                estimated_bytes,
                mode,
            } => {
                self.phase = "Executing cleanup".to_string();
                self.targets_started = 0;
                self.targets_finished = 0;
                self.estimated_bytes = estimated_bytes;
                self.last_event = format!("{executable_targets} target(s), {mode}");
            }
            TuiTaskProgressEvent::ExecutionTargetStarted {
                rule_id,
                path,
                estimated_bytes,
            } => {
                self.phase = "Executing cleanup target".to_string();
                self.current_rule_id = Some(rule_id.clone());
                self.current_path = Some(path.clone());
                self.targets_started = self.targets_started.saturating_add(1);
                self.last_event =
                    format!("{rule_id}: {} ({estimated_bytes} bytes)", path.display());
            }
            TuiTaskProgressEvent::ExecutionTargetFinished {
                rule_id,
                path,
                status: target_status,
                freed_bytes,
                pending_reclaim_bytes,
            } => {
                self.phase = "Executed cleanup target".to_string();
                self.current_rule_id = Some(rule_id.clone());
                self.current_path = Some(path.clone());
                self.targets_finished = self.targets_finished.saturating_add(1);
                self.logical_bytes = self
                    .logical_bytes
                    .saturating_add(freed_bytes)
                    .saturating_add(pending_reclaim_bytes);
                self.last_event = format!("{rule_id} {target_status}: {}", path.display());
            }
            TuiTaskProgressEvent::ExecutionFinished {
                completed_targets,
                freed_bytes,
                pending_reclaim_bytes,
            } => {
                self.phase = "Cleanup execution finished".to_string();
                self.targets_finished = completed_targets;
                self.logical_bytes = freed_bytes.saturating_add(pending_reclaim_bytes);
                self.last_event = format!("{completed_targets} target(s) completed");
            }
        }
    }

    pub(crate) fn mark_cancel_requested(&mut self) {
        self.cancel_requested = true;
        self.phase = "Cancel requested".to_string();
    }

    pub(crate) fn cancel_wait_message(&self) -> &'static str {
        if self.label.contains("trash") || self.targets_started > 0 {
            "Cancel requested; cleanup will stop before the next target or after the current trash operation returns."
        } else {
            "Cancel requested; waiting for cooperative checkpoint."
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiTaskId(pub(crate) u64);

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn traversal_updates_structured_status() {
        let mut status = TuiTaskStatus::started("Scanning fixture...");

        status.apply_event(TuiTaskProgressEvent::Traversal {
            root: PathBuf::from("/tmp"),
            counter: "files".to_string(),
            value: 8,
            logical_bytes: 42,
            files: 8,
            directories: 2,
        });

        assert_eq!(status.phase, "Walking files 8");
        assert_eq!(status.current_root, Some(PathBuf::from("/tmp")));
        assert_eq!(status.files, 8);
        assert_eq!(status.directories, 2);
        assert_eq!(status.logical_bytes, 42);
        assert_eq!(status.last_event, "files: 8");
    }

    #[test]
    fn cleanup_cache_events_accumulate_counts_and_bytes() {
        let mut status = TuiTaskStatus::started("Preview cleanup...");

        status.apply_event(TuiTaskProgressEvent::CleanupCacheHit {
            rule_id: "linux.user-cache".to_string(),
            path: PathBuf::from("/home/me/.cache/app"),
            estimated_bytes: 1024,
        });
        status.apply_event(TuiTaskProgressEvent::CleanupCacheMiss {
            rule_id: "linux.user-cache".to_string(),
            path: PathBuf::from("/home/me/.cache/other"),
            reason: "stale".to_string(),
            pruned: true,
        });
        status.apply_event(TuiTaskProgressEvent::CleanupCacheWriteSkipped {
            rule_id: "linux.user-cache".to_string(),
            path: PathBuf::from("/home/me/.cache/readonly"),
        });

        assert_eq!(status.cache_hits, 1);
        assert_eq!(status.cache_misses, 1);
        assert_eq!(status.cache_write_skipped, 1);
        assert_eq!(status.cache_pruned, 1);
        assert_eq!(status.estimated_bytes, 1024);
        assert_eq!(status.current_rule_id.as_deref(), Some("linux.user-cache"));
        assert_eq!(
            status.current_path,
            Some(PathBuf::from("/home/me/.cache/readonly"))
        );
    }

    #[test]
    fn execution_event_tracks_completed_reclaim_bytes() {
        let mut status = TuiTaskStatus::started("Executing cleanup...");

        status.apply_event(TuiTaskProgressEvent::ExecutionFinished {
            completed_targets: 2,
            freed_bytes: 128,
            pending_reclaim_bytes: 384,
        });

        assert_eq!(status.phase, "Cleanup execution finished");
        assert_eq!(status.targets_finished, 2);
        assert_eq!(status.logical_bytes, 512);
        assert_eq!(status.last_event, "2 target(s) completed");
    }

    #[test]
    fn execution_target_events_update_structured_status() {
        let mut status = TuiTaskStatus::started("Executing cleanup...");

        status.apply_event(TuiTaskProgressEvent::ExecutionStarted {
            executable_targets: 2,
            estimated_bytes: 512,
            mode: "recoverable-delete".to_string(),
        });
        status.apply_event(TuiTaskProgressEvent::ExecutionTargetStarted {
            rule_id: "linux.user-cache".to_string(),
            path: PathBuf::from("/tmp/cache"),
            estimated_bytes: 512,
        });
        status.apply_event(TuiTaskProgressEvent::ExecutionTargetFinished {
            rule_id: "linux.user-cache".to_string(),
            path: PathBuf::from("/tmp/cache"),
            status: "completed".to_string(),
            freed_bytes: 0,
            pending_reclaim_bytes: 512,
        });

        assert_eq!(status.phase, "Executed cleanup target");
        assert_eq!(status.targets_started, 1);
        assert_eq!(status.targets_finished, 1);
        assert_eq!(status.estimated_bytes, 512);
        assert_eq!(status.logical_bytes, 512);
        assert_eq!(status.current_rule_id.as_deref(), Some("linux.user-cache"));
        assert_eq!(
            status.current_path.as_deref(),
            Some(Path::new("/tmp/cache"))
        );
        assert_eq!(status.last_event, "linux.user-cache completed: /tmp/cache");
    }

    #[test]
    fn backend_fallback_and_cancel_request_update_status() {
        let mut status = TuiTaskStatus::started("Scanning fixture...");

        status.apply_event(TuiTaskProgressEvent::BackendFallback {
            root: PathBuf::from("/tmp"),
            backend: "portable".to_string(),
            reason: "MFT unavailable".to_string(),
        });
        status.mark_cancel_requested();

        assert_eq!(status.backend.as_deref(), Some("portable"));
        assert_eq!(status.current_path.as_deref(), Some(Path::new("/tmp")));
        assert_eq!(status.last_event, "portable: MFT unavailable");
        assert!(status.cancel_requested);
        assert_eq!(status.phase, "Cancel requested");
        assert_eq!(
            status.cancel_wait_message(),
            "Cancel requested; waiting for cooperative checkpoint."
        );
    }

    #[test]
    fn execution_cancel_request_reports_target_boundary() {
        let mut status =
            TuiTaskStatus::started("Moving allowed targets to the system trash or Recycle Bin...");

        status.mark_cancel_requested();

        assert_eq!(
            status.cancel_wait_message(),
            "Cancel requested; cleanup will stop before the next target or after the current trash operation returns."
        );
    }

    #[test]
    fn only_high_volume_progress_events_are_coalescible() {
        assert!(
            TuiTaskProgressEvent::Traversal {
                root: PathBuf::from("/tmp"),
                counter: "files".to_string(),
                value: 10,
                logical_bytes: 128,
                files: 10,
                directories: 2,
            }
            .is_coalescible()
        );
        assert!(
            TuiTaskProgressEvent::FileMeasured {
                target_path: PathBuf::from("/tmp"),
                path: PathBuf::from("/tmp/a.bin"),
                file_size: 128,
                files_scanned: 1,
                bytes_scanned: 128,
            }
            .is_coalescible()
        );
        assert!(
            !TuiTaskProgressEvent::BackendFallback {
                root: PathBuf::from("/tmp"),
                backend: "portable".to_string(),
                reason: "fallback".to_string(),
            }
            .is_coalescible()
        );
        assert!(
            !TuiTaskProgressEvent::ExecutionFinished {
                completed_targets: 1,
                freed_bytes: 128,
                pending_reclaim_bytes: 0,
            }
            .is_coalescible()
        );
    }
}
