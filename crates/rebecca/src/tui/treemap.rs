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

    if area.width == 1 && area.height == 1 {
        return items
            .iter()
            .find(|item| item.logical_bytes > 0)
            .map(|item| {
                vec![TreemapTile {
                    row_index: item.row_index,
                    label: item.label.clone(),
                    logical_bytes: item.logical_bytes,
                    rect: area,
                }]
            })
            .unwrap_or_default();
    }

    let cell_capacity = usize::from(area.width) * usize::from(area.height);
    let weighted_items = capped_items(items, max_tiles.min(cell_capacity));
    let total = total_bytes(&weighted_items);
    if weighted_items.is_empty() || total == 0 {
        return Vec::new();
    }

    let area_cells = f64::from(area.width) * f64::from(area.height);
    let weighted_items = weighted_items
        .into_iter()
        .map(|item| WeightedTreemapItem {
            area: (item.logical_bytes as f64 / total as f64) * area_cells,
            item,
        })
        .collect::<Vec<_>>();

    let mut layout = SquarifiedLayout {
        remaining: area,
        pending: weighted_items.as_slice(),
        tiles: Vec::with_capacity(weighted_items.len()),
    };
    layout.run();
    let mut tiles = layout.tiles;
    tiles.sort_by_key(|tile| {
        (
            tile.rect.y,
            tile.rect.x,
            tile.row_index.unwrap_or(usize::MAX),
        )
    });
    tiles
}

fn capped_items(items: &[TreemapItem], max_tiles: usize) -> Vec<TreemapItem> {
    if max_tiles == 0 {
        return Vec::new();
    }

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

#[derive(Debug, Clone)]
struct WeightedTreemapItem {
    item: TreemapItem,
    area: f64,
}

struct SquarifiedLayout<'a> {
    remaining: Rect,
    pending: &'a [WeightedTreemapItem],
    tiles: Vec<TreemapTile>,
}

impl SquarifiedLayout<'_> {
    fn run(&mut self) {
        while !self.pending.is_empty() && self.remaining.width > 0 && self.remaining.height > 0 {
            let row_len = self.next_row_len();
            let (row, rest) = self.pending.split_at(row_len);
            let is_last = rest.is_empty();
            self.layout_row(row, is_last);
            self.pending = rest;
        }
    }

    fn next_row_len(&self) -> usize {
        let side = f64::from(self.remaining.width.min(self.remaining.height)).max(1.0);
        let mut row_len = 1;
        let mut current_worst = worst_aspect(&self.pending[..row_len], side);
        while row_len < self.pending.len() {
            let next_worst = worst_aspect(&self.pending[..=row_len], side);
            let break_would_leave_one_tile = self.pending.len().saturating_sub(row_len) == 1;
            if next_worst > current_worst && !break_would_leave_one_tile {
                break;
            }
            row_len += 1;
            current_worst = next_worst;
        }
        row_len
    }

    fn layout_row(&mut self, row: &[WeightedTreemapItem], is_last: bool) {
        if row.is_empty() {
            return;
        }
        if self.remaining.width == 1 && self.remaining.height == 1 {
            self.push_tile(&row[0], self.remaining);
            if is_last {
                self.remaining.width = 0;
                self.remaining.height = 0;
            } else if self.remaining.width >= self.remaining.height {
                self.remaining.x = self.remaining.x.saturating_add(self.remaining.width);
                self.remaining.width = 0;
            } else {
                self.remaining.y = self.remaining.y.saturating_add(self.remaining.height);
                self.remaining.height = 0;
            }
            return;
        }

        if self.remaining.width >= self.remaining.height {
            self.layout_horizontal_row(row, is_last);
        } else {
            self.layout_vertical_row(row, is_last);
        }
    }

    fn layout_horizontal_row(&mut self, row: &[WeightedTreemapItem], is_last: bool) {
        let height = if is_last {
            self.remaining.height
        } else {
            strip_size(row, self.remaining.width, self.remaining.height)
        };
        let lengths = proportional_lengths(self.remaining.width, row);
        let mut x = self.remaining.x;
        for (item, width) in row.iter().zip(lengths) {
            if width > 0 && height > 0 {
                self.push_tile(
                    item,
                    Rect {
                        x,
                        y: self.remaining.y,
                        width,
                        height,
                    },
                );
            }
            x = x.saturating_add(width);
        }
        self.remaining.y = self.remaining.y.saturating_add(height);
        self.remaining.height = self.remaining.height.saturating_sub(height);
    }

    fn layout_vertical_row(&mut self, row: &[WeightedTreemapItem], is_last: bool) {
        let width = if is_last {
            self.remaining.width
        } else {
            strip_size(row, self.remaining.height, self.remaining.width)
        };
        let lengths = proportional_lengths(self.remaining.height, row);
        let mut y = self.remaining.y;
        for (item, height) in row.iter().zip(lengths) {
            if width > 0 && height > 0 {
                self.push_tile(
                    item,
                    Rect {
                        x: self.remaining.x,
                        y,
                        width,
                        height,
                    },
                );
            }
            y = y.saturating_add(height);
        }
        self.remaining.x = self.remaining.x.saturating_add(width);
        self.remaining.width = self.remaining.width.saturating_sub(width);
    }

    fn push_tile(&mut self, item: &WeightedTreemapItem, rect: Rect) {
        self.tiles.push(TreemapTile {
            row_index: item.item.row_index,
            label: item.item.label.clone(),
            logical_bytes: item.item.logical_bytes,
            rect,
        });
    }
}

