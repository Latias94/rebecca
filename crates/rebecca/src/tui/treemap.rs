use ratatui::layout::Rect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreemapItem {
    pub(crate) row_index: Option<usize>,
    pub(crate) label: String,
    pub(crate) logical_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreemapTile {
    pub(crate) row_index: Option<usize>,
    pub(crate) label: String,
    pub(crate) logical_bytes: u64,
    pub(crate) rect: Rect,
}

pub(crate) fn layout_treemap(
    items: &[TreemapItem],
    area: Rect,
    max_tiles: usize,
) -> Vec<TreemapTile> {
    if area.width == 0 || area.height == 0 || max_tiles == 0 {
        return Vec::new();
    }

    let weighted_items = capped_items(items, max_tiles);
    if weighted_items.is_empty() {
        return Vec::new();
    }

    let mut tiles = Vec::with_capacity(weighted_items.len());
    split_items(&weighted_items, area, &mut tiles);
    tiles
}

fn capped_items(items: &[TreemapItem], max_tiles: usize) -> Vec<TreemapItem> {
    let mut weighted = items
        .iter()
        .filter(|item| item.logical_bytes > 0)
        .map(|item| TreemapItem {
            row_index: item.row_index,
            label: item.label.clone(),
            logical_bytes: item.logical_bytes,
        })
        .collect::<Vec<_>>();

    if weighted.len() <= max_tiles {
        return weighted;
    }

    if max_tiles == 1 {
        let logical_bytes = weighted.iter().map(|item| item.logical_bytes).sum();
        return vec![TreemapItem {
            row_index: None,
            label: "Other".to_string(),
            logical_bytes,
        }];
    }

    let trimmed = weighted.split_off(max_tiles - 1);
    let other_bytes = trimmed.iter().map(|item| item.logical_bytes).sum();
    if other_bytes > 0 {
        weighted.push(TreemapItem {
            row_index: None,
            label: "Other".to_string(),
            logical_bytes: other_bytes,
        });
    }
    weighted
}

fn split_items(items: &[TreemapItem], area: Rect, tiles: &mut Vec<TreemapTile>) {
    if items.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    if items.len() == 1 || area.width == 1 && area.height == 1 {
        let item = &items[0];
        tiles.push(TreemapTile {
            row_index: item.row_index,
            label: item.label.clone(),
            logical_bytes: item.logical_bytes,
            rect: area,
        });
        return;
    }

    let total = total_bytes(items);
    if total == 0 {
        return;
    }

    let split_at = split_index(items, total);
    let (left, right) = items.split_at(split_at);
    if right.is_empty() {
        split_items(left, area, tiles);
        return;
    }

    let left_total = total_bytes(left);
    if area.width >= area.height {
        let left_width = split_dimension(area.width, left_total, total);
        let right_width = area.width.saturating_sub(left_width);
        split_items(
            left,
            Rect {
                x: area.x,
                y: area.y,
                width: left_width,
                height: area.height,
            },
            tiles,
        );
        split_items(
            right,
            Rect {
                x: area.x.saturating_add(left_width),
                y: area.y,
                width: right_width,
                height: area.height,
            },
            tiles,
        );
    } else {
        let top_height = split_dimension(area.height, left_total, total);
        let bottom_height = area.height.saturating_sub(top_height);
        split_items(
            left,
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: top_height,
            },
            tiles,
        );
        split_items(
            right,
            Rect {
                x: area.x,
                y: area.y.saturating_add(top_height),
                width: area.width,
                height: bottom_height,
            },
            tiles,
        );
    }
}

fn total_bytes(items: &[TreemapItem]) -> u64 {
    items.iter().map(|item| item.logical_bytes).sum()
}

fn split_index(items: &[TreemapItem], total: u64) -> usize {
    let mut acc = 0_u64;
    for (index, item) in items.iter().enumerate().take(items.len() - 1) {
        acc = acc.saturating_add(item.logical_bytes);
        if acc.saturating_mul(2) >= total {
            return index + 1;
        }
    }
    1
}

