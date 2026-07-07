use rebecca::core::disk_map::{DiskMapEntryKind, DiskMapGroupKind};
use rebecca::core::disk_session::DiskMapDistributionRow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiGroupFilter {
    Type {
        entry_kind: DiskMapEntryKind,
        label: String,
    },
    Extension {
        key: String,
        label: String,
    },
}

impl TuiGroupFilter {
    pub(crate) fn from_distribution_row(row: &DiskMapDistributionRow) -> Option<Self> {
        match row.kind {
            DiskMapGroupKind::Type => {
                entry_kind_from_group_key(&row.key).map(|entry_kind| Self::Type {
                    entry_kind,
                    label: row.label.clone(),
                })
            }
            DiskMapGroupKind::Extension => Some(Self::Extension {
                key: row.key.clone(),
                label: row.label.clone(),
            }),
            DiskMapGroupKind::Depth | DiskMapGroupKind::Age => None,
        }
    }

    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Type { label, .. } | Self::Extension { label, .. } => label,
        }
    }

    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Type { label, .. } => format!("type {label}"),
            Self::Extension { label, .. } => format!("extension {label}"),
        }
    }

    pub(crate) fn entry_kind(&self) -> Option<DiskMapEntryKind> {
        match self {
            Self::Type { entry_kind, .. } => Some(*entry_kind),
            Self::Extension { .. } => None,
        }
    }

    pub(crate) fn extension_key(&self) -> Option<&str> {
        match self {
            Self::Type { .. } => None,
            Self::Extension { key, .. } => Some(key),
        }
    }
}

fn entry_kind_from_group_key(key: &str) -> Option<DiskMapEntryKind> {
    match key {
        "file" => Some(DiskMapEntryKind::File),
        "directory" => Some(DiskMapEntryKind::Directory),
        "other" => Some(DiskMapEntryKind::Other),
        _ => None,
    }
}
