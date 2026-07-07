use std::collections::BTreeMap;
use std::path::PathBuf;

use rebecca::core::cleanup_advice::{CleanupAdvice, CleanupAdviceStatus};
use rebecca::core::disk_map::{DiskMapEntryKind, DiskMapGroupKind, DiskMapSortField};
use rebecca::core::disk_session::{
    DiskMapDistributionFilter, DiskMapDistributionRow, DiskMapNodeId, DiskMapSession,
    DiskMapSessionFilter, DiskMapVisibleRow,
};
use rebecca::core::history::HistoryEntry;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::scan::ScanBackendKind;

use crate::output::format_bytes;
use crate::tui::basket::CleanupBasketItem;
use crate::tui::effect::TuiEffect;
use crate::tui::input::{TuiKey, TuiMouseAction};
use crate::tui::model::TuiScreen;
use crate::tui::navigation::RootChoice;
use crate::tui::progress::{TuiTaskId, TuiTaskProgressEvent, TuiTaskStatus};
use crate::workbench::CleanupWorkbenchRequest;

#[derive(Debug, Clone)]
pub(crate) struct TuiApp {
    pub(crate) screen: TuiScreen,
    previous_screen: TuiScreen,
    task_return_screen: TuiScreen,
    pub(crate) root_choices: Vec<RootChoice>,
    pub(crate) session: Option<DiskMapSession>,
    pub(crate) current_parent: Option<DiskMapNodeId>,
    pub(crate) selected: usize,
    pub(crate) selected_type: usize,
    pub(crate) selected_extension: usize,
    pub(crate) sort: DiskMapSortField,
    pub(crate) search_query: String,
    pub(crate) type_search_query: String,
    pub(crate) extension_search_query: String,
    search_editing: bool,
    pub(crate) basket: BTreeMap<String, CleanupBasketItem>,
    pub(crate) preview: Option<CleanupPlan>,
    pub(crate) executed: Option<CleanupPlan>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) message: String,
    pub(crate) task_status: Option<TuiTaskStatus>,
    pub(crate) scan_backend: ScanBackendKind,
    pub(crate) entry_limit: usize,
    session_stack: Vec<TuiSessionSnapshot>,
    pending_refresh_snapshot: Option<TuiSessionSnapshot>,
    failed_effect: Option<TuiEffect>,
    error_return_screen: TuiScreen,
    next_task_id: u64,
    should_quit: bool,
}

#[derive(Debug, Clone)]
struct TuiSessionSnapshot {
    session: DiskMapSession,
    current_parent: Option<DiskMapNodeId>,
    screen: TuiScreen,
    selected: usize,
    selected_type: usize,
    selected_extension: usize,
    search_query: String,
    type_search_query: String,
    extension_search_query: String,
    message: String,
}

impl TuiApp {
    pub(crate) fn root_picker(
        root_choices: Vec<RootChoice>,
        scan_backend: ScanBackendKind,
        entry_limit: usize,
    ) -> Self {
        Self {
            screen: TuiScreen::RootPicker,
            previous_screen: TuiScreen::RootPicker,
            task_return_screen: TuiScreen::RootPicker,
            root_choices,
            session: None,
            current_parent: None,
            selected: 0,
            selected_type: 0,
            selected_extension: 0,
            sort: DiskMapSortField::Logical,
            search_query: String::new(),
            type_search_query: String::new(),
            extension_search_query: String::new(),
            search_editing: false,
            basket: BTreeMap::new(),
            preview: None,
            executed: None,
            history: Vec::new(),
            message: "Choose a root and press Enter to scan.".to_string(),
            task_status: None,
            scan_backend,
            entry_limit,
            session_stack: Vec::new(),
            pending_refresh_snapshot: None,
            failed_effect: None,
            error_return_screen: TuiScreen::RootPicker,
            next_task_id: 0,
            should_quit: false,
        }
    }

    pub(crate) fn from_session(
        session: DiskMapSession,
        scan_backend: ScanBackendKind,
        entry_limit: usize,
    ) -> Self {
        let current_parent = session.root_ids().first().copied();
        Self {
            screen: TuiScreen::Map,
            previous_screen: TuiScreen::Map,
            task_return_screen: TuiScreen::Map,
            root_choices: Vec::new(),
            session: Some(session),
            current_parent,
            selected: 0,
            selected_type: 0,
            selected_extension: 0,
            sort: DiskMapSortField::Logical,
            search_query: String::new(),
            type_search_query: String::new(),
            extension_search_query: String::new(),
            search_editing: false,
            basket: BTreeMap::new(),
            preview: None,
            executed: None,
            history: Vec::new(),
            message: "Space stages a cleanup rule, c previews all matching targets.".to_string(),
            task_status: None,
            scan_backend,
            entry_limit,
            session_stack: Vec::new(),
            pending_refresh_snapshot: None,
            failed_effect: None,
            error_return_screen: TuiScreen::Map,
            next_task_id: 0,
            should_quit: false,
        }
    }

    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub(crate) fn is_search_editing(&self) -> bool {
        self.search_editing
    }

    pub(crate) fn allocate_task_id(&mut self) -> TuiTaskId {
        let id = TuiTaskId(self.next_task_id);
        self.next_task_id = self.next_task_id.saturating_add(1);
        id
    }

    pub(crate) fn selected_root(&self) -> Option<PathBuf> {
        self.root_choices
            .get(self.selected)
            .map(|choice| choice.path.clone())
    }

    pub(crate) fn current_node_name(&self) -> String {
        self.session
            .as_ref()
            .and_then(|session| self.current_parent.and_then(|id| session.node(id)))
            .map(|node| node.display_name())
            .unwrap_or_else(|| "Roots".to_string())
    }

    pub(crate) fn visible_rows(&self) -> Vec<DiskMapVisibleRow> {
        self.session
            .as_ref()
            .map(|session| {
                session.visible_rows(
                    self.current_parent,
                    self.sort,
                    DiskMapSessionFilter {
                        path_contains: (!self.search_query.is_empty())
                            .then_some(self.search_query.as_str()),
                    },
                )
            })
            .unwrap_or_default()
    }

    pub(crate) fn selected_row(&self) -> Option<DiskMapVisibleRow> {
        self.visible_rows().get(self.selected).cloned()
    }

    pub(crate) fn distribution_rows(&self) -> Vec<DiskMapDistributionRow> {
        let Some(kind) = self.active_distribution_kind() else {
            return Vec::new();
        };
        self.distribution_rows_for(kind)
    }