fn split_dimension(size: u16, numerator: u64, denominator: u64) -> u16 {
    if size <= 1 {
        return size;
    }
    let raw = ((size as u128) * (numerator as u128) / (denominator as u128)) as u16;
    raw.clamp(1, size - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_empty_input_returns_no_tiles() {
        let tiles = layout_treemap(&[], Rect::new(0, 0, 80, 20), 16);

        assert!(tiles.is_empty());
    }

    #[test]
    fn layout_ignores_zero_byte_items() {
        let tiles = layout_treemap(
            &[item(Some(0), "empty", 0), item(Some(1), "full", 10)],
            Rect::new(0, 0, 10, 5),
            16,
        );

        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].label, "full");
    }

    #[test]
    fn layout_tiles_stay_inside_area_without_overlap() {
        let area = Rect::new(3, 2, 30, 10);
        let tiles = layout_treemap(
            &[
                item(Some(0), "a", 50),
                item(Some(1), "b", 30),
                item(Some(2), "c", 20),
                item(Some(3), "d", 10),
            ],
            area,
            16,
        );

        for tile in &tiles {
            assert!(tile.rect.x >= area.x);
            assert!(tile.rect.y >= area.y);
            assert!(tile.rect.right() <= area.right());
            assert!(tile.rect.bottom() <= area.bottom());
        }
        for left in 0..tiles.len() {
            for right in (left + 1)..tiles.len() {
                assert!(
                    !overlaps(tiles[left].rect, tiles[right].rect),
                    "{:?} overlaps {:?}",
                    tiles[left],
                    tiles[right]
                );
            }
        }
    }

    #[test]
    fn layout_largest_item_gets_largest_area() {
        let tiles = layout_treemap(
            &[
                item(Some(0), "big", 80),
                item(Some(1), "mid", 15),
                item(Some(2), "small", 5),
            ],
            Rect::new(0, 0, 100, 20),
            16,
        );

        let big_area = tile_area(tile(&tiles, "big"));
        let mid_area = tile_area(tile(&tiles, "mid"));
        let small_area = tile_area(tile(&tiles, "small"));

        assert!(big_area >= mid_area);
        assert!(mid_area >= small_area);
    }

    #[test]
    fn layout_capped_items_aggregate_remainder_as_other() {
        let tiles = layout_treemap(
            &[
                item(Some(0), "a", 40),
                item(Some(1), "b", 30),
                item(Some(2), "c", 20),
                item(Some(3), "d", 10),
            ],
            Rect::new(0, 0, 40, 10),
            3,
        );

        assert_eq!(tiles.len(), 3);
        let other = tile(&tiles, "Other");
        assert_eq!(other.row_index, None);
        assert_eq!(other.logical_bytes, 30);
    }

    #[test]
    fn layout_tiny_area_does_not_panic() {
        let tiles = layout_treemap(
            &[item(Some(0), "a", 10), item(Some(1), "b", 5)],
            Rect::new(0, 0, 1, 1),
            16,
        );

        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].label, "a");
    }

    fn item(row_index: Option<usize>, label: &str, logical_bytes: u64) -> TreemapItem {
        TreemapItem {
            row_index,
            label: label.to_string(),
            logical_bytes,
        }
    }

    fn tile<'a>(tiles: &'a [TreemapTile], label: &str) -> &'a TreemapTile {
        tiles
            .iter()
            .find(|tile| tile.label == label)
            .unwrap_or_else(|| panic!("missing tile {label}: {tiles:?}"))
    }

    fn tile_area(tile: &TreemapTile) -> u32 {
        u32::from(tile.rect.width) * u32::from(tile.rect.height)
    }

    fn overlaps(left: Rect, right: Rect) -> bool {
        left.x < right.right()
            && left.right() > right.x
            && left.y < right.bottom()
            && left.bottom() > right.y
    }
}
