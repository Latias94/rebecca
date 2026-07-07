#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiScreen {
    RootPicker,
    Map,
    Treemap,
    Types,
    Extensions,
    Busy,
    Preview,
    Confirm,
    Executed,
    History,
    Help,
    Error,
}