    pub(crate) fn distribution_rows_for(
        &self,
        kind: DiskMapGroupKind,
    ) -> Vec<DiskMapDistributionRow> {
        let query = match kind {
            DiskMapGroupKind::Type => self.type_search_query.as_str(),
            DiskMapGroupKind::Extension => self.extension_search_query.as_str(),
            DiskMapGroupKind::Depth | DiskMapGroupKind::Age => "",
        };
        self.session
            .as_ref()
            .map(|session| {
                session.distribution_rows(
                    kind,
                    self.sort,
                    DiskMapDistributionFilter {
                        label_contains: (!query.is_empty()).then_some(query),
                    },
                )
            })
            .unwrap_or_default()
    }

    pub(crate) fn selected_distribution_index(&self) -> usize {
        match self.screen {
            TuiScreen::Types => self.selected_type,
            TuiScreen::Extensions => self.selected_extension,
            _ => 0,
        }
    }

    pub(crate) fn active_filter_text(&self) -> &str {
        match self.screen {
            TuiScreen::Types => &self.type_search_query,
            TuiScreen::Extensions => &self.extension_search_query,
            _ => &self.search_query,
        }
    }

    pub(crate) fn active_distribution_kind(&self) -> Option<DiskMapGroupKind> {
        match self.screen {
            TuiScreen::Types => Some(DiskMapGroupKind::Type),
            TuiScreen::Extensions => Some(DiskMapGroupKind::Extension),
            _ => None,
        }
    }

    pub(crate) fn handle_key(&mut self, key: TuiKey) -> TuiEffect {
        if self.search_editing {
            return self.handle_search_key(key);
        }

        match self.screen {
            TuiScreen::RootPicker => self.handle_root_picker_key(key),
            TuiScreen::Map | TuiScreen::Treemap => self.handle_map_key(key),
            TuiScreen::Types | TuiScreen::Extensions => self.handle_distribution_key(key),
            TuiScreen::Busy => self.handle_busy_key(key),
            TuiScreen::Preview => self.handle_preview_key(key),
            TuiScreen::Confirm => self.handle_confirm_key(key),
            TuiScreen::History => self.handle_history_key(key),
            TuiScreen::Executed => self.handle_terminal_screen_key(key),
            TuiScreen::Error => self.handle_error_key(key),
            TuiScreen::Help => self.handle_help_key(key),
        }
    }

    pub(crate) fn handle_mouse_action(&mut self, action: TuiMouseAction) -> TuiEffect {
        match action {
            TuiMouseAction::SwitchScreen(screen) => self.open_screen(screen),
            TuiMouseAction::SelectMapRow(index) => {
                self.select_map_row(index);
                TuiEffect::None
            }
            TuiMouseAction::SelectDistributionRow(index) => {
                self.select_distribution_row(index);
                TuiEffect::None
            }
            TuiMouseAction::ScrollUp => {
                self.move_active_selection(-1);
                TuiEffect::None
            }
            TuiMouseAction::ScrollDown => {
                self.move_active_selection(1);
                TuiEffect::None
            }
        }
    }

    pub(crate) fn apply_scan_result(&mut self, session: DiskMapSession) {
        self.session = Some(session);
        self.session_stack.clear();
        self.pending_refresh_snapshot = None;
        self.current_parent = self
            .session
            .as_ref()
            .and_then(|session| session.root_ids().first().copied());
        self.screen = TuiScreen::Map;
        self.selected = 0;
        self.selected_type = 0;
        self.selected_extension = 0;
        self.search_query.clear();
        self.type_search_query.clear();
        self.extension_search_query.clear();
        self.message =
            "Scan complete. Space stages cleanup rules, c previews all matching targets."
                .to_string();
        self.task_status = None;
        self.failed_effect = None;
    }

    pub(crate) fn apply_refresh_result(&mut self, session: DiskMapSession) {
        if let Some(snapshot) = self.pending_refresh_snapshot.take() {
            self.session_stack.push(snapshot);
        } else {
            self.push_session_snapshot();
        }
        self.session = Some(session);
        self.current_parent = self
            .session
            .as_ref()
            .and_then(|session| session.root_ids().first().copied());
        self.screen = match self.task_return_screen {
            TuiScreen::Map | TuiScreen::Treemap | TuiScreen::Types | TuiScreen::Extensions => {
                self.task_return_screen
            }
            _ => TuiScreen::Map,
        };
        self.selected = 0;
        self.selected_type = 0;
        self.selected_extension = 0;
        self.message = "Refresh complete. Press b to restore the previous scan.".to_string();
        self.task_status = None;
        self.failed_effect = None;
    }

    pub(crate) fn apply_task_started(&mut self, label: impl Into<String>) {
        self.task_return_screen = self.screen;
        self.screen = TuiScreen::Busy;
        let label = label.into();
        self.task_status = Some(TuiTaskStatus::started(label.clone()));
        self.message = label;
    }

