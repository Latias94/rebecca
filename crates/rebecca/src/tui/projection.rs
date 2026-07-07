use std::path::{Path, PathBuf};

use rebecca::core::disk_map::{DiskMapGroupKind, DiskMapSortField};
use rebecca::core::disk_session::{
    DiskMapDistributionFilter, DiskMapDistributionRow, DiskMapNodeId, DiskMapSession,
    DiskMapSessionFilter, DiskMapVisibleRow,
};

use crate::tui::model::TuiGroupFilter;

#[derive(Debug, Clone, Default)]
pub(crate) struct TuiProjectionCache {
    visible_rows: Option<CachedVisibleRows>,
    distribution_rows: Option<CachedDistributionRows>,
    stats: TuiProjectionStats,
}

impl TuiProjectionCache {
    pub(crate) fn clear(&mut self) {
        self.visible_rows = None;
        self.distribution_rows = None;
    }

    pub(crate) fn visible_rows(
        &mut self,
        input: TuiVisibleProjectionInput<'_>,
    ) -> &[DiskMapVisibleRow] {
        let key = input.key();
        if self
            .visible_rows
            .as_ref()
            .is_none_or(|cached| cached.key != key)
        {
            let group_filter = input.group_filter;
            let rows = input.session.visible_rows(
                input.parent,
                input.sort,
                DiskMapSessionFilter {
                    path_contains: non_empty(input.search_query),
                    entry_kind: group_filter.and_then(TuiGroupFilter::entry_kind),
                    extension: group_filter.and_then(TuiGroupFilter::extension_key),
                },
            );
            self.visible_rows = Some(CachedVisibleRows { key, rows });
            self.stats.visible_rebuilds = self.stats.visible_rebuilds.saturating_add(1);
        }

        self.visible_rows
            .as_ref()
            .map(|cached| cached.rows.as_slice())
            .unwrap_or_default()
    }

