use std::path::PathBuf;

use rebecca_core::disk_map::DiskMapGroupKind;

use crate::tui::model::TuiScreen;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootChoice {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    current
        .saturating_add_signed(delta)
        .min(len.saturating_sub(1))
}

pub(crate) fn clamp_index(index: usize, len: usize) -> usize {
    if len == 0 { 0 } else { index.min(len - 1) }
}

pub(crate) fn distribution_kind(screen: TuiScreen) -> Option<DiskMapGroupKind> {
    match screen {
        TuiScreen::Types => Some(DiskMapGroupKind::Type),
        TuiScreen::Extensions => Some(DiskMapGroupKind::Extension),
        _ => None,
    }
}

pub(crate) fn cycle_workbench_screen(screen: TuiScreen) -> Option<TuiScreen> {
    match screen {
        TuiScreen::Map => Some(TuiScreen::Treemap),
        TuiScreen::Treemap => Some(TuiScreen::Types),
        TuiScreen::Types => Some(TuiScreen::Extensions),
        TuiScreen::Extensions => Some(TuiScreen::Map),
        _ => None,
    }
}

pub(crate) fn filter_label(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::Types => "types",
        TuiScreen::Extensions => "extensions",
        _ => "paths",
    }
}

pub(crate) fn filter_singular_label(screen: TuiScreen) -> &'static str {
    match screen {
        TuiScreen::Types => "type",
        TuiScreen::Extensions => "extension",
        _ => "path",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_index_clamps_empty_underflow_and_overflow() {
        assert_eq!(move_index(4, 0, 1), 0);
        assert_eq!(move_index(0, 5, -1), 0);
        assert_eq!(move_index(3, 5, 10), 4);
        assert_eq!(move_index(2, 5, -1), 1);
        assert_eq!(move_index(2, 5, 1), 3);
    }

    #[test]
    fn cycle_workbench_screen_walks_main_views_only() {
        assert_eq!(
            cycle_workbench_screen(TuiScreen::Map),
            Some(TuiScreen::Treemap)
        );
        assert_eq!(
            cycle_workbench_screen(TuiScreen::Treemap),
            Some(TuiScreen::Types)
        );
        assert_eq!(
            cycle_workbench_screen(TuiScreen::Types),
            Some(TuiScreen::Extensions)
        );
        assert_eq!(
            cycle_workbench_screen(TuiScreen::Extensions),
            Some(TuiScreen::Map)
        );
        assert_eq!(cycle_workbench_screen(TuiScreen::Help), None);
    }

    #[test]
    fn distribution_kind_maps_distribution_screens() {
        assert_eq!(
            distribution_kind(TuiScreen::Types),
            Some(DiskMapGroupKind::Type)
        );
        assert_eq!(
            distribution_kind(TuiScreen::Extensions),
            Some(DiskMapGroupKind::Extension)
        );
        assert_eq!(distribution_kind(TuiScreen::Map), None);
    }
}