    pub(crate) fn apply_task_progress(&mut self, event: TuiTaskProgressEvent) {
        let status = self
            .task_status
            .get_or_insert_with(|| TuiTaskStatus::started("Working..."));
        match event {
            TuiTaskProgressEvent::RootStarted {
                root_index,
                root_count,
                root,
                backend,
            } => {
                status.phase = format!("Scanning root {}/{}", root_index + 1, root_count);
                status.backend = Some(backend);
                status.current_root = Some(root.clone());
                status.current_path = Some(root.clone());
                status.root_count = root_count;
                status.last_event = format!("Started {}", root.display());
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
                status.phase = format!("Finished root {}/{}", root_index + 1, root_count);
                status.current_root = Some(root.clone());
                status.current_path = Some(root.clone());
                status.roots_finished = status.roots_finished.max(root_index + 1);
                status.root_count = root_count;
                status.logical_bytes = logical_bytes;
                status.files = files;
                status.directories = directories;
                status.last_event = format!("{root_status}: {}", root.display());
            }
            TuiTaskProgressEvent::Traversal {
                root,
                counter,
                value,
                logical_bytes,
                files,
                directories,
            } => {
                status.phase = format!("Walking {counter} {value}");
                status.current_root = Some(root);
                status.logical_bytes = logical_bytes;
                status.files = files;
                status.directories = directories;
                status.last_event = format!("{counter}: {value}");
            }
            TuiTaskProgressEvent::FileMeasured {
                target_path,
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            } => {
                status.phase = "Measuring files".to_string();
                status.current_root = Some(target_path.clone());
                status.current_path = Some(path.clone());
                status.files = files_scanned;
                status.logical_bytes = bytes_scanned;
                status.last_event = format!("{} ({file_size} bytes)", path.display());
            }
            TuiTaskProgressEvent::BackendFallback {
                root,
                backend,
                reason,
            } => {
                status.phase = "Backend fallback".to_string();
                status.backend = Some(backend.clone());
                status.current_root = Some(root.clone());
                status.current_path = Some(root.clone());
                status.last_event = format!("{backend}: {reason}");
            }
            TuiTaskProgressEvent::BackendStage {
                root,
                backend,
                stage,
                finished,
            } => {
                status.phase = if finished {
                    format!("{stage} finished")
                } else {
                    format!("{stage} started")
                };
                status.backend = Some(backend);
                status.current_root = Some(root.clone());
                status.current_path = Some(root);
                status.last_event = status.phase.clone();
            }
            TuiTaskProgressEvent::BackendMetric { metric, value } => {
                status.phase = "Reading backend metrics".to_string();
                status.last_event = format!("{metric}: {value}");
            }
            TuiTaskProgressEvent::Finalizing {
                roots,
                logical_bytes,
                files,
                directories,
            } => {
                status.phase = "Finalizing map".to_string();
                status.root_count = roots;
                status.roots_finished = roots;
                status.logical_bytes = logical_bytes;
                status.files = files;
                status.directories = directories;
                status.last_event = "Building ranked tree".to_string();
            }
            TuiTaskProgressEvent::CleanupTargetScanning { rule_id, path } => {
                status.phase = "Scanning cleanup target".to_string();
                status.current_rule_id = Some(rule_id.clone());
                status.current_path = Some(path.clone());
                status.targets_started = status.targets_started.saturating_add(1);
                status.last_event = format!("{rule_id}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupTargetFinished {
                rule_id,
                path,
                status: target_status,
                estimated_bytes,
            } => {
                status.phase = "Measured cleanup target".to_string();
                status.current_rule_id = Some(rule_id.clone());
                status.current_path = Some(path.clone());
                status.targets_finished = status.targets_finished.saturating_add(1);
                status.estimated_bytes = status.estimated_bytes.saturating_add(estimated_bytes);
                status.last_event = format!("{rule_id} {target_status}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupFileMeasured {
                rule_id,
                target_path,
                path,
                file_size,
                files_scanned,
                bytes_scanned,
            } => {
                status.phase = "Measuring cleanup files".to_string();
                status.current_rule_id = Some(rule_id);
                status.current_root = Some(target_path);
                status.current_path = Some(path.clone());
                status.files = files_scanned;
                status.logical_bytes = bytes_scanned;
                status.last_event = format!("{} ({file_size} bytes)", path.display());
            }
            TuiTaskProgressEvent::CleanupCacheHit {
                rule_id,
                path,
                estimated_bytes,
            } => {
                status.phase = "Using scan cache".to_string();
                status.current_rule_id = Some(rule_id.clone());
                status.current_path = Some(path.clone());
                status.cache_hits = status.cache_hits.saturating_add(1);
                status.estimated_bytes = status.estimated_bytes.saturating_add(estimated_bytes);
                status.last_event = format!("{rule_id} cache hit: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupCacheMiss {
                rule_id,
                path,
                reason,
                pruned,
            } => {
                status.phase = "Refreshing scan cache".to_string();
                status.current_rule_id = Some(rule_id.clone());
                status.current_path = Some(path.clone());
                status.cache_misses = status.cache_misses.saturating_add(1);
                status.cache_pruned += usize::from(pruned);
                status.last_event = format!("{rule_id} cache miss {reason}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupCacheWriteSkipped { rule_id, path } => {
                status.phase = "Scan cache write skipped".to_string();
                status.current_rule_id = Some(rule_id.clone());
                status.current_path = Some(path.clone());
                status.cache_write_skipped = status.cache_write_skipped.saturating_add(1);
                status.last_event = format!("{rule_id}: {}", path.display());
            }
            TuiTaskProgressEvent::CleanupCachePruned {
                inspected,
                pruned,
                retained,
            } => {
                status.phase = "Pruning scan cache".to_string();
                status.cache_pruned = status.cache_pruned.saturating_add(pruned);
                status.last_event =
                    format!("cache inspected {inspected}, pruned {pruned}, retained {retained}");
            }
            TuiTaskProgressEvent::ExecutionFinished {
                completed_targets,
                freed_bytes,
                pending_reclaim_bytes,
            } => {
                status.phase = "Cleanup execution finished".to_string();
                status.targets_finished = completed_targets;
                status.logical_bytes = freed_bytes.saturating_add(pending_reclaim_bytes);
                status.last_event = format!("{completed_targets} target(s) completed");
            }
        }
    }

    pub(crate) fn apply_cancel_requested(&mut self) {
        if let Some(status) = &mut self.task_status {
            status.cancel_requested = true;
            status.phase = "Cancel requested".to_string();
        }
        self.message = "Cancel requested; waiting for the worker to stop.".to_string();
    }

    pub(crate) fn apply_task_cancelled(&mut self) {
        self.screen = self.task_return_screen;
        self.task_status = None;
        self.pending_refresh_snapshot = None;
        self.message = "Task cancelled.".to_string();
    }

    pub(crate) fn apply_task_already_running(&mut self) {
        self.message = "A background task is already running.".to_string();
    }

    pub(crate) fn prepare_refresh(&mut self) {
        self.pending_refresh_snapshot = self.current_session_snapshot();
    }

    pub(crate) fn apply_preview(&mut self, plan: CleanupPlan) {
        let allowed = plan.summary.allowed_targets;
        let bytes = plan.summary.estimated_bytes;
        self.preview = Some(plan);
        self.screen = TuiScreen::Preview;
        self.selected = 0;
        self.message = format!(
            "Preview ready: {allowed} allowed target(s), {}.",
            format_bytes(bytes)
        );
        self.task_status = None;
        self.failed_effect = None;
    }

    pub(crate) fn apply_execution(&mut self, plan: CleanupPlan) {
        self.executed = Some(plan);
        self.screen = TuiScreen::Executed;
        self.basket.clear();
        self.preview = None;
        self.message = "Cleanup finished and history was updated.".to_string();
        self.task_status = None;
        self.failed_effect = None;
    }

    pub(crate) fn set_history(&mut self, entries: Vec<HistoryEntry>) {
        self.history = entries;
    }

    pub(crate) fn apply_error(&mut self, message: impl Into<String>) {
        self.screen = TuiScreen::Error;
        self.message = message.into();
        self.task_status = None;
        self.pending_refresh_snapshot = None;
        self.failed_effect = None;
    }

    pub(crate) fn apply_task_error(&mut self, message: impl Into<String>, retry: TuiEffect) {
        self.screen = TuiScreen::Error;
        self.message = message.into();
        self.task_status = None;
        self.pending_refresh_snapshot = None;
        self.failed_effect = Some(retry);
        self.error_return_screen = self.task_return_screen;
    }

    pub(crate) fn workbench_request(&self) -> CleanupWorkbenchRequest {
        CleanupWorkbenchRequest {
            selected_rule_ids: self.basket.keys().cloned().collect(),
            allow_moderate: false,
            allow_risky: false,
            allowed_warnings: Vec::new(),
            scan_cache: true,
            scan_backend: self.scan_backend,
            exclude_paths: Vec::new(),
        }
    }

    pub(crate) fn confirmation_phrase(&self) -> String {
        let bytes = self
            .preview
            .as_ref()
            .map(|plan| plan.summary.estimated_bytes)
            .unwrap_or(0);
        format!("CLEAN {bytes}")
    }

    fn handle_root_picker_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Down | TuiKey::Char('j') => {
                self.move_selection(self.root_choices.len(), 1);
                TuiEffect::None
            }
            TuiKey::Up | TuiKey::Char('k') => {
                self.move_selection(self.root_choices.len(), -1);
                TuiEffect::None
            }
            TuiKey::Enter => self
                .selected_root()
                .map(|root| TuiEffect::Scan(vec![root]))
                .unwrap_or(TuiEffect::None),
            TuiKey::Char('q') | TuiKey::Esc => self.quit(),
            TuiKey::Char('?') => self.open_help(),
            _ => TuiEffect::None,
        }
    }

    fn handle_map_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Down | TuiKey::Char('j') => {
                self.move_selection(self.visible_rows().len(), 1);
                TuiEffect::None
            }
            TuiKey::Up | TuiKey::Char('k') => {
                self.move_selection(self.visible_rows().len(), -1);
                TuiEffect::None
            }
            TuiKey::Right | TuiKey::Enter | TuiKey::Char('l') => {
                self.open_selected_node();
                TuiEffect::None
            }
            TuiKey::Left | TuiKey::Char('h') | TuiKey::Esc => {
                self.open_parent_node();
                TuiEffect::None
            }
            TuiKey::Char('/') => {
                self.search_editing = true;
                self.message = "Type search text, Enter to apply, Esc to cancel.".to_string();
                TuiEffect::None
            }
            TuiKey::Char('s') => {
                self.cycle_sort();
                TuiEffect::None
            }
            TuiKey::Char('1') => self.open_map_view(),
            TuiKey::Char('4') | TuiKey::Char('w') => self.open_treemap_view(),
            TuiKey::Char('2') | TuiKey::Char('t') => self.open_types_view(),
            TuiKey::Char('3') | TuiKey::Char('x') => self.open_extensions_view(),
            TuiKey::Tab => self.cycle_view_mode(),
            TuiKey::Char('r') => self.refresh_selected_directory(),
            TuiKey::Char('R') => self.refresh_current_root(),
            TuiKey::Char('b') => {
                self.restore_previous_session();
                TuiEffect::None
            }
            TuiKey::Space => {
                self.toggle_selected_rule();
                TuiEffect::None
            }
            TuiKey::Char('c') => {
                if self.basket.is_empty() {
                    self.message = "Stage at least one cleanup rule before preview.".to_string();
                    TuiEffect::None
                } else {
                    TuiEffect::Preview(self.workbench_request())
                }
            }
            TuiKey::Char('?') => self.open_help(),
            TuiKey::Char('g') => self.open_history(),
            TuiKey::Char('q') => self.quit(),
            _ => TuiEffect::None,
        }
    }

