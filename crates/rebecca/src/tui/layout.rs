use ratatui::layout::{Constraint, Direction, Layout, Rect};
use rebecca::core::disk_session::DiskMapVisibleRow;

use crate::tui::model::TuiScreen;
use crate::tui::treemap::{self, TreemapItem, TreemapTile};

const TREEMAP_TILE_LIMIT: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiFrameLayout {
    pub(crate) header: Rect,
    pub(crate) body: Rect,
    pub(crate) status: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiWorkbenchLayout {
    pub(crate) primary: Rect,
    pub(crate) details: Rect,
}

pub(crate) fn frame(area: Rect) -> TuiFrameLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    TuiFrameLayout {
        header: chunks[0],
        body: chunks[1],
        status: chunks[2],
    }
}

pub(crate) fn workbench_body(area: Rect) -> TuiWorkbenchLayout {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);
    TuiWorkbenchLayout {
        primary: chunks[0],
        details: chunks[1],
    }
}

pub(crate) fn header_tab_specs() -> [(&'static str, TuiScreen); 4] {
    [
        ("1 Map", TuiScreen::Map),
        ("4 Treemap", TuiScreen::Treemap),
        ("2 Types", TuiScreen::Types),
        ("3 Ext", TuiScreen::Extensions),
    ]
}

pub(crate) fn header_tab_rects(area: Rect) -> Vec<(Rect, TuiScreen)> {
    let mut x = area.x.saturating_add("Rebecca ".len() as u16);
    let mut rects = Vec::new();
    for (label, screen) in header_tab_specs() {
        let width = (label.len() + 2) as u16;
        rects.push((
            Rect {
                x,
                y: area.y,
                width,
                height: area.height.min(1),
            },
            screen,
        ));
        x = x.saturating_add(width).saturating_add(1);
    }
    rects
}

pub(crate) fn table_row_at(area: Rect, point: (u16, u16), len: usize) -> Option<usize> {
    if len == 0 || !rect_contains(area, point) {
        return None;
    }
    let body_y = area.y.saturating_add(1);
    if point.1 < body_y {
        return None;
    }
    let body_height = area.height.saturating_sub(2);
    if body_height == 0 {
        return None;
    }
    let index = usize::from(point.1.saturating_sub(body_y));
    (index < len && index < usize::from(body_height)).then_some(index)
}

pub(crate) fn bordered_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub(crate) fn treemap_tiles(rows: &[DiskMapVisibleRow], area: Rect) -> Vec<TreemapTile> {
    let items = rows
        .iter()
        .enumerate()
        .map(|(index, row)| TreemapItem {
            row_index: Some(index),
            label: row.name.clone(),
            logical_bytes: row.metrics.logical_bytes,
        })
        .collect::<Vec<_>>();
    treemap::layout_treemap(&items, area, TREEMAP_TILE_LIMIT)
}

pub(crate) fn rect_contains(rect: Rect, point: (u16, u16)) -> bool {
    point.0 >= rect.x
        && point.0 < rect.x.saturating_add(rect.width)
        && point.1 >= rect.y
        && point.1 < rect.y.saturating_add(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_row_at_ignores_table_borders() {
        let area = Rect::new(0, 2, 20, 6);

        assert_eq!(table_row_at(area, (1, 2), 3), None);
        assert_eq!(table_row_at(area, (1, 3), 3), Some(0));
        assert_eq!(table_row_at(area, (1, 5), 3), Some(2));
        assert_eq!(table_row_at(area, (1, 6), 3), None);
    }

    #[test]
    fn header_tab_rects_match_visible_order() {
        let rects = header_tab_rects(Rect::new(0, 0, 80, 1));

        assert_eq!(rects[0].1, TuiScreen::Map);
        assert_eq!(rects[1].1, TuiScreen::Treemap);
        assert_eq!(rects[2].1, TuiScreen::Types);
        assert_eq!(rects[3].1, TuiScreen::Extensions);
        assert!(rects.windows(2).all(|pair| pair[0].0.x < pair[1].0.x));
    }
}