fn total_bytes(items: &[TreemapItem]) -> u64 {
    items.iter().map(|item| item.logical_bytes).sum()
}

fn worst_aspect(row: &[WeightedTreemapItem], side: f64) -> f64 {
    if row.is_empty() {
        return f64::INFINITY;
    }
    let sum = row.iter().map(|item| item.area).sum::<f64>().max(1.0);
    let max = row
        .iter()
        .map(|item| item.area)
        .fold(f64::MIN, f64::max)
        .max(1.0);
    let min = row
        .iter()
        .map(|item| item.area)
        .fold(f64::MAX, f64::min)
        .max(1.0);
    let side_squared = side * side;
    ((side_squared * max) / (sum * sum)).max((sum * sum) / (side_squared * min))
}

fn strip_size(row: &[WeightedTreemapItem], long_side: u16, max_short_side: u16) -> u16 {
    if long_side == 0 || max_short_side == 0 {
        return 0;
    }
    let row_area = row.iter().map(|item| item.area).sum::<f64>();
    let raw = (row_area / f64::from(long_side)).round() as u16;
    raw.clamp(1, max_short_side)
}

fn proportional_lengths(total: u16, row: &[WeightedTreemapItem]) -> Vec<u16> {
    if row.is_empty() {
        return Vec::new();
    }
    if row.len() == 1 {
        return vec![total];
    }
    let row_area = row.iter().map(|item| item.area).sum::<f64>().max(1.0);
    let mut lengths = Vec::with_capacity(row.len());
    let mut remaining = total;
    for (index, item) in row.iter().enumerate() {
        let remaining_items = row.len() - index;
        if remaining_items == 1 {
            lengths.push(remaining);
            break;
        }
        let reserve_for_rest = (remaining_items - 1).min(usize::from(remaining)) as u16;
        let max_len = remaining.saturating_sub(reserve_for_rest);
        let min_len = u16::from(remaining > reserve_for_rest);
        let raw =
            ((f64::from(total) * item.area / row_area).round() as u16).clamp(min_len, max_len);
        lengths.push(raw);
        remaining = remaining.saturating_sub(raw);
    }
    lengths
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
    fn layout_equal_items_uses_balanced_strips() {
        let tiles = layout_treemap(
            &[
                item(Some(0), "a", 25),
                item(Some(1), "b", 25),
                item(Some(2), "c", 25),
                item(Some(3), "d", 25),
            ],
            Rect::new(0, 0, 40, 20),
            16,
        );

        assert_eq!(tiles.len(), 4);
        for tile in &tiles {
            assert_eq!(tile.rect.width, 20, "{tile:?}");
            assert_eq!(tile.rect.height, 10, "{tile:?}");
        }
        assert_eq!(tile(&tiles, "a").rect, Rect::new(0, 0, 20, 10));
        assert_eq!(tile(&tiles, "b").rect, Rect::new(20, 0, 20, 10));
        assert_eq!(tile(&tiles, "c").rect, Rect::new(0, 10, 20, 10));
        assert_eq!(tile(&tiles, "d").rect, Rect::new(20, 10, 20, 10));
    }

    #[test]
    fn layout_keeps_positive_tiles_visible_when_area_has_capacity() {
        let tiles = layout_treemap(
            &[
                item(Some(0), "a", 91),
                item(Some(1), "b", 7),
                item(Some(2), "c", 2),
            ],
            Rect::new(0, 0, 30, 10),
            16,
        );

        assert_eq!(tiles.len(), 3);
        assert!(tiles.iter().all(|tile| tile.rect.width > 0));
        assert!(tiles.iter().all(|tile| tile.rect.height > 0));
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
