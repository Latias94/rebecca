use std::cell::RefCell;
use std::path::{Path, PathBuf};

use rebecca::core::disk_map::{DiskMapEntryKind, DiskMapGroupKind, DiskMapSortField};
use rebecca::core::disk_session::{
    DiskMapDistributionRow, DiskMapNodeId, DiskMapSession, DiskMapSubtreePatch, DiskMapVisibleRow,
};
use rebecca::core::history::HistoryEntry;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::scan::ScanBackendKind;

use crate::output::format_bytes;
use crate::tui::basket::{CleanupBasket, confirmation_phrase, toggle_advice, workbench_request};
use crate::tui::effect::TuiEffect;
use crate::tui::input::{TuiKey, TuiMouseAction};
use crate::tui::model::{TuiGroupFilter, TuiScreen};
use crate::tui::navigation::{
    RootChoice, clamp_index, cycle_workbench_screen, distribution_kind, filter_label,
    filter_singular_label, move_index,
};
use crate::tui::progress::{TuiTaskId, TuiTaskProgressEvent, TuiTaskStatus};
use crate::tui::projection::{
    TuiDistributionProjectionInput, TuiProjectionCache, TuiVisibleProjectionInput,
};
use crate::workbench::CleanupWorkbenchRequest;

#[derive(Debug, Clone)]
pub(crate) struct TuiApp {
    pub(crate) screen: TuiScreen,
    previous_screen: TuiScreen,
    task_return_screen: TuiScreen,
    pending_initial_screen: Option<TuiScreen>,
    pub(crate) root_choices: Vec<RootChoice>,
    pub(crate) session: Option<DiskMapSession>,
    pub(crate) current_parent: Option<DiskMapNodeId>,
    zoom_stack: Vec<PathBuf>,
    pub(crate) selected: usize,
    pub(crate) selected_type: usize,
    pub(crate) selected_extension: usize,
    pub(crate) sort: DiskMapSortField,
    pub(crate) search_query: String,
    pub(crate) type_search_query: String,
    pub(crate) extension_search_query: String,
    pub(crate) group_filter: Option<TuiGroupFilter>,
    search_editing: bool,
    pub(crate) basket: CleanupBasket,
    pub(crate) preview: Option<CleanupPlan>,
    pub(crate) executed: Option<CleanupPlan>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) message: String,
    pub(crate) task_status: Option<TuiTaskStatus>,
    pub(crate) scan_backend: ScanBackendKind,
    pub(crate) entry_limit: usize,
    failed_effect: Option<TuiEffect>,
    error_return_screen: TuiScreen,
    next_task_id: u64,
    session_generation: u64,
    projection: RefCell<TuiProjectionCache>,
    should_quit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiTreemapSelectionSummary {
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) drillable: bool,
    pub(crate) non_drillable_reason: Option<String>,
    pub(crate) primary_action: &'static str,
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
            pending_initial_screen: None,
            root_choices,
            session: None,
            current_parent: None,
            zoom_stack: Vec::new(),
            selected: 0,
            selected_type: 0,
            selected_extension: 0,
            sort: DiskMapSortField::Logical,
            search_query: String::new(),
            type_search_query: String::new(),
            extension_search_query: String::new(),
            group_filter: None,
            search_editing: false,
            basket: CleanupBasket::new(),
            preview: None,
            executed: None,
            history: Vec::new(),
            message: "Choose a root and press Enter to scan.".to_string(),
            task_status: None,
            scan_backend,
            entry_limit,
            failed_effect: None,
            error_return_screen: TuiScreen::RootPicker,
            next_task_id: 0,
            session_generation: 0,
            projection: RefCell::new(TuiProjectionCache::default()),
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
            pending_initial_screen: None,
            root_choices: Vec::new(),
            session: Some(session),
            current_parent,
            zoom_stack: Vec::new(),
            selected: 0,
            selected_type: 0,
            selected_extension: 0,
            sort: DiskMapSortField::Logical,
            search_query: String::new(),
            type_search_query: String::new(),
            extension_search_query: String::new(),
            group_filter: None,
            search_editing: false,
            basket: CleanupBasket::new(),
            preview: None,
            executed: None,
            history: Vec::new(),
            message: "Space stages a cleanup rule, c previews all matching targets.".to_string(),
            task_status: None,
            scan_backend,
            entry_limit,
            failed_effect: None,
            error_return_screen: TuiScreen::Map,
            next_task_id: 0,
            session_generation: 0,
            projection: RefCell::new(TuiProjectionCache::default()),
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

    pub(crate) fn set_pending_initial_screen(&mut self, screen: Option<TuiScreen>) {
        self.pending_initial_screen = screen;
        if self.session.is_some() {
            self.apply_pending_initial_screen();
        }
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

    pub(crate) fn current_scope_path(&self) -> Option<PathBuf> {
        self.session
            .as_ref()
            .and_then(|session| self.current_parent.and_then(|id| session.node_path(id)))
            .map(PathBuf::from)
    }

    pub(crate) fn current_scope_breadcrumb(&self) -> String {
        let Some(session) = self.session.as_ref() else {
            return "Roots".to_string();
        };
        let mut current = self.current_parent;
        let mut parts = Vec::new();
        while let Some(id) = current {
            let Some(node) = session.node(id) else {
                break;
            };
            parts.push(node.display_name());
            current = node.parent;
        }
        if parts.is_empty() {
            "Roots".to_string()
        } else {
            parts.reverse();
            parts.join(" > ")
        }
    }

    pub(crate) fn zoom_depth(&self) -> usize {
        self.zoom_stack.len()
    }

    pub(crate) fn active_group_filter_label(&self) -> Option<&str> {
        self.group_filter.as_ref().map(TuiGroupFilter::label)
    }

    pub(crate) fn active_scope_filter_summary(&self) -> Option<String> {
        let mut filters = Vec::new();
        let search = self.search_query.trim();
        if !search.is_empty() {
            filters.push(format!("search '{search}'"));
        }
        if let Some(label) = self.active_group_filter_label() {
            filters.push(format!("group {label}"));
        }
        (!filters.is_empty()).then(|| filters.join(", "))
    }

    pub(crate) fn visible_rows(&self) -> Vec<DiskMapVisibleRow> {
        let Some(session) = self.session.as_ref() else {
            return Vec::new();
        };
        let parent_path = self.current_parent.and_then(|id| session.node_path(id));
        self.projection
            .borrow_mut()
            .visible_rows(TuiVisibleProjectionInput {
                session,
                session_generation: self.session_generation,
                parent: self.current_parent,
                parent_path,
                sort: self.sort,
                search_query: self.search_query.as_str(),
                group_filter: self.group_filter.as_ref(),
            })
            .to_vec()
    }

    pub(crate) fn selected_row(&self) -> Option<DiskMapVisibleRow> {
        self.visible_rows().get(self.selected).cloned()
    }

    pub(crate) fn treemap_selection_summary(&self) -> Option<TuiTreemapSelectionSummary> {
        self.selected_row().map(|row| {
            let drillable = is_drillable_row(&row);
            let non_drillable_reason = (!drillable).then(|| {
                format!(
                    "{} is a {} and cannot be opened as a scope.",
                    row.name,
                    row.kind.label()
                )
            });
            TuiTreemapSelectionSummary {
                name: row.name,
                kind: row.kind.label(),
                drillable,
                non_drillable_reason,
                primary_action: if drillable {
                    "Enter/l opens this scope"
                } else {
                    "Select a directory tile"
                },
            }
        })
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
        let Some(session) = self.session.as_ref() else {
            return Vec::new();
        };
        self.projection
            .borrow_mut()
            .distribution_rows(TuiDistributionProjectionInput {
                session,
                session_generation: self.session_generation,
                kind,
                sort: self.sort,
                search_query: query,
            })
            .to_vec()
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
        distribution_kind(self.screen)
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
                self.apply_selected_group_filter();
                TuiEffect::None
            }
            TuiMouseAction::OpenTreemapRow(index) => {
                self.select_map_row(index);
                self.open_selected_treemap_tile();
                TuiEffect::None
            }
            TuiMouseAction::OpenTreemapAggregate => {
                self.message =
                    "Aggregate Other tile cannot be opened as a scope; narrow the filter or increase the terminal size."
                        .to_string();
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
        self.zoom_stack.clear();
        self.current_parent = self
            .session
            .as_ref()
            .and_then(|session| session.root_ids().first().copied());
        self.screen = TuiScreen::Map;
        self.apply_pending_initial_screen();
        self.selected = 0;
        self.selected_type = 0;
        self.selected_extension = 0;
        self.search_query.clear();
        self.type_search_query.clear();
        self.extension_search_query.clear();
        self.group_filter = None;
        self.bump_session_generation();
        self.message =
            "Scan complete. Space stages cleanup rules, c previews all matching targets."
                .to_string();
        self.task_status = None;
        self.failed_effect = None;
    }

    pub(crate) fn apply_refresh_result(&mut self, anchor: PathBuf, refreshed: DiskMapSession) {
        let restore_parent_path = self.current_scope_path();
        let restore_selected_path = self.selected_row().map(|row| row.path);

        if let Some(session) = self.session.as_mut() {
            let outcome = session
                .replace_subtree_by_path(DiskMapSubtreePatch::new(anchor.clone(), refreshed));
            self.current_parent = session.restore_parent_by_path(restore_parent_path.as_deref());
            self.message = if outcome.anchor_missing {
                format!(
                    "Refresh complete; {} no longer exists. Scope moved to the nearest remaining ancestor.",
                    anchor.display()
                )
            } else {
                format!(
                    "Refresh patched {}: {} replaced, {} inserted. Aggregates may be stale until a root refresh.",
                    anchor.display(),
                    outcome.replaced_node_count,
                    outcome.inserted_node_count
                )
            };
        } else {
            self.session = Some(refreshed);
            self.current_parent = self
                .session
                .as_ref()
                .and_then(|session| session.restore_parent_by_path(Some(anchor.as_path())));
            self.message = format!("Refresh complete for {}.", anchor.display());
        }
        self.screen = match self.task_return_screen {
            TuiScreen::Map | TuiScreen::Treemap | TuiScreen::Types | TuiScreen::Extensions => {
                self.task_return_screen
            }
            _ => TuiScreen::Map,
        };
        self.selected = self
            .selected_index_for_path(restore_selected_path.as_deref())
            .unwrap_or(0);
        self.selected_type = 0;
        self.selected_extension = 0;
        self.bump_session_generation();
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
        status.apply_event(event);
    }

    pub(crate) fn apply_cancel_requested(&mut self) {
        let message = if let Some(status) = &mut self.task_status {
            status.mark_cancel_requested();
            status.cancel_wait_message().to_string()
        } else {
            "Cancel requested; waiting for the worker to stop.".to_string()
        };
        self.message = message;
    }

    pub(crate) fn apply_task_cancelled(&mut self) {
        self.screen = self.task_return_screen;
        self.task_status = None;
        self.message = "Task cancelled.".to_string();
    }

    pub(crate) fn apply_task_already_running(&mut self) {
        self.message = "A background task is already running.".to_string();
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
        self.failed_effect = None;
    }

    pub(crate) fn apply_task_error(&mut self, message: impl Into<String>, retry: TuiEffect) {
        self.screen = TuiScreen::Error;
        self.message = message.into();
        self.task_status = None;
        self.failed_effect = Some(retry);
        self.error_return_screen = self.task_return_screen;
    }

    pub(crate) fn workbench_request(&self) -> CleanupWorkbenchRequest {
        workbench_request(&self.basket, self.scan_backend)
    }

    pub(crate) fn confirmation_phrase(&self) -> String {
        confirmation_phrase(self.preview.as_ref())
    }

    fn handle_root_picker_key(&mut self, key: TuiKey) -> TuiEffect {
        match key {
            TuiKey::Down | TuiKey::Char('j') => {
                self.selected = move_index(self.selected, self.root_choices.len(), 1);
                TuiEffect::None
            }
            TuiKey::Up | TuiKey::Char('k') => {
                self.selected = move_index(self.selected, self.root_choices.len(), -1);
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
                self.selected = move_index(self.selected, self.visible_rows().len(), 1);
                TuiEffect::None
            }
            TuiKey::Up | TuiKey::Char('k') => {
                self.selected = move_index(self.selected, self.visible_rows().len(), -1);
                TuiEffect::None
            }
            TuiKey::Right | TuiKey::Enter | TuiKey::Char('l') => {
                if self.screen == TuiScreen::Treemap {
                    self.open_selected_treemap_tile();
                } else {
                    self.open_selected_node();
                }
                TuiEffect::None
            }
            TuiKey::Left | TuiKey::Char('h') | TuiKey::Esc => {
                if self.screen == TuiScreen::Treemap {
                    self.open_treemap_previous_scope();
                } else {
                    self.open_parent_node();
                }
                TuiEffect::None
            }
            TuiKey::Backspace => {
                self.clear_group_filter();
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
            TuiKey::Enter => {
                self.apply_selected_group_filter();
                TuiEffect::None
            }
            TuiKey::Backspace => {
                self.clear_group_filter();
                TuiEffect::None
            }
            TuiKey::Char('1') | TuiKey::Esc | TuiKey::Char('h') => self.open_map_view(),
            TuiKey::Char('4') | TuiKey::Char('w') => self.open_treemap_view(),
            TuiKey::Char('2') | TuiKey::Char('t') => self.open_types_view(),
            TuiKey::Char('3') | TuiKey::Char('x') => self.open_extensions_view(),
            TuiKey::Tab => self.cycle_view_mode(),
            TuiKey::Char('r') => self.refresh_current_directory(),
            TuiKey::Char('R') => self.refresh_current_root(),
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
            self.message = "No entry is selected.".to_string();
            return;
        };
        if is_drillable_row(&row) {
            self.current_parent = Some(row.id);
            self.zoom_stack.clear();
            self.selected = 0;
            self.message = format!("Opened {}.", row.name);
        } else {
            self.message = format!(
                "{} is a {} and cannot be opened as a scope.",
                row.name,
                row.kind.label()
            );
        }
    }

    fn open_selected_treemap_tile(&mut self) {
        let Some(row) = self.selected_row() else {
            self.message = "No treemap tile is selected.".to_string();
            return;
        };
        if !is_drillable_row(&row) {
            self.message = format!(
                "{} is a {} and cannot be opened as a scope.",
                row.name,
                row.kind.label()
            );
            return;
        }
        if let Some(scope_path) = self.current_scope_path() {
            self.zoom_stack.push(scope_path);
        }
        self.current_parent = Some(row.id);
        self.selected = 0;
        self.message = format!(
            "Opened {}. Zoom depth {}. Esc returns to the previous scope.",
            row.name,
            self.zoom_depth()
        );
    }

    fn open_treemap_previous_scope(&mut self) {
        let selected_path = self.current_scope_path();
        if let Some(previous_scope) = self.zoom_stack.pop() {
            let Some(session) = self.session.as_ref() else {
                self.message = "No scan is loaded.".to_string();
                return;
            };
            self.current_parent = session.restore_parent_by_path(Some(previous_scope.as_path()));
            self.selected = self
                .selected_index_for_path(selected_path.as_deref())
                .unwrap_or(0);
            self.message = format!(
                "Returned to {}. Zoom depth {}.",
                self.current_node_name(),
                self.zoom_depth()
            );
        } else {
            self.open_parent_node();
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
            self.zoom_stack.clear();
            self.selected = 0;
            self.message = "Moved up one level.".to_string();
        }
    }

    fn refresh_selected_directory(&mut self) -> TuiEffect {
        let Some(row) = self.selected_row() else {
            return self.refresh_current_directory();
        };
        if row.kind == DiskMapEntryKind::Directory || row.has_children {
            return TuiEffect::Refresh { anchor: row.path };
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
        TuiEffect::Refresh { anchor: path }
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
        TuiEffect::Refresh { anchor: path }
    }

    fn selected_index_for_path(&self, path: Option<&Path>) -> Option<usize> {
        let path = path?;
        self.visible_rows()
            .iter()
            .position(|row| paths_equal(&row.path, path))
    }

    fn bump_session_generation(&mut self) {
        self.session_generation = self.session_generation.saturating_add(1);
        self.invalidate_projection();
    }

    fn invalidate_projection(&self) {
        self.projection.borrow_mut().clear();
    }

    fn toggle_selected_rule(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        self.message = toggle_advice(&mut self.basket, row.cleanup_advice.as_ref());
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

    fn move_distribution_selection(&mut self, delta: isize) {
        let len = self.distribution_rows().len();
        let selected = match self.screen {
            TuiScreen::Types => &mut self.selected_type,
            TuiScreen::Extensions => &mut self.selected_extension,
            _ => return,
        };
        *selected = move_index(*selected, len, delta);
    }

    fn move_active_selection(&mut self, delta: isize) {
        match self.screen {
            TuiScreen::Map | TuiScreen::Treemap => {
                let len = self.visible_rows().len();
                self.selected = move_index(self.selected, len, delta);
            }
            TuiScreen::Types | TuiScreen::Extensions => self.move_distribution_selection(delta),
            TuiScreen::RootPicker => {
                let len = self.root_choices.len();
                self.selected = move_index(self.selected, len, delta);
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
        self.selected = clamp_index(index, len);
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
        let selected = clamp_index(index, len);
        match self.screen {
            TuiScreen::Types => self.selected_type = selected,
            TuiScreen::Extensions => self.selected_extension = selected,
            _ => return,
        }
        if let Some(row) = rows.get(selected) {
            self.message = format!("Selected {}.", row.label);
        }
    }

    fn apply_selected_group_filter(&mut self) {
        let rows = self.distribution_rows();
        let selected = match self.screen {
            TuiScreen::Types => self.selected_type,
            TuiScreen::Extensions => self.selected_extension,
            _ => return,
        };
        let Some(row) = rows.get(selected) else {
            self.message = "No distribution row to filter by.".to_string();
            return;
        };
        let Some(filter) = TuiGroupFilter::from_distribution_row(row) else {
            self.message = format!("{} cannot filter the map.", row.label);
            return;
        };
        let summary = filter.summary();
        self.group_filter = Some(filter);
        self.selected = 0;
        self.screen = TuiScreen::Map;
        self.invalidate_projection();
        self.message = format!("Filtering map and treemap by {summary}. Backspace clears it.");
    }

    fn clear_group_filter(&mut self) {
        let Some(filter) = self.group_filter.take() else {
            self.message = "No type or extension filter is active.".to_string();
            return;
        };
        self.selected = 0;
        self.invalidate_projection();
        self.message = format!("Cleared {} filter.", filter.summary());
    }

    fn clamp_distribution_selection(&self, screen: TuiScreen) -> usize {
        let Some(kind) = distribution_kind(screen) else {
            return 0;
        };
        let len = self.distribution_rows_for(kind).len();
        let selected = match screen {
            TuiScreen::Types => self.selected_type,
            TuiScreen::Extensions => self.selected_extension,
            _ => 0,
        };
        clamp_index(selected, len)
    }

    fn open_map_view(&mut self) -> TuiEffect {
        self.screen = TuiScreen::Map;
        self.selected = clamp_index(self.selected, self.visible_rows().len());
        self.message = "Returned to map view.".to_string();
        TuiEffect::None
    }

    fn apply_pending_initial_screen(&mut self) {
        let Some(screen) = self.pending_initial_screen.take() else {
            return;
        };
        match screen {
            TuiScreen::Map | TuiScreen::Treemap | TuiScreen::Types | TuiScreen::Extensions => {
                self.screen = screen;
            }
            TuiScreen::RootPicker
            | TuiScreen::Busy
            | TuiScreen::Preview
            | TuiScreen::Confirm
            | TuiScreen::Executed
            | TuiScreen::History
            | TuiScreen::Help
            | TuiScreen::Error => {}
        }
    }

    fn open_treemap_view(&mut self) -> TuiEffect {
        self.screen = TuiScreen::Treemap;
        self.selected = clamp_index(self.selected, self.visible_rows().len());
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
        if let Some(screen) = cycle_workbench_screen(self.screen) {
            self.open_screen(screen)
        } else {
            TuiEffect::None
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
        filter_label(self.screen)
    }

    fn active_filter_singular_label(&self) -> &'static str {
        filter_singular_label(self.screen)
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

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str()
            .to_string_lossy()
            .replace('/', "\\")
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy().replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn is_drillable_row(row: &DiskMapVisibleRow) -> bool {
    row.kind == DiskMapEntryKind::Directory || row.has_children
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rebecca::core::cleanup_advice::{
        CleanupAdvice, CleanupAdviceCommand, CleanupAdviceSource, CleanupAdviceStatus,
    };
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
    fn distribution_enter_filters_map_projection_and_backspace_clears() {
        let mut app = TuiApp::from_session(
            DiskMapSession::from_report(test_mixed_distribution_report()),
            ScanBackendKind::PortableRecursive,
            100,
        );

        assert_eq!(app.visible_rows().len(), 3);
        assert_eq!(app.handle_key(TuiKey::Char('x')), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Extensions);

        let tmp_index = app
            .distribution_rows()
            .iter()
            .position(|row| row.key == ".tmp")
            .expect("tmp distribution row");
        app.select_distribution_row(tmp_index);

        assert_eq!(app.handle_key(TuiKey::Enter), TuiEffect::None);
        assert_eq!(app.screen, TuiScreen::Map);
        assert_eq!(app.active_group_filter_label(), Some(".tmp"));
        let rows = app.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "cache.tmp");

        assert_eq!(app.handle_key(TuiKey::Backspace), TuiEffect::None);
        assert_eq!(app.active_group_filter_label(), None);
        assert_eq!(app.visible_rows().len(), 3);
    }

    #[test]
    fn distribution_mouse_click_filters_map_projection() {
        let mut app = TuiApp::from_session(
            DiskMapSession::from_report(test_mixed_distribution_report()),
            ScanBackendKind::PortableRecursive,
            100,
        );

        assert_eq!(app.handle_key(TuiKey::Char('x')), TuiEffect::None);
        let tmp_index = app
            .distribution_rows()
            .iter()
            .position(|row| row.key == ".tmp")
            .expect("tmp distribution row");

        assert_eq!(
            app.handle_mouse_action(TuiMouseAction::SelectDistributionRow(tmp_index)),
            TuiEffect::None
        );

        let rows = app.visible_rows();
        assert_eq!(app.screen, TuiScreen::Map);
        assert_eq!(app.active_group_filter_label(), Some(".tmp"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "cache.tmp");
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
    fn treemap_enter_drills_down_and_escape_returns_zoom_scope() {
        let mut app = TuiApp::from_session(
            DiskMapSession::from_report(test_nested_report()),
            ScanBackendKind::PortableRecursive,
            100,
        );

        assert_eq!(app.handle_key(TuiKey::Char('4')), TuiEffect::None);
        assert_eq!(app.handle_key(TuiKey::Enter), TuiEffect::None);

        assert_eq!(app.screen, TuiScreen::Treemap);
        assert_eq!(app.current_node_name(), "cache");
        assert_eq!(app.zoom_depth(), 1);
        assert_eq!(app.visible_rows()[0].name, "data.tmp");

        assert_eq!(app.handle_key(TuiKey::Esc), TuiEffect::None);

        assert_eq!(app.current_node_name(), "tmp");
        assert_eq!(app.selected_row().unwrap().name, "cache");
        assert_eq!(app.zoom_depth(), 0);
    }

    #[test]
    fn treemap_file_open_reports_non_drillable_without_changing_scope() {
        let mut app = TuiApp::from_session(
            DiskMapSession::from_report(test_file_report()),
            ScanBackendKind::PortableRecursive,
            100,
        );

        assert_eq!(app.handle_key(TuiKey::Char('4')), TuiEffect::None);
        assert_eq!(app.handle_key(TuiKey::Enter), TuiEffect::None);

        assert_eq!(app.current_node_name(), "tmp");
        assert!(app.message.contains("cache.tmp is a file"));
        assert_eq!(app.zoom_depth(), 0);
    }

    #[test]
    fn treemap_open_action_drills_down_but_click_only_selects() {
        let mut app = TuiApp::from_session(
            DiskMapSession::from_report(test_nested_report()),
            ScanBackendKind::PortableRecursive,
            100,
        );
        app.handle_key(TuiKey::Char('4'));

        assert_eq!(
            app.handle_mouse_action(TuiMouseAction::SelectMapRow(0)),
            TuiEffect::None
        );
        assert_eq!(app.current_node_name(), "tmp");
        assert!(app.preview.is_none());
        assert!(app.executed.is_none());

        assert_eq!(
            app.handle_mouse_action(TuiMouseAction::OpenTreemapRow(0)),
            TuiEffect::None
        );
        assert_eq!(app.current_node_name(), "cache");
        assert_eq!(app.zoom_depth(), 1);
    }

    #[test]
    fn treemap_opening_aggregate_other_reports_non_drillable() {
        let mut app = test_app();
        app.handle_key(TuiKey::Char('4'));

        assert_eq!(
            app.handle_mouse_action(TuiMouseAction::OpenTreemapAggregate),
            TuiEffect::None
        );

        assert_eq!(app.current_node_name(), "tmp");
        assert!(
            app.message
                .contains("Aggregate Other tile cannot be opened")
        );
    }

    #[test]
    fn treemap_drilldown_preserves_group_filter_and_empty_scope_state() {
        let mut app = TuiApp::from_session(
            DiskMapSession::from_report(test_mixed_distribution_report()),
            ScanBackendKind::PortableRecursive,
            100,
        );

        assert_eq!(app.handle_key(TuiKey::Char('t')), TuiEffect::None);
        let directory_index = app
            .distribution_rows()
            .iter()
            .position(|row| row.key == "directory")
            .expect("directory distribution row");
        app.select_distribution_row(directory_index);
        assert_eq!(app.handle_key(TuiKey::Enter), TuiEffect::None);
        assert_eq!(app.active_group_filter_label(), Some("Directories"));
        assert_eq!(app.handle_key(TuiKey::Char('4')), TuiEffect::None);
        assert_eq!(app.handle_key(TuiKey::Enter), TuiEffect::None);

        assert_eq!(app.current_node_name(), "build");
        assert_eq!(app.active_group_filter_label(), Some("Directories"));
        assert!(app.visible_rows().is_empty());
        assert_eq!(app.zoom_depth(), 1);
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
            TuiEffect::Refresh {
                anchor: PathBuf::from("/tmp")
            }
        );
    }

    #[test]
    fn refresh_selected_directory_returns_refresh_effect() {
        let mut app = test_app();

        assert_eq!(
            app.handle_key(TuiKey::Char('r')),
            TuiEffect::Refresh {
                anchor: PathBuf::from("/tmp/cache")
            }
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
    fn refresh_result_patches_subtree_without_scan_restore_stack() {
        let mut app = test_app();
        let refreshed = DiskMapSession::from_report(test_child_report());

        app.apply_refresh_result(PathBuf::from("/tmp/cache"), refreshed);
        assert_eq!(app.current_node_name(), "tmp");
        assert_eq!(app.selected_row().unwrap().name, "cache");
        assert!(
            app.message.contains("Refresh patched"),
            "message: {}",
            app.message
        );
        assert!(app.message.contains("cache"));

        app.handle_key(TuiKey::Enter);
        assert_eq!(app.current_node_name(), "cache");
        assert!(app.visible_rows().iter().any(|row| row.name == "data.tmp"));

        assert_eq!(app.handle_key(TuiKey::Char('b')), TuiEffect::None);

        assert_eq!(app.current_node_name(), "cache");
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
                    evidence: Vec::new(),
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

    fn test_mixed_distribution_report() -> DiskMapReport {
        let root = PathBuf::from("/tmp");
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: test_metrics(72, 2, 1),
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: test_metrics(72, 2, 1),
            top_entries: vec![
                test_entry(
                    &root,
                    "cache.tmp",
                    DiskMapEntryKind::File,
                    test_metrics(10, 1, 0),
                ),
                test_entry(
                    &root,
                    "log.txt",
                    DiskMapEntryKind::File,
                    test_metrics(20, 1, 0),
                ),
                test_entry(
                    &root,
                    "build",
                    DiskMapEntryKind::Directory,
                    test_metrics(42, 0, 1),
                ),
            ],
            groups: vec![
                test_group(
                    DiskMapGroupKind::Type,
                    "file",
                    "Files",
                    test_metrics(30, 2, 0),
                ),
                test_group(
                    DiskMapGroupKind::Type,
                    "directory",
                    "Directories",
                    test_metrics(42, 0, 1),
                ),
                test_group(
                    DiskMapGroupKind::Extension,
                    ".tmp",
                    ".tmp",
                    test_metrics(10, 1, 0),
                ),
                test_group(
                    DiskMapGroupKind::Extension,
                    ".txt",
                    ".txt",
                    test_metrics(20, 1, 0),
                ),
            ],
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    fn test_nested_report() -> DiskMapReport {
        let root = PathBuf::from("/tmp");
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: test_metrics(43, 2, 1),
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: test_metrics(43, 2, 1),
            top_entries: vec![
                DiskMapEntry {
                    path: root.join("cache"),
                    root: root.clone(),
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
                    cleanup_advice: None,
                },
                DiskMapEntry {
                    path: root.join("cache").join("data.tmp"),
                    root: root.clone(),
                    kind: DiskMapEntryKind::File,
                    depth: 2,
                    logical_bytes: 42,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(42),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 0,
                    estimate_source: EstimateSource::FreshScan,
                    estimate_provenance: EstimateProvenance::default(),
                    cleanup_advice: None,
                },
                DiskMapEntry {
                    path: root.join("small.txt"),
                    root: root.clone(),
                    kind: DiskMapEntryKind::File,
                    depth: 1,
                    logical_bytes: 1,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(1),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 0,
                    estimate_source: EstimateSource::FreshScan,
                    estimate_provenance: EstimateProvenance::default(),
                    cleanup_advice: None,
                },
            ],
            groups: Vec::new(),
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    fn test_entry(
        root: &Path,
        name: &str,
        kind: DiskMapEntryKind,
        metrics: DiskMapMetrics,
    ) -> DiskMapEntry {
        DiskMapEntry {
            path: root.join(name),
            root: root.to_path_buf(),
            kind,
            depth: 1,
            logical_bytes: metrics.logical_bytes,
            allocated_bytes: metrics.allocated_bytes,
            unique_logical_bytes: metrics.unique_logical_bytes,
            unique_allocated_bytes: metrics.unique_allocated_bytes,
            files: metrics.files,
            directories: metrics.directories,
            estimate_source: EstimateSource::FreshScan,
            estimate_provenance: EstimateProvenance::default(),
            cleanup_advice: None,
        }
    }

    fn test_group(
        kind: DiskMapGroupKind,
        key: &str,
        label: &str,
        metrics: DiskMapMetrics,
    ) -> DiskMapGroup {
        DiskMapGroup {
            kind,
            key: key.to_string(),
            label: label.to_string(),
            metrics,
        }
    }

    fn test_metrics(logical_bytes: u64, files: u64, directories: u64) -> DiskMapMetrics {
        DiskMapMetrics {
            logical_bytes,
            allocated_bytes: None,
            unique_logical_bytes: Some(logical_bytes),
            unique_allocated_bytes: None,
            files,
            directories,
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
