use std::path::PathBuf;

use crate::workbench::CleanupWorkbenchRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiEffect {
    None,
    Scan(Vec<PathBuf>),
    Refresh(Vec<PathBuf>),
    Preview(CleanupWorkbenchRequest),
    Execute(CleanupWorkbenchRequest),
    CancelTask,
    Quit,
}
