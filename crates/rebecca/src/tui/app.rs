use std::collections::BTreeMap;
use std::path::PathBuf;

use rebecca::core::cleanup_advice::{CleanupAdvice, CleanupAdviceStatus};
use rebecca::core::disk_map::DiskMapSortField;
use rebecca::core::disk_session::{
    DiskMapNodeId, DiskMapSession, DiskMapSessionFilter, DiskMapVisibleRow,
};
use rebecca::core::history::HistoryEntry;
use rebecca::core::plan::CleanupPlan;
use rebecca::core::scan::ScanBackendKind;

use crate::output::format_bytes;
use crate::workbench::CleanupWorkbenchRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiScreen {
    RootPicker,
    Map,
    Preview,
    Confirm,
    Executed,
    History,
    Help,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootChoice {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupBasketItem {
    pub(crate) rule_id: String,
    pub(crate) status: CleanupAdviceStatus,
    pub(crate) reason: String,
    pub(crate) required_flags: Vec<String>,
    pub(crate) required_warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Backspace,
    Esc,
    Tab,
    Space,
    Char(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiEffect {
    None,
    Scan(Vec<PathBuf>),
    Preview(CleanupWorkbenchRequest),
    Execute(CleanupWorkbenchRequest),
    Quit,
}

#[derive(Debug, Clone)]
pub(crate) struct TuiApp {
    pub(crate) screen: TuiScreen,
    previous_screen: TuiScreen,
    pub(crate) root_choices: Vec<RootChoice>,
    pub(crate) session: Option<DiskMapSession>,
    pub(crate) current_parent: Option<DiskMapNodeId>,
    pub(crate) selected: usize,
    pub(crate) sort: DiskMapSortField,
    pub(crate) search_query: String,
    search_editing: bool,
    pub(crate) basket: BTreeMap<String, CleanupBasketItem>,
    pub(crate) preview: Option<CleanupPlan>,
    pub(crate) executed: Option<CleanupPlan>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) message: String,
    pub(crate) scan_backend: ScanBackendKind,
    pub(crate) entry_limit: usize,
    should_quit: bool,
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
            root_choices,
            session: None,
            current_parent: None,
            selected: 0,
            sort: DiskMapSortField::Logical,
            search_query: String::new(),
            search_editing: false,
            basket: BTreeMap::new(),
            preview: None,
            executed: None,
            history: Vec::new(),
            message: "Choose a root and press Enter to scan.".to_string(),
            scan_backend,
            entry_limit,
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
            root_choices: Vec::new(),
            session: Some(session),
            current_parent,
            selected: 0,
            sort: DiskMapSortField::Logical,
            search_query: String::new(),
            search_editing: false,
            basket: BTreeMap::new(),
            preview: None,
            executed: None,
            history: Vec::new(),
            message: "Space stages a rule, c previews cleanup, ? opens help.".to_string(),
            scan_backend,
            entry_limit,
            should_quit: false,
        }
    }

    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub(crate) fn is_search_editing(&self) -> bool {
        self.search_editing
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

    pub(crate) fn handle_key(&mut self, key: TuiKey) -> TuiEffect {
        if self.search_editing {
            return self.handle_search_key(key);
        }

        match self.screen {
            TuiScreen::RootPicker => self.handle_root_picker_key(key),
            TuiScreen::Map => self.handle_map_key(key),
            TuiScreen::Preview => self.handle_preview_key(key),
            TuiScreen::Confirm => self.handle_confirm_key(key),
            TuiScreen::History => self.handle_history_key(key),
            TuiScreen::Executed | TuiScreen::Error => self.handle_terminal_screen_key(key),
            TuiScreen::Help => self.handle_help_key(key),
        }
    }

    pub(crate) fn apply_scan_result(&mut self, session: DiskMapSession) {
        self.session = Some(session);
        self.current_parent = self
            .session
            .as_ref()
            .and_then(|session| session.root_ids().first().copied());
        self.screen = TuiScreen::Map;
        self.selected = 0;
        self.message = "Scan complete. Space stages cleanup advice, c previews.".to_string();
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
    }

    pub(crate) fn apply_execution(&mut self, plan: CleanupPlan) {
        self.executed = Some(plan);
        self.screen = TuiScreen::Executed;
        self.basket.clear();
        self.preview = None;
        self.message = "Cleanup finished and history was updated.".to_string();
    }

    pub(crate) fn set_history(&mut self, entries: Vec<HistoryEntry>) {
        self.history = entries;
    }

    pub(crate) fn apply_error(&mut self, message: impl Into<String>) {
        self.screen = TuiScreen::Error;
        self.message = message.into();
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
            TuiKey::Tab => {
                self.message = "Details pane is always visible in this version.".to_string();
                TuiEffect::None
            }
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
                self.selected = 0;
                self.message = if self.search_query.is_empty() {
                    "Search cleared.".to_string()
                } else {
                    format!("Filtering paths containing '{}'.", self.search_query)
                };
            }
            TuiKey::Esc => {
                self.search_editing = false;
                self.search_query.clear();
                self.selected = 0;
                self.message = "Search cleared.".to_string();
            }
            TuiKey::Backspace => {
                self.search_query.pop();
            }
            TuiKey::Space => {
                self.search_query.push(' ');
            }
            TuiKey::Char(ch) => {
                self.search_query.push(ch);
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
            self.message = format!("Unstaged {rule_id}.");
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
        self.message = format!("Staged {rule_id}.");
    }

    fn cycle_sort(&mut self) {
        self.sort = match self.sort {
            DiskMapSortField::Logical => DiskMapSortField::Allocated,
            DiskMapSortField::Allocated => DiskMapSortField::Files,
            DiskMapSortField::Files => DiskMapSortField::Unique,
            DiskMapSortField::Unique => DiskMapSortField::Logical,
        };
        self.selected = 0;
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
        DiskMapEntry, DiskMapEntryKind, DiskMapMetrics, DiskMapReport, DiskMapRoot,
        DiskMapRootStatus,
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
            groups: Vec::new(),
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }
}
