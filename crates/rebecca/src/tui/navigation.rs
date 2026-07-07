use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootChoice {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}
