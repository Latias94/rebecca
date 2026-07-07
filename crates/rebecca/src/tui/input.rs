use crate::tui::model::TuiScreen;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInput {
    Key(TuiKey),
    Mouse(TuiMouseEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiMouseEvent {
    pub(crate) column: u16,
    pub(crate) row: u16,
    pub(crate) kind: TuiMouseEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiMouseEventKind {
    LeftDown,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiMouseAction {
    SwitchScreen(TuiScreen),
    SelectMapRow(usize),
    SelectDistributionRow(usize),
    OpenTreemapRow(usize),
    OpenTreemapAggregate,
    ScrollUp,
    ScrollDown,
}