    pub(crate) fn distribution_rows(
        &mut self,
        input: TuiDistributionProjectionInput<'_>,
    ) -> &[DiskMapDistributionRow] {
        let key = input.key();
        if self
            .distribution_rows
            .as_ref()
            .is_none_or(|cached| cached.key != key)
        {
            let rows = input.session.distribution_rows(
                input.kind,
                input.sort,
                DiskMapDistributionFilter {
                    label_contains: non_empty(input.search_query),
                },
            );
            self.distribution_rows = Some(CachedDistributionRows { key, rows });
            self.stats.distribution_rebuilds = self.stats.distribution_rebuilds.saturating_add(1);
        }

        self.distribution_rows
            .as_ref()
            .map(|cached| cached.rows.as_slice())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> TuiProjectionStats {
        self.stats
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TuiProjectionStats {
    pub(crate) visible_rebuilds: u64,
    pub(crate) distribution_rebuilds: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiVisibleProjectionInput<'a> {
    pub(crate) session: &'a DiskMapSession,
    pub(crate) session_generation: u64,
    pub(crate) parent: Option<DiskMapNodeId>,
    pub(crate) parent_path: Option<&'a Path>,
    pub(crate) sort: DiskMapSortField,
    pub(crate) search_query: &'a str,
    pub(crate) group_filter: Option<&'a TuiGroupFilter>,
}

impl TuiVisibleProjectionInput<'_> {
    fn key(&self) -> VisibleRowsKey {
        VisibleRowsKey {
            session_generation: self.session_generation,
            parent_path: self.parent_path.map(PathBuf::from),
            sort: self.sort,
            search_query: self.search_query.trim().to_ascii_lowercase(),
            group_filter: self.group_filter.cloned(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiDistributionProjectionInput<'a> {
    pub(crate) session: &'a DiskMapSession,
    pub(crate) session_generation: u64,
    pub(crate) kind: DiskMapGroupKind,
    pub(crate) sort: DiskMapSortField,
    pub(crate) search_query: &'a str,
}

impl TuiDistributionProjectionInput<'_> {
    fn key(&self) -> DistributionRowsKey {
        DistributionRowsKey {
            session_generation: self.session_generation,
            kind: self.kind,
            sort: self.sort,
            search_query: self.search_query.trim().to_ascii_lowercase(),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedVisibleRows {
    key: VisibleRowsKey,
    rows: Vec<DiskMapVisibleRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleRowsKey {
    session_generation: u64,
    parent_path: Option<PathBuf>,
    sort: DiskMapSortField,
    search_query: String,
    group_filter: Option<TuiGroupFilter>,
}

#[derive(Debug, Clone)]
struct CachedDistributionRows {
    key: DistributionRowsKey,
    rows: Vec<DiskMapDistributionRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DistributionRowsKey {
    session_generation: u64,
    kind: DiskMapGroupKind,
    sort: DiskMapSortField,
    search_query: String,
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rebecca::core::disk_map::{
        DiskMapEntry, DiskMapEntryKind, DiskMapGroup, DiskMapMetrics, DiskMapReport, DiskMapRoot,
        DiskMapRootStatus,
    };
    use rebecca::core::plan::{EstimateProvenance, EstimateSource};

    use super::*;

    #[test]
    fn visible_projection_reuses_rows_until_inputs_change() {
        let session = test_session();
        let root_id = session.root_ids()[0];
        let parent_path = session.node_path(root_id);
        let mut cache = TuiProjectionCache::default();

        assert_eq!(
            cache
                .visible_rows(input(&session, root_id, parent_path, "", None, 0))
                .len(),
            3
        );
        assert_eq!(cache.stats().visible_rebuilds, 1);

        assert_eq!(
            cache
                .visible_rows(input(&session, root_id, parent_path, "", None, 0))
                .len(),
            3
        );
        assert_eq!(cache.stats().visible_rebuilds, 1);

        assert_eq!(
            cache
                .visible_rows(input(&session, root_id, parent_path, "cache", None, 0))
                .len(),
            1
        );
        assert_eq!(cache.stats().visible_rebuilds, 2);

        let file_filter = TuiGroupFilter::Type {
            entry_kind: DiskMapEntryKind::File,
            label: "Files".to_string(),
        };
        assert_eq!(
            cache
                .visible_rows(input(
                    &session,
                    root_id,
                    parent_path,
                    "",
                    Some(&file_filter),
                    0
                ))
                .len(),
            2
        );
        assert_eq!(cache.stats().visible_rebuilds, 3);

        assert_eq!(
            cache
                .visible_rows(input(
                    &session,
                    root_id,
                    parent_path,
                    "",
                    Some(&file_filter),
                    1
                ))
                .len(),
            2
        );
        assert_eq!(cache.stats().visible_rebuilds, 4);
    }

    #[test]
    fn visible_projection_filters_extensions() {
        let session = test_session();
        let root_id = session.root_ids()[0];
        let parent_path = session.node_path(root_id);
        let mut cache = TuiProjectionCache::default();
        let extension_filter = TuiGroupFilter::Extension {
            key: ".tmp".to_string(),
            label: ".tmp".to_string(),
        };

        let rows = cache.visible_rows(input(
            &session,
            root_id,
            parent_path,
            "",
            Some(&extension_filter),
            0,
        ));

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "cache.tmp");
    }

    #[test]
    fn distribution_projection_reuses_rows_until_inputs_change() {
        let session = test_session();
        let mut cache = TuiProjectionCache::default();

        assert_eq!(
            cache
                .distribution_rows(distribution_input(&session, DiskMapGroupKind::Type, "", 0))
                .len(),
            2
        );
        assert_eq!(cache.stats().distribution_rebuilds, 1);

        assert_eq!(
            cache
                .distribution_rows(distribution_input(&session, DiskMapGroupKind::Type, "", 0))
                .len(),
            2
        );
        assert_eq!(cache.stats().distribution_rebuilds, 1);

        assert_eq!(
            cache
                .distribution_rows(distribution_input(
                    &session,
                    DiskMapGroupKind::Extension,
                    "",
                    0
                ))
                .len(),
            2
        );
        assert_eq!(cache.stats().distribution_rebuilds, 2);

        assert_eq!(
            cache
                .distribution_rows(distribution_input(
                    &session,
                    DiskMapGroupKind::Extension,
                    "tmp",
                    0,
                ))
                .len(),
            1
        );
        assert_eq!(cache.stats().distribution_rebuilds, 3);
    }

    fn input<'a>(
        session: &'a DiskMapSession,
        parent: DiskMapNodeId,
        parent_path: Option<&'a Path>,
        search_query: &'a str,
        group_filter: Option<&'a TuiGroupFilter>,
        session_generation: u64,
    ) -> TuiVisibleProjectionInput<'a> {
        TuiVisibleProjectionInput {
            session,
            session_generation,
            parent: Some(parent),
            parent_path,
            sort: DiskMapSortField::Logical,
            search_query,
            group_filter,
        }
    }

    fn distribution_input<'a>(
        session: &'a DiskMapSession,
        kind: DiskMapGroupKind,
        search_query: &'a str,
        session_generation: u64,
    ) -> TuiDistributionProjectionInput<'a> {
        TuiDistributionProjectionInput {
            session,
            session_generation,
            kind,
            sort: DiskMapSortField::Logical,
            search_query,
        }
    }

    fn test_session() -> DiskMapSession {
        let root = PathBuf::from("/tmp");
        DiskMapSession::from_report(DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.clone(),
                status: DiskMapRootStatus::Scanned,
                metrics: metrics(30, 2, 1),
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: metrics(30, 2, 1),
            top_entries: vec![
                entry(
                    &root,
                    "cache.tmp",
                    DiskMapEntryKind::File,
                    metrics(10, 1, 0),
                ),
                entry(&root, "log.txt", DiskMapEntryKind::File, metrics(20, 1, 0)),
                entry(
                    &root,
                    "build",
                    DiskMapEntryKind::Directory,
                    metrics(0, 0, 1),
                ),
            ],
            groups: vec![
                group(DiskMapGroupKind::Type, "file", "Files", metrics(30, 2, 0)),
                group(
                    DiskMapGroupKind::Type,
                    "directory",
                    "Directories",
                    metrics(0, 0, 1),
                ),
                group(
                    DiskMapGroupKind::Extension,
                    ".tmp",
                    ".tmp",
                    metrics(10, 1, 0),
                ),
                group(
                    DiskMapGroupKind::Extension,
                    ".txt",
                    ".txt",
                    metrics(20, 1, 0),
                ),
            ],
            diagnostic_summary: Default::default(),
            diagnostics: Vec::new(),
        })
    }

    fn entry(
        root: &Path,
        name: &str,
        kind: DiskMapEntryKind,
        metrics: DiskMapMetrics,
    ) -> DiskMapEntry {
        DiskMapEntry {
            path: root.join(name),
            root: root.to_path_buf(),
            kind,
            depth: 1,
            logical_bytes: metrics.logical_bytes,
            allocated_bytes: metrics.allocated_bytes,
            unique_logical_bytes: metrics.unique_logical_bytes,
            unique_allocated_bytes: metrics.unique_allocated_bytes,
            files: metrics.files,
            directories: metrics.directories,
            estimate_source: EstimateSource::FreshScan,
            estimate_provenance: EstimateProvenance::default(),
            cleanup_advice: None,
        }
    }

    fn group(
        kind: DiskMapGroupKind,
        key: &str,
        label: &str,
        metrics: DiskMapMetrics,
    ) -> DiskMapGroup {
        DiskMapGroup {
            kind,
            key: key.to_string(),
            label: label.to_string(),
            metrics,
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
