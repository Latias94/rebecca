use ratatui::layout::Rect;

use crate::tui::app::TuiApp;
use crate::tui::frame_projection::TuiFrameProjection;
use crate::tui::input::{TuiMouseAction, TuiMouseEvent, TuiMouseEventKind};
use crate::tui::layout;
use crate::tui::model::TuiScreen;
use crate::tui::view::ViewOptions;

pub(crate) fn hit_test(
    app: &TuiApp,
    _options: ViewOptions,
    area: Rect,
    mouse: TuiMouseEvent,
) -> Option<TuiMouseAction> {
    let layout = layout::frame(area);
    let projection = app.frame_projection();
    let point = (mouse.column, mouse.row);
    if matches!(mouse.kind, TuiMouseEventKind::LeftDown)
        && layout::rect_contains(layout.header, point)
    {
        return hit_header_tab(layout.header, point);
    }

    if !layout::rect_contains(layout.body, point) {
        return None;
    }

    match mouse.kind {
        TuiMouseEventKind::ScrollUp => Some(TuiMouseAction::ScrollUp),
        TuiMouseEventKind::ScrollDown => Some(TuiMouseAction::ScrollDown),
        TuiMouseEventKind::LeftDown => match app.screen {
            TuiScreen::Map => hit_map_row(&projection, layout.body, point),
            TuiScreen::Treemap => hit_treemap_tile(&projection, layout.body, point),
            TuiScreen::Types | TuiScreen::Extensions => {
                hit_distribution_row(&projection, layout.body, point)
            }
            _ => None,
        },
    }
}

fn hit_header_tab(area: Rect, point: (u16, u16)) -> Option<TuiMouseAction> {
    layout::header_tab_rects(area)
        .into_iter()
        .find_map(|(rect, screen)| {
            layout::rect_contains(rect, point).then_some(TuiMouseAction::SwitchScreen(screen))
        })
}

fn hit_map_row(
    projection: &TuiFrameProjection,
    area: Rect,
    point: (u16, u16),
) -> Option<TuiMouseAction> {
    let chunks = layout::workbench_body(area);
    layout::table_row_at(chunks.primary, point, projection.visible_rows().len())
        .map(TuiMouseAction::SelectMapRow)
}

fn hit_distribution_row(
    projection: &TuiFrameProjection,
    area: Rect,
    point: (u16, u16),
) -> Option<TuiMouseAction> {
    let chunks = layout::workbench_body(area);
    layout::table_row_at(chunks.primary, point, projection.distribution_rows().len())
        .map(TuiMouseAction::SelectDistributionRow)
}

fn hit_treemap_tile(
    projection: &TuiFrameProjection,
    area: Rect,
    point: (u16, u16),
) -> Option<TuiMouseAction> {
    let chunks = layout::workbench_body(area);
    let tile_area = layout::bordered_inner(chunks.primary);
    if !layout::rect_contains(tile_area, point) {
        return None;
    }
    layout::treemap_tiles(projection.visible_rows(), tile_area)
        .into_iter()
        .find_map(|tile| {
            layout::rect_contains(tile.rect, point)
                .then_some(tile.row_index)
                .flatten()
                .map(TuiMouseAction::SelectMapRow)
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca_core::disk_map::{
        DiskMapEntry, DiskMapEntryKind, DiskMapGroup, DiskMapGroupKind, DiskMapMetrics,
        DiskMapReport, DiskMapRoot, DiskMapRootStatus,
    };
    use rebecca_core::disk_session::DiskMapSession;
    use rebecca_core::plan::{EstimateProvenance, EstimateSource};
    use rebecca_core::scan::ScanBackendKind;

    use super::*;
    use crate::tui::input::TuiKey;

    #[test]
    fn hit_test_header_tab_switches_to_treemap() {
        let app = test_app();

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(18, 0),
        );

        assert_eq!(
            action,
            Some(TuiMouseAction::SwitchScreen(TuiScreen::Treemap))
        );
    }

    #[test]
    fn hit_test_map_row_selects_visible_row() {
        let app = test_app();

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(2, 2),
        );

        assert_eq!(action, Some(TuiMouseAction::SelectMapRow(0)));
    }

    #[test]
    fn hit_test_distribution_row_selects_distribution_row() {
        let mut app = test_app();
        app.handle_key(TuiKey::Char('x'));

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(2, 2),
        );

        assert_eq!(action, Some(TuiMouseAction::SelectDistributionRow(0)));
    }

    #[test]
    fn hit_test_treemap_tile_selects_map_row() {
        let mut app = test_app();
        app.handle_key(TuiKey::Char('4'));

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            mouse_left(2, 3),
        );

        assert_eq!(action, Some(TuiMouseAction::SelectMapRow(0)));
    }

    #[test]
    fn hit_test_wheel_moves_active_selection() {
        let app = test_app();

        let action = hit_test(
            &app,
            view_options(),
            Rect::new(0, 0, 100, 30),
            TuiMouseEvent {
                column: 2,
                row: 3,
                kind: TuiMouseEventKind::ScrollDown,
            },
        );

        assert_eq!(action, Some(TuiMouseAction::ScrollDown));
    }

    fn view_options() -> ViewOptions {
        ViewOptions {
            width: 100,
            visual_bars: true,
            color: true,
        }
    }

    fn mouse_left(column: u16, row: u16) -> TuiMouseEvent {
        TuiMouseEvent {
            column,
            row,
            kind: TuiMouseEventKind::LeftDown,
        }
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
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: metrics(100, 2, 1),
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: metrics(100, 2, 1),
            top_entries: vec![
                DiskMapEntry {
                    path: root.join("cache"),
                    root: root.clone(),
                    kind: DiskMapEntryKind::Directory,
                    depth: 1,
                    logical_bytes: 60,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(60),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 1,
                    estimate_source: EstimateSource::FreshScan,
                    estimate_provenance: EstimateProvenance::default(),
                    cleanup_advice: None,
                },
                DiskMapEntry {
                    path: root.join("notes.tmp"),
                    root,
                    kind: DiskMapEntryKind::File,
                    depth: 1,
                    logical_bytes: 40,
                    allocated_bytes: None,
                    unique_logical_bytes: Some(40),
                    unique_allocated_bytes: None,
                    files: 1,
                    directories: 0,
                    estimate_source: EstimateSource::FreshScan,
                    estimate_provenance: EstimateProvenance::default(),
                    cleanup_advice: None,
                },
            ],
            groups: vec![DiskMapGroup {
                kind: DiskMapGroupKind::Extension,
                key: ".tmp".to_string(),
                label: ".tmp".to_string(),
                metrics: metrics(40, 1, 0),
            }],
            volume_contexts: Vec::new(),
            workspace_insights: Vec::new(),
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    fn metrics(logical_bytes: u64, files: u64, directories: u64) -> DiskMapMetrics {
        DiskMapMetrics {
            logical_bytes,
            allocated_bytes: None,
            unique_logical_bytes: Some(logical_bytes),
            unique_allocated_bytes: None,
            files,
            directories,
        }
    }
}