    fn handle_distribution_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Up | TuiKey::Char('k') => {
                self.move_distribution_selection(-1);
                TuiEffect::None
            }
            TuiKey::Down | TuiKey::Char('j') => {
                self.move_distribution_selection(1);
                TuiEffect::None
            }
            TuiKey::Char('/') => {
                self.search_editing = true;
                self.message = format!(
                    "Type {} filter, Enter to apply, Esc to clear.",
                    self.active_filter_singular_label()
                );
                TuiEffect::None
            }
            TuiKey::Char('s') => {
                self.cycle_sort();
                TuiEffect::None
            }
            TuiKey::Char('1') | TuiKey::Esc | TuiKey::Char('h') => self.open_map_view(),
            TuiKey::Char('4') | TuiKey::Char('w') => self.open_treemap_view(),
            TuiKey::Char('2') | TuiKey::Char('t') => self.open_types_view(),
            TuiKey::Char('3') | TuiKey::Char('x') => self.open_extensions_view(),
            TuiKey::Tab => self.cycle_view_mode(),
            TuiKey::Char('r') => self.refresh_current_directory(),
            TuiKey::Char('R') => self.refresh_current_root(),
            TuiKey::Char('b') => {
                self.restore_previous_session();
                TuiEffect::None
            }
            TuiKey::Char('?') => self.open_help(),
            TuiKey::Char('g') => self.open_history(),
            TuiKey::Char('q') => self.quit(),
            _ => TuiEffect::None,
        }
    }

    fn handle_preview_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Char('e') => {
                if self
                    .preview
                    .as_ref()
                    .is_some_and(|plan| plan.summary.allowed_targets > 0)
                {
                    self.screen = TuiScreen::Confirm;
                    self.message = format!(
                        "Type {} and press Enter to move targets to recoverable trash.",
                        self.confirmation_phrase()
                    );
                } else {
                    self.message = "Preview has no allowed targets to execute.".to_string();
                }
                TuiEffect::None
            }
            TuiKey::Esc | TuiKey::Char('h') | TuiKey::Char('c') => {
                self.screen = TuiScreen::Map;
                self.message = "Preview closed.".to_string();
                TuiEffect::None
            }
            TuiKey::Char('q') => self.quit(),
            TuiKey::Char('?') => self.open_help(),
            TuiKey::Char('g') => self.open_history(),
            _ => TuiEffect::None,
        }
    }

    fn handle_busy_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Char('q') => self.quit(),
            TuiKey::Esc => TuiEffect::CancelTask,
            TuiKey::Char('?') => self.open_help(),
            _ => {
                self.message = "A background task is still running.".to_string();
                TuiEffect::None
            }
        }
    }

    fn handle_confirm_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Enter => {
                let expected = self.confirmation_phrase();
                if self.message == expected {
                    return TuiEffect::Execute(self.workbench_request());
                }
                self.message = format!("Confirmation must exactly match: {expected}");
                TuiEffect::None
            }
            TuiKey::Backspace => {
                self.message.pop();
                TuiEffect::None
            }
            TuiKey::Space => {
                if self.message.starts_with("Type ") {
                    self.message.clear();
                }
                self.message.push(' ');
                TuiEffect::None
            }
            TuiKey::Esc => {
                self.screen = TuiScreen::Preview;
                self.message = "Execution cancelled.".to_string();
                TuiEffect::None
            }
            TuiKey::Char(ch) => {
                if self.message.starts_with("Type ") {
                    self.message.clear();
                }
                self.message.push(ch);
                TuiEffect::None
            }
            _ => TuiEffect::None,
        }
    }

    fn handle_history_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Esc | TuiKey::Char('h') | TuiKey::Char('g') => {
                self.screen = TuiScreen::Map;
                self.message = "History closed.".to_string();
                TuiEffect::None
            }
            TuiKey::Char('?') => self.open_help(),
            TuiKey::Char('q') => self.quit(),
            _ => TuiEffect::None,
        }
    }

    fn handle_terminal_screen_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Char('g') => self.open_history(),
            TuiKey::Char('q') | TuiKey::Esc | TuiKey::Enter => self.quit(),
            TuiKey::Char('?') => self.open_help(),
            _ => TuiEffect::None,
        }
    }

    fn handle_error_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Char('r') => {
                if let Some(effect) = self.failed_effect.clone() {
                    self.message = "Retrying failed task.".to_string();
                    return effect;
                }
                self.message = "No retry is available for this error.".to_string();
                TuiEffect::None
            }
            TuiKey::Esc | TuiKey::Char('h') | TuiKey::Char('b') => {
                self.screen = self.error_return_screen;
                self.message = "Returned from error.".to_string();
                TuiEffect::None
            }
            TuiKey::Char('?') => self.open_help(),
            TuiKey::Char('q') => self.quit(),
            _ => TuiEffect::None,
        }
    }

    fn handle_help_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Esc | TuiKey::Char('?') | TuiKey::Char('q') => {
                self.screen = self.previous_screen;
                TuiEffect::None
            }
            _ => TuiEffect::None,
        }
    }

    fn handle_search_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Enter => {
                self.search_editing = false;
                self.reset_active_selection();
                let query = self.active_filter_text().to_string();
                let label = self.active_filter_label();
                self.message = if query.is_empty() {
                    "Search cleared.".to_string()
                } else {
                    format!("Filtering {label} containing '{query}'.")
                };
            }
            TuiKey::Esc => {
                self.search_editing = false;
                self.active_search_query_mut().clear();
                self.reset_active_selection();
                self.message = "Search cleared.".to_string();
            }
            TuiKey::Backspace => {
                self.active_search_query_mut().pop();
            }
            TuiKey::Space => {
                self.active_search_query_mut().push(' ');
            }
            TuiKey::Char(ch) => {
                self.active_search_query_mut().push(ch);
            }
            _ => {}
        }
        TuiEffect::None
    }

    fn open_selected_node(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        if row.has_children {
            self.current_parent = Some(row.id);
            self.selected = 0;
            self.message = format!("Opened {}.", row.name);
        }
    }

    fn open_parent_node(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let Some(current_parent) = self.current_parent else {
            return;
        };
        let parent = session.node(current_parent).and_then(|node| node.parent);
        if parent.is_some() {
            self.current_parent = parent;
            self.selected = 0;
            self.message = "Moved up one level.".to_string();
        }
    }

    fn refresh_selected_directory(&mut self) -> TuiEffect {
        let Some(row) = self.selected_row() else {
            return self.refresh_current_directory();
        };
        if row.kind == DiskMapEntryKind::Directory || row.has_children {
            return TuiEffect::Refresh(vec![row.path]);
        }
        self.message = "Selected file cannot be refreshed as a subtree.".to_string();
        TuiEffect::None
    }

    fn refresh_current_directory(&mut self) -> TuiEffect {
        let Some(session) = self.session.as_ref() else {
            self.message = "Scan a root before refreshing.".to_string();
            return TuiEffect::None;
        };
        let Some(path) = self
            .current_parent
            .and_then(|id| session.node(id))
            .map(|node| node.path.clone())
        else {
            self.message = "No current directory to refresh.".to_string();
            return TuiEffect::None;
        };
        TuiEffect::Refresh(vec![path])
    }

    fn refresh_current_root(&mut self) -> TuiEffect {
        let Some(session) = self.session.as_ref() else {
            self.message = "Scan a root before refreshing.".to_string();
            return TuiEffect::None;
        };
        let Some(path) = self
            .current_parent
            .and_then(|id| session.node(id))
            .map(|node| node.root.clone())
            .or_else(|| {
                session
                    .root_ids()
                    .first()
                    .and_then(|id| session.node(*id))
                    .map(|node| node.path.clone())
            })
        else {
            self.message = "No scan root to refresh.".to_string();
            return TuiEffect::None;
        };
        TuiEffect::Refresh(vec![path])
    }

    fn push_session_snapshot(&mut self) {
        if let Some(snapshot) = self.current_session_snapshot() {
            self.session_stack.push(snapshot);
        }
    }

    fn current_session_snapshot(&self) -> Option<TuiSessionSnapshot> {
        self.session
            .as_ref()
            .cloned()
            .map(|session| TuiSessionSnapshot {
                session,
                current_parent: self.current_parent,
                screen: self.screen,
                selected: self.selected,
                selected_type: self.selected_type,
                selected_extension: self.selected_extension,
                search_query: self.search_query.clone(),
                type_search_query: self.type_search_query.clone(),
                extension_search_query: self.extension_search_query.clone(),
                message: self.message.clone(),
            })
    }

    fn restore_previous_session(&mut self) {
        let Some(snapshot) = self.session_stack.pop() else {
            self.message = "No previous scan to restore.".to_string();
            return;
        };
        self.session = Some(snapshot.session);
        self.current_parent = snapshot.current_parent;
        self.selected = snapshot.selected;
        self.selected_type = snapshot.selected_type;
        self.selected_extension = snapshot.selected_extension;
        self.search_query = snapshot.search_query;
        self.type_search_query = snapshot.type_search_query;
        self.extension_search_query = snapshot.extension_search_query;
        self.screen = match snapshot.screen {
            TuiScreen::Map | TuiScreen::Treemap | TuiScreen::Types | TuiScreen::Extensions => {
                snapshot.screen
            }
            _ => TuiScreen::Map,
        };
        self.message = format!("Restored previous scan. {}", snapshot.message);
    }

    fn toggle_selected_rule(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let Some(advice) = row.cleanup_advice.as_ref() else {
            self.message = "Selected entry has no cleanup advice to stage.".to_string();
            return;
        };
        if !stageable_advice(advice) {
            self.message = format!("{} entries cannot be staged.", advice.status.label());
            return;
        }
        let Some(rule_id) = advice.rule_id.as_ref() else {
            self.message = "This advice is not backed by a cleanup rule yet.".to_string();
            return;
        };

        if self.basket.remove(rule_id).is_some() {
            self.message = format!("Unstaged rule {rule_id}.");
            return;
        }

        self.basket.insert(
            rule_id.clone(),
            CleanupBasketItem {
                rule_id: rule_id.clone(),
                status: advice.status,
                reason: advice.reason.clone(),
                required_flags: advice.required_flags.clone(),
                required_warnings: advice.required_warnings.clone(),
            },
        );
        self.message = format!("Staged rule {rule_id}; preview covers all matching targets.");
    }

    fn cycle_sort(&mut self) {
        self.sort = match self.sort {
            DiskMapSortField::Logical => DiskMapSortField::Allocated,
            DiskMapSortField::Allocated => DiskMapSortField::Files,
            DiskMapSortField::Files => DiskMapSortField::Unique,
            DiskMapSortField::Unique => DiskMapSortField::Logical,
        };
        self.reset_active_selection();
        self.message = format!("Sorted by {}.", self.sort.label());
    }

    fn move_selection(&mut self, len: usize, delta: isize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
    }

    fn move_distribution_selection(&mut self, delta: isize) {
        let len = self.distribution_rows().len();
        let selected = match self.screen {
            TuiScreen::Types => &mut self.selected_type,
            TuiScreen::Extensions => &mut self.selected_extension,
            _ => return,
        };
        if len == 0 {
            *selected = 0;
            return;
        }
        *selected = selected
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
    }

    fn move_active_selection(&mut self, delta: isize) {
        match self.screen {
            TuiScreen::Map | TuiScreen::Treemap => {
                let len = self.visible_rows().len();
                self.move_selection(len, delta);
            }
            TuiScreen::Types | TuiScreen::Extensions => self.move_distribution_selection(delta),
            TuiScreen::RootPicker => {
                let len = self.root_choices.len();
                self.move_selection(len, delta);
            }
            _ => {}
        }
    }

    fn select_map_row(&mut self, index: usize) {
        let len = self.visible_rows().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = index.min(len - 1);
        if let Some(row) = self.selected_row() {
            self.message = format!("Selected {}.", row.name);
        }
    }

    fn select_distribution_row(&mut self, index: usize) {
        let rows = self.distribution_rows();
        let len = rows.len();
        if len == 0 {
            match self.screen {
                TuiScreen::Types => self.selected_type = 0,
                TuiScreen::Extensions => self.selected_extension = 0,
                _ => {}
            }
            return;
        }
        let selected = index.min(len - 1);
        match self.screen {
            TuiScreen::Types => self.selected_type = selected,
            TuiScreen::Extensions => self.selected_extension = selected,
            _ => return,
        }
        if let Some(row) = rows.get(selected) {
            self.message = format!("Selected {}.", row.label);
        }
    }

    fn clamp_distribution_selection(&self, screen: TuiScreen) -> usize {
        let kind = match screen {
            TuiScreen::Types => DiskMapGroupKind::Type,
            TuiScreen::Extensions => DiskMapGroupKind::Extension,
            _ => return 0,
        };
        let len = self.distribution_rows_for(kind).len();
        let selected = match screen {
            TuiScreen::Types => self.selected_type,
            TuiScreen::Extensions => self.selected_extension,
            _ => 0,
        };
        if len == 0 { 0 } else { selected.min(len - 1) }
    }

    fn open_map_view(&mut self) -> TuiEffect {
        self.screen = TuiScreen::Map;
        self.selected = self
            .selected
            .min(self.visible_rows().len().saturating_sub(1));
        self.message = "Returned to map view.".to_string();
        TuiEffect::None
    }

    fn open_treemap_view(&mut self) -> TuiEffect {
        self.screen = TuiScreen::Treemap;
        self.selected = self
            .selected
            .min(self.visible_rows().len().saturating_sub(1));
        self.message = "Treemap view shows proportional disk usage for this scope.".to_string();
        TuiEffect::None
    }

    fn open_types_view(&mut self) -> TuiEffect {
        self.screen = TuiScreen::Types;
        self.selected_type = self.clamp_distribution_selection(TuiScreen::Types);
        self.message =
            "Types view shows file and directory distribution for this scan.".to_string();
        TuiEffect::None
    }

    fn open_extensions_view(&mut self) -> TuiEffect {
        self.screen = TuiScreen::Extensions;
        self.selected_extension = self.clamp_distribution_selection(TuiScreen::Extensions);
        self.message = "Extensions view shows suffix distribution for this scan.".to_string();
        TuiEffect::None
    }

    fn cycle_view_mode(&mut self) -> TuiEffect {
        match self.screen {
            TuiScreen::Map => self.open_treemap_view(),
            TuiScreen::Treemap => self.open_types_view(),
            TuiScreen::Types => self.open_extensions_view(),
            TuiScreen::Extensions => self.open_map_view(),
            _ => TuiEffect::None,
        }
    }

    fn reset_active_selection(&mut self) {
        match self.screen {
            TuiScreen::Types => self.selected_type = 0,
            TuiScreen::Extensions => self.selected_extension = 0,
            _ => self.selected = 0,
        }
    }

    fn open_screen(&mut self, screen: TuiScreen) -> TuiEffect {
        match screen {
            TuiScreen::Map => self.open_map_view(),
            TuiScreen::Treemap => self.open_treemap_view(),
            TuiScreen::Types => self.open_types_view(),
            TuiScreen::Extensions => self.open_extensions_view(),
            _ => TuiEffect::None,
        }
    }

    pub(crate) fn active_filter_label(&self) -> &'static str {
        match self.screen {
            TuiScreen::Types => "types",
            TuiScreen::Extensions => "extensions",
            _ => "paths",
        }
    }

    fn active_filter_singular_label(&self) -> &'static str {
        match self.screen {
            TuiScreen::Types => "type",
            TuiScreen::Extensions => "extension",
            _ => "path",
        }
    }

    fn active_search_query_mut(&mut self) -> &mut String {
        match self.screen {
            TuiScreen::Types => &mut self.type_search_query,
            TuiScreen::Extensions => &mut self.extension_search_query,
            _ => &mut self.search_query,
        }
    }

    fn open_help(&mut self) -> TuiEffect {
        self.previous_screen = self.screen;
        self.screen = TuiScreen::Help;
        TuiEffect::None
    }

    fn open_history(&mut self) -> TuiEffect {
        self.screen = TuiScreen::History;
        self.message = if self.history.is_empty() {
            "No cleanup history entries yet.".to_string()
        } else {
            format!("Showing {} recent history entries.", self.history.len())
        };
        TuiEffect::None
    }

    fn quit(&mut self) -> TuiEffect {
        self.should_quit = true;
        TuiEffect::Quit
    }
}

