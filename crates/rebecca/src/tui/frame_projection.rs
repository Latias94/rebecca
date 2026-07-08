use rebecca::core::disk_map::DiskMapEntryKind;
use rebecca::core::disk_session::{DiskMapDistributionRow, DiskMapVisibleRow};

use crate::tui::app::TuiTreemapSelectionSummary;

#[derive(Debug, Clone, Default)]
pub(crate) struct TuiFrameProjection {
    visible_rows: Vec<DiskMapVisibleRow>,
    distribution_rows: Vec<DiskMapDistributionRow>,
    treemap_selection: Option<TuiTreemapSelectionSummary>,
}

impl TuiFrameProjection {
    pub(super) fn new(
        visible_rows: Vec<DiskMapVisibleRow>,
        distribution_rows: Vec<DiskMapDistributionRow>,
        selected: usize,
    ) -> Self {
        let treemap_selection = visible_rows.get(selected).map(treemap_selection_summary);
        Self {
            visible_rows,
            distribution_rows,
            treemap_selection,
        }
    }

    pub(crate) fn visible_rows(&self) -> &[DiskMapVisibleRow] {
        &self.visible_rows
    }

    pub(crate) fn distribution_rows(&self) -> &[DiskMapDistributionRow] {
        &self.distribution_rows
    }

    pub(crate) fn selected_row(&self, selected: usize) -> Option<&DiskMapVisibleRow> {
        self.visible_rows.get(selected)
    }

    pub(crate) fn selected_distribution_row(
        &self,
        selected: usize,
    ) -> Option<&DiskMapDistributionRow> {
        self.distribution_rows.get(selected)
    }

    pub(crate) fn treemap_selection_summary(&self) -> Option<&TuiTreemapSelectionSummary> {
        self.treemap_selection.as_ref()
    }
}

fn treemap_selection_summary(row: &DiskMapVisibleRow) -> TuiTreemapSelectionSummary {
    let drillable = row.kind == DiskMapEntryKind::Directory || row.has_children;
    let non_drillable_reason = (!drillable).then(|| {
        format!(
            "{} is a {} and cannot be opened as a scope.",
            row.name,
            row.kind.label()
        )
    });
    TuiTreemapSelectionSummary {
        name: row.name.clone(),
        kind: row.kind.label(),
        drillable,
        non_drillable_reason,
        primary_action: if drillable {
            "Enter/l opens this scope"
        } else {
            "Select a directory tile"
        },
    }
}