fn stageable_advice(advice: &CleanupAdvice) -> bool {
    matches!(
        advice.status,
        CleanupAdviceStatus::Cleanable
            | CleanupAdviceStatus::MaybeCleanable
            | CleanupAdviceStatus::ContainsCleanable
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca::core::cleanup_advice::{CleanupAdviceCommand, CleanupAdviceSource};
    use rebecca::core::disk_map::{
        DiskMapEntry, DiskMapEntryKind, DiskMapGroup, DiskMapGroupKind, DiskMapMetrics,
        DiskMapReport, DiskMapRoot, DiskMapRootStatus,
    };
    use rebecca::core::disk_session::DiskMapSession;
    use rebecca::core::plan::{CleanupPlan, EstimateProvenance, EstimateSource};
    use rebecca::core::{DeleteMode, PlanRequest, Platform};

    use super::*;

    #[test]
    fn space_stages_cleanable_rule_and_preview_effect_uses_workbench_request() {
        let mut app = test_app();

        assert_eq!(app.handle_key(TuiKey::Space), TuiEffect::None);
        assert!(app.basket.contains_key("linux.user-temp"));

        let effect = app.handle_key(TuiKey::Char('c'));
        assert_eq!(
            effect,
            TuiEffect::Preview(CleanupWorkbenchRequest {
                selected_rule_ids: vec!["linux.user-temp".to_string()],
                allow_moderate: false,
                allow_risky: false,
                allowed_warnings: Vec::new(),
                scan_cache: true,
                scan_backend: ScanBackendKind::PortableRecursive,
                exclude_paths: Vec::new(),
            })
        );
    }

    #[test]
    fn confirmation_requires_exact_reclaim_phrase_before_execution() {
        let mut app = test_app();
        app.handle_key(TuiKey::Space);
        let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
            Platform::current(),
            DeleteMode::DryRun,
        ));
        plan.summary.allowed_targets = 1;
        plan.summary.estimated_bytes = 42;
        app.apply_preview(plan);

        app.handle_key(TuiKey::Char('e'));
        for key in [
            TuiKey::Char('C'),
            TuiKey::Char('L'),
            TuiKey::Char('E'),
            TuiKey::Char('A'),
            TuiKey::Char('N'),
            TuiKey::Space,
            TuiKey::Char('4'),
            TuiKey::Char('2'),
        ] {
            app.handle_key(key);
        }

        assert!(matches!(
            app.handle_key(TuiKey::Enter),
            TuiEffect::Execute(_)
        ));
    }

    #[test]
    fn search_editing_accepts_space_and_escape_clears() {
        let mut app = TuiApp::root_picker(Vec::new(), ScanBackendKind::PortableRecursive, 10);
        app.screen = TuiScreen::Map;

        app.handle_key(TuiKey::Char('/'));
        for ch in "node".chars() {
            app.handle_key(TuiKey::Char(ch));
        }
        app.handle_key(TuiKey::Space);
        for ch in "cache".chars() {
            app.handle_key(TuiKey::Char(ch));
        }
        app.handle_key(TuiKey::Enter);

        assert_eq!(app.search_query, "node cache");

        app.handle_key(TuiKey::Char('/'));
        app.handle_key(TuiKey::Esc);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn busy_screen_ignores_navigation_and_allows_quit() {
        let mut app = test_app();
        app.apply_task_started("Scanning fixture...");

        assert_eq!(app.screen, TuiScreen::Busy);
        assert_eq!(app.handle_key(TuiKey::Down), TuiEffect::None);
        assert_eq!(app.message, "A background task is still running.");
        assert_eq!(app.handle_key(TuiKey::Char('q')), TuiEffect::Quit);
        assert!(app.should_quit());
    }

    #[test]
    fn busy_screen_escape_requests_task_cancellation_without_quitting() {
        let mut app = test_app();
        app.apply_task_started("Scanning fixture...");

        assert_eq!(app.handle_key(TuiKey::Esc), TuiEffect::CancelTask);
        assert!(!app.should_quit());

        app.apply_cancel_requested();

        let status = app.task_status.as_ref().unwrap();
        assert!(status.cancel_requested);
        assert_eq!(status.phase, "Cancel requested");

        app.apply_task_cancelled();

        assert_eq!(app.screen, TuiScreen::Map);
        assert!(app.task_status.is_none());
        assert_eq!(app.message, "Task cancelled.");
    }

    #[test]
    fn task_progress_updates_structured_status() {
        let mut app = test_app();
        app.apply_task_started("Scanning fixture...");

        app.apply_task_progress(TuiTaskProgressEvent::Traversal {
            root: PathBuf::from("/tmp"),
            counter: "files".to_string(),
            value: 8,
            logical_bytes: 42,
            files: 8,
            directories: 2,
        });

        let status = app.task_status.as_ref().unwrap();
        assert_eq!(status.phase, "Walking files 8");
        assert_eq!(status.files, 8);
        assert_eq!(status.directories, 2);
        assert_eq!(status.logical_bytes, 42);
        assert_eq!(status.last_event, "files: 8");
    }

    #[test]
    fn distribution_views_switch_without_losing_map_state() {
        let mut app = test_app();
        app.selected = 0;

        app.handle_key(TuiKey::Char('/'));
        for ch in "cache".chars() {
            app.handle_key(TuiKey::Char(ch));
        }
        app.handle_key(TuiKey::Enter);

        assert_eq!(app.handle_key(TuiKey::Char('t')), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Types);
        assert_eq!(app.distribution_rows()[app.selected_type].label, "Files");

        assert_eq!(app.handle_key(TuiKey::Tab), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Extensions);
        assert_eq!(
            app.distribution_rows()[app.selected_extension].label,
            ".tmp"
        );

        assert_eq!(app.handle_key(TuiKey::Tab), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Map);
        assert_eq!(app.selected, 0);
        assert_eq!(app.search_query, "cache");
    }

    #[test]
    fn tab_cycle_includes_treemap_before_distributions() {
        let mut app = test_app();

        assert_eq!(app.handle_key(TuiKey::Tab), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Treemap);

        assert_eq!(app.handle_key(TuiKey::Tab), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Types);

        assert_eq!(app.handle_key(TuiKey::Tab), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Extensions);

        assert_eq!(app.handle_key(TuiKey::Tab), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Map);
    }

    #[test]
    fn treemap_view_keeps_map_cleanup_keyboard_parity() {
        let mut app = test_app();

        assert_eq!(app.handle_key(TuiKey::Char('w')), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Treemap);

        assert_eq!(app.handle_key(TuiKey::Space), TuiEffect::None);
        assert!(app.basket.contains_key("linux.user-temp"));
        assert!(matches!(
            app.handle_key(TuiKey::Char('c')),
            TuiEffect::Preview(_)
        ));
    }

    #[test]
    fn mouse_actions_select_rows_and_do_not_execute_cleanup() {
        let mut app = test_app();

        assert_eq!(
            app.handle_mouse_action(TuiMouseAction::SwitchScreen(TuiScreen::Treemap)),
            TuiEffect::None
        );
        assert_eq!(app.screen, TuiScreen::Treemap);

        assert_eq!(
            app.handle_mouse_action(TuiMouseAction::SelectMapRow(0)),
            TuiEffect::None
        );
        assert_eq!(app.selected, 0);

        let mut plan = CleanupPlan::empty(PlanRequest::for_platform(
            Platform::current(),
            DeleteMode::DryRun,
        ));
        plan.summary.allowed_targets = 1;
        app.apply_preview(plan);
        app.handle_key(TuiKey::Char('e'));
        assert_eq!(app.screen, TuiScreen::Confirm);

        assert_eq!(
            app.handle_mouse_action(TuiMouseAction::ScrollDown),
            TuiEffect::None
        );
        assert_eq!(app.screen, TuiScreen::Confirm);
    }

    #[test]
    fn distribution_refresh_uses_current_tree_scope() {
        let mut app = test_app();

        assert_eq!(app.handle_key(TuiKey::Char('x')), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Extensions);
        assert_eq!(
            app.handle_key(TuiKey::Char('r')),
            TuiEffect::Refresh(vec![PathBuf::from("/tmp")])
        );
    }

    #[test]
    fn refresh_selected_directory_returns_refresh_effect() {
        let mut app = test_app();

        assert_eq!(
            app.handle_key(TuiKey::Char('r')),
            TuiEffect::Refresh(vec![PathBuf::from("/tmp/cache")])
        );
    }

    #[test]
    fn refresh_selected_file_is_explained_without_starting_task() {
        let mut app = TuiApp::from_session(
            DiskMapSession::from_report(test_file_report()),
            ScanBackendKind::PortableRecursive,
            100,
        );

        assert_eq!(app.handle_key(TuiKey::Char('r')), TuiEffect::None);
        assert_eq!(
            app.message,
            "Selected file cannot be refreshed as a subtree."
        );
    }

    #[test]
    fn refresh_result_can_restore_previous_session() {
        let mut app = test_app();
        let refreshed = DiskMapSession::from_report(test_child_report());

        app.apply_refresh_result(refreshed);
        assert_eq!(app.current_node_name(), "cache");

        app.handle_key(TuiKey::Char('b'));

        assert_eq!(app.current_node_name(), "tmp");
        assert!(app.message.contains("Restored previous scan."));
    }

    fn test_app() -> TuiApp {
        TuiApp::from_session(
            DiskMapSession::from_report(test_report()),
            ScanBackendKind::PortableRecursive,
            100,
        )
    }

    fn test_report() -> DiskMapReport {
        let root = PathBuf::from("/tmp");
        let path = root.join("cache");
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: DiskMapMetrics {
                    logical_bytes: 42,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(42),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 1,
                },
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: DiskMapMetrics {
                logical_bytes: 42,
                allocated_bytes: None,
                unique_logical_bytes: Some(42),
                unique_allocated_bytes: None,
                files: 1,
                directories: 1,
            },
            top_entries: vec![DiskMapEntry {
                path,
                root,
                kind: DiskMapEntryKind::Directory,
                depth: 1,
                logical_bytes: 42,
                allocated_bytes: None,
                unique_logical_bytes: Some(42),
                unique_allocated_bytes: None,
                files: 1,
                directories: 1,
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                cleanup_advice: Some(CleanupAdvice {
                    status: CleanupAdviceStatus::Cleanable,
                    source: Some(CleanupAdviceSource::CleanupRule),
                    relation: None,
                    rule_id: Some("linux.user-temp".to_string()),
                    category: Some("system".to_string()),
                    safety_level: None,
                    required_flags: Vec::new(),
                    required_warnings: Vec::new(),
                    protection_kind: None,
                    matched_path: None,
                    app_leftover: None,
                    reason: "test rule".to_string(),
                    suggested_command: Some(CleanupAdviceCommand {
                        command: "rebecca".to_string(),
                        args: vec!["clean".to_string(), "--rule".to_string()],
                    }),
                }),
            }],
            groups: vec![
                DiskMapGroup {
                    kind: DiskMapGroupKind::Type,
                    key: "file".to_string(),
                    label: "Files".to_string(),
                    metrics: DiskMapMetrics {
                        logical_bytes: 42,
                        allocated_bytes: None,
                        unique_logical_bytes: Some(42),
                        unique_allocated_bytes: None,
                        files: 1,
                        directories: 0,
                    },
                },
                DiskMapGroup {
                    kind: DiskMapGroupKind::Type,
                    key: "directory".to_string(),
                    label: "Directories".to_string(),
                    metrics: DiskMapMetrics {
                        logical_bytes: 0,
                        allocated_bytes: None,
                        unique_logical_bytes: Some(0),
                        unique_allocated_bytes: None,
                        files: 0,
                        directories: 1,
                    },
                },
                DiskMapGroup {
                    kind: DiskMapGroupKind::Extension,
                    key: ".tmp".to_string(),
                    label: ".tmp".to_string(),
                    metrics: DiskMapMetrics {
                        logical_bytes: 42,
                        allocated_bytes: None,
                        unique_logical_bytes: Some(42),
                        unique_allocated_bytes: None,
                        files: 1,
                        directories: 0,
                    },
                },
            ],
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    fn test_file_report() -> DiskMapReport {
        let root = PathBuf::from("/tmp");
        let path = root.join("cache.tmp");
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: DiskMapMetrics {
                    logical_bytes: 42,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(42),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 0,
                },
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: DiskMapMetrics {
                logical_bytes: 42,
                allocated_bytes: None,
                unique_logical_bytes: Some(42),
                unique_allocated_bytes: None,
                files: 1,
                directories: 0,
            },
            top_entries: vec![DiskMapEntry {
                path,
                root,
                kind: DiskMapEntryKind::File,
                depth: 1,
                logical_bytes: 42,
                allocated_bytes: None,
                unique_logical_bytes: Some(42),
                unique_allocated_bytes: None,
                files: 1,
                directories: 0,
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                cleanup_advice: None,
            }],
            groups: Vec::new(),
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    fn test_child_report() -> DiskMapReport {
        let root = PathBuf::from("/tmp/cache");
        let path = root.join("data.tmp");
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: DiskMapMetrics {
                    logical_bytes: 42,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(42),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 0,
                },
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: DiskMapMetrics {
                logical_bytes: 42,
                allocated_bytes: None,
                unique_logical_bytes: Some(42),
                unique_allocated_bytes: None,
                files: 1,
                directories: 0,
            },
            top_entries: vec![DiskMapEntry {
                path,
                root,
                kind: DiskMapEntryKind::File,
                depth: 1,
                logical_bytes: 42,
                allocated_bytes: None,
                unique_logical_bytes: Some(42),
                unique_allocated_bytes: None,
                files: 1,
                directories: 0,
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                cleanup_advice: None,
            }],
            groups: Vec::new(),
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }
}
