use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cleanup_advice::CleanupAdvice;
use crate::disk_map::{
    DiskMapEntry, DiskMapEntryKind, DiskMapGroup, DiskMapGroupKind, DiskMapMetrics, DiskMapReport,
    DiskMapRootStatus, DiskMapSortField,
};
use crate::plan::{EstimateProvenance, EstimateSource};

const SUBTREE_REFRESH_AGGREGATE_STALE_CAVEAT: &str = "subtree-refresh-aggregate-stale";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DiskMapNodeId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapSession {
    nodes: Vec<DiskMapSessionNode>,
    root_ids: Vec<DiskMapNodeId>,
    #[serde(default)]
    totals: DiskMapMetrics,
    #[serde(default)]
    groups: Vec<DiskMapGroup>,
    #[serde(
        default,
        skip_serializing_if = "DiskMapSessionFreshness::is_fully_fresh"
    )]
    freshness: DiskMapSessionFreshness,
}

impl DiskMapSession {
    pub fn from_report(report: DiskMapReport) -> Self {
        let DiskMapReport {
            roots,
            totals,
            top_entries,
            groups,
            ..
        } = report;
        let mut builder = DiskMapSessionBuilder::default();

        for root in roots {
            if matches!(root.status, DiskMapRootStatus::Scanned) {
                builder.ensure_root(
                    root.path,
                    root.metrics,
                    root.estimate_source,
                    root.estimate_provenance,
                );
            }
        }

        for entry in top_entries {
            builder.insert_entry(entry);
        }

        builder.finish(totals, groups)
    }

    pub fn nodes(&self) -> &[DiskMapSessionNode] {
        &self.nodes
    }

    pub fn root_ids(&self) -> &[DiskMapNodeId] {
        &self.root_ids
    }

    pub fn totals(&self) -> DiskMapMetrics {
        self.totals
    }

    pub fn groups(&self) -> &[DiskMapGroup] {
        &self.groups
    }

    pub fn freshness(&self) -> &DiskMapSessionFreshness {
        &self.freshness
    }

    pub fn replace_subtree_by_path(
        &mut self,
        patch: DiskMapSubtreePatch,
    ) -> DiskMapSubtreePatchOutcome {
        let anchor_path = patch.anchor_path;
        let old_anchor = self.node_id_by_path(&anchor_path);
        let old_anchor_node = old_anchor.and_then(|id| self.node(id));
        let target_root = old_anchor_node
            .map(|node| node.root.clone())
            .or_else(|| {
                self.nearest_existing_ancestor(&anchor_path)
                    .and_then(|id| self.node(id))
                    .map(|node| node.root.clone())
            })
            .unwrap_or_else(|| anchor_path.clone());
        let target_depth = old_anchor_node
            .map(|node| node.depth)
            .unwrap_or_else(|| depth_relative_to_root(&anchor_path, &target_root));
        let mut removed = vec![false; self.nodes.len()];
        if let Some(id) = old_anchor {
            self.mark_subtree(id, &mut removed);
        }
        let replaced_node_count = removed.iter().filter(|removed| **removed).count();

        let mut builder = DiskMapSessionBuilder::default();
        self.rebuild_without_removed_subtree(&mut builder, &removed);
        let inserted_node_count = append_refreshed_subtree(
            &mut builder,
            &patch.refreshed,
            &anchor_path,
            &target_root,
            target_depth,
        );

        let mut rebuilt = builder.finish(self.totals, self.groups.clone());
        rebuilt.freshness = self.freshness.clone();
        let caveat = subtree_refresh_caveat(&anchor_path);
        rebuilt.freshness.add_caveat(caveat.clone());

        let restored_anchor_path = rebuilt
            .node_id_by_path(&anchor_path)
            .and_then(|id| rebuilt.node_path(id).map(PathBuf::from));
        let nearest_existing_ancestor_path = rebuilt
            .restore_parent_by_path(Some(&anchor_path))
            .and_then(|id| rebuilt.node_path(id).map(PathBuf::from));
        let anchor_missing = restored_anchor_path.is_none();

        *self = rebuilt;

        DiskMapSubtreePatchOutcome {
            anchor_path,
            restored_anchor_path,
            nearest_existing_ancestor_path,
            anchor_missing,
            replaced_node_count,
            inserted_node_count,
            aggregate_caveat: caveat,
        }
    }

    pub fn distribution_rows(
        &self,
        kind: DiskMapGroupKind,
        sort: DiskMapSortField,
        filter: DiskMapDistributionFilter<'_>,
    ) -> Vec<DiskMapDistributionRow> {
        let mut rows = self
            .groups
            .iter()
            .filter(|group| group.kind == kind)
            .filter(|group| filter.matches(group))
            .map(|group| DiskMapDistributionRow {
                kind: group.kind,
                key: group.key.clone(),
                label: group.label.clone(),
                metrics: group.metrics,
                scope_logical_bytes: self.totals.logical_bytes,
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            sort.metrics_value(&right.metrics)
                .cmp(&sort.metrics_value(&left.metrics))
                .then_with(|| right.metrics.logical_bytes.cmp(&left.metrics.logical_bytes))
                .then_with(|| right.metrics.files.cmp(&left.metrics.files))
                .then_with(|| left.key.cmp(&right.key))
        });
        rows
    }

    pub fn node(&self, id: DiskMapNodeId) -> Option<&DiskMapSessionNode> {
        self.nodes.get(id.0)
    }

    pub fn node_id_by_path(&self, path: impl AsRef<Path>) -> Option<DiskMapNodeId> {
        let path = path.as_ref();
        self.nodes
            .iter()
            .find(|node| same_path(&node.path, path))
            .map(|node| node.id)
    }

    pub fn node_path(&self, id: DiskMapNodeId) -> Option<&Path> {
        self.node(id).map(|node| node.path.as_path())
    }

    pub fn nearest_existing_ancestor(&self, path: impl AsRef<Path>) -> Option<DiskMapNodeId> {
        let mut current = Some(path.as_ref());
        while let Some(path) = current {
            if let Some(id) = self.node_id_by_path(path) {
                return Some(id);
            }
            current = path.parent();
        }
        None
    }

    pub fn restore_parent_by_path(&self, path: Option<&Path>) -> Option<DiskMapNodeId> {
        path.and_then(|path| {
            self.node_id_by_path(path)
                .or_else(|| self.nearest_existing_ancestor(path))
        })
        .or_else(|| self.root_ids.first().copied())
    }

    pub fn children_sorted_by(
        &self,
        id: Option<DiskMapNodeId>,
        sort: DiskMapSortField,
    ) -> Vec<DiskMapNodeId> {
        let mut ids = match id {
            Some(id) => self
                .node(id)
                .map(|node| node.children.clone())
                .unwrap_or_default(),
            None => self.root_ids.clone(),
        };
        ids.sort_by(|left, right| {
            let left = &self.nodes[left.0];
            let right = &self.nodes[right.0];
            sort.metrics_value(&right.metrics)
                .cmp(&sort.metrics_value(&left.metrics))
                .then_with(|| right.metrics.logical_bytes.cmp(&left.metrics.logical_bytes))
                .then_with(|| left.path.cmp(&right.path))
        });
        ids
    }

    pub fn visible_rows(
        &self,
        parent: Option<DiskMapNodeId>,
        sort: DiskMapSortField,
        filter: DiskMapSessionFilter<'_>,
    ) -> Vec<DiskMapVisibleRow> {
        self.children_sorted_by(parent, sort)
            .into_iter()
            .filter_map(|id| {
                let node = self.node(id)?;
                filter.matches(node).then(|| DiskMapVisibleRow {
                    id,
                    path: node.path.clone(),
                    name: node.display_name(),
                    kind: node.kind,
                    depth: node.depth,
                    metrics: node.metrics,
                    cleanup_advice: node.cleanup_advice.clone(),
                    has_children: !node.children.is_empty(),
                    synthetic: node.synthetic,
                })
            })
            .collect()
    }

    fn mark_subtree(&self, id: DiskMapNodeId, removed: &mut [bool]) {
        if let Some(slot) = removed.get_mut(id.0) {
            *slot = true;
        }
        if let Some(node) = self.node(id) {
            for child in &node.children {
                self.mark_subtree(*child, removed);
            }
        }
    }

    fn collect_subtree_ids(&self, id: DiskMapNodeId, ids: &mut Vec<DiskMapNodeId>) {
        ids.push(id);
        if let Some(node) = self.node(id) {
            for child in &node.children {
                self.collect_subtree_ids(*child, ids);
            }
        }
    }

    fn rebuild_without_removed_subtree(
        &self,
        builder: &mut DiskMapSessionBuilder,
        removed: &[bool],
    ) {
        for root_id in &self.root_ids {
            if removed.get(root_id.0).copied().unwrap_or(false) {
                continue;
            }
            if let Some(root) = self.node(*root_id) {
                builder.ensure_root(
                    root.path.clone(),
                    root.metrics,
                    root.estimate_source,
                    root.estimate_provenance.clone(),
                );
            }
        }

        for node in &self.nodes {
            if node.parent.is_none()
                || node.synthetic
                || removed.get(node.id.0).copied().unwrap_or(false)
            {
                continue;
            }
            builder.insert_entry(entry_from_node(node, node.root.clone(), node.depth));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskMapSubtreePatch {
    anchor_path: PathBuf,
    refreshed: DiskMapSession,
}

impl DiskMapSubtreePatch {
    pub fn new(anchor_path: impl Into<PathBuf>, refreshed: DiskMapSession) -> Self {
        Self {
            anchor_path: anchor_path.into(),
            refreshed,
        }
    }

    pub fn from_report(anchor_path: impl Into<PathBuf>, refreshed: DiskMapReport) -> Self {
        Self::new(anchor_path, DiskMapSession::from_report(refreshed))
    }

    pub fn anchor_path(&self) -> &Path {
        &self.anchor_path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskMapSubtreePatchOutcome {
    pub anchor_path: PathBuf,
    pub restored_anchor_path: Option<PathBuf>,
    pub nearest_existing_ancestor_path: Option<PathBuf>,
    pub anchor_missing: bool,
    pub replaced_node_count: usize,
    pub inserted_node_count: usize,
    pub aggregate_caveat: DiskMapSessionCaveat,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapSessionFreshness {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caveats: Vec<DiskMapSessionCaveat>,
}

impl DiskMapSessionFreshness {
    pub fn is_fully_fresh(&self) -> bool {
        self.caveats.is_empty()
    }

    fn add_caveat(&mut self, caveat: DiskMapSessionCaveat) {
        if !self.caveats.iter().any(|existing| existing == &caveat) {
            self.caveats.push(caveat);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapSessionCaveat {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskMapSessionFilter<'a> {
    pub path_contains: Option<&'a str>,
    pub entry_kind: Option<DiskMapEntryKind>,
    pub extension: Option<&'a str>,
}

impl DiskMapSessionFilter<'_> {
    fn matches(self, node: &DiskMapSessionNode) -> bool {
        if let Some(entry_kind) = self.entry_kind
            && node.kind != entry_kind
        {
            return false;
        }

        if let Some(extension) = self.extension
            && !node_matches_extension(node, extension)
        {
            return false;
        }

        if let Some(needle) = self.path_contains {
            let needle = needle.trim();
            if !needle.is_empty()
                && !node
                    .path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            {
                return false;
            }
        }

        true
    }
}

fn node_matches_extension(node: &DiskMapSessionNode, extension: &str) -> bool {
    if node.kind != DiskMapEntryKind::File {
        return false;
    }
    let expected = extension.trim().to_ascii_lowercase();
    if expected.is_empty() {
        return true;
    }
    let actual = node
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!(".{}", extension.to_ascii_lowercase()))
        .unwrap_or_else(|| "[no-extension]".to_string());
    actual == expected
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskMapDistributionFilter<'a> {
    pub label_contains: Option<&'a str>,
}

impl DiskMapDistributionFilter<'_> {
    fn matches(self, group: &DiskMapGroup) -> bool {
        let Some(needle) = self.label_contains else {
            return true;
        };
        let needle = needle.trim();
        if needle.is_empty() {
            return true;
        }
        let needle = needle.to_ascii_lowercase();
        group.key.to_ascii_lowercase().contains(&needle)
            || group.label.to_ascii_lowercase().contains(&needle)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskMapSessionNode {
    pub id: DiskMapNodeId,
    pub parent: Option<DiskMapNodeId>,
    pub path: PathBuf,
    pub root: PathBuf,
    pub kind: DiskMapEntryKind,
    pub depth: usize,
    pub metrics: DiskMapMetrics,
    pub estimate_source: EstimateSource,
    #[serde(default, flatten)]
    pub estimate_provenance: EstimateProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_advice: Option<CleanupAdvice>,
    pub children: Vec<DiskMapNodeId>,
    pub synthetic: bool,
}

impl DiskMapSessionNode {
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskMapVisibleRow {
    pub id: DiskMapNodeId,
    pub path: PathBuf,
    pub name: String,
    pub kind: DiskMapEntryKind,
    pub depth: usize,
    pub metrics: DiskMapMetrics,
    pub cleanup_advice: Option<CleanupAdvice>,
    pub has_children: bool,
    pub synthetic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskMapDistributionRow {
    pub kind: DiskMapGroupKind,
    pub key: String,
    pub label: String,
    pub metrics: DiskMapMetrics,
    pub scope_logical_bytes: u64,
}

#[derive(Default)]
struct DiskMapSessionBuilder {
    nodes: Vec<DiskMapSessionNode>,
    root_ids: Vec<DiskMapNodeId>,
    path_to_id: BTreeMap<PathBuf, DiskMapNodeId>,
}

impl DiskMapSessionBuilder {
    fn ensure_root(
        &mut self,
        path: PathBuf,
        metrics: DiskMapMetrics,
        estimate_source: EstimateSource,
        estimate_provenance: EstimateProvenance,
    ) -> DiskMapNodeId {
        if let Some(id) = self.path_to_id.get(&path).copied() {
            if let Some(node) = self.nodes.get_mut(id.0) {
                node.metrics = metrics;
                node.estimate_source = estimate_source;
                node.estimate_provenance = estimate_provenance;
                node.synthetic = false;
            }
            return id;
        }

        let id = self.push_node(DiskMapSessionNode {
            id: DiskMapNodeId(self.nodes.len()),
            parent: None,
            root: path.clone(),
            path,
            kind: DiskMapEntryKind::Directory,
            depth: 0,
            metrics,
            estimate_source,
            estimate_provenance,
            cleanup_advice: None,
            children: Vec::new(),
            synthetic: false,
        });
        self.root_ids.push(id);
        id
    }

    fn insert_entry(&mut self, entry: DiskMapEntry) -> DiskMapNodeId {
        let root_id = self
            .path_to_id
            .get(&entry.root)
            .copied()
            .unwrap_or_else(|| {
                self.ensure_root(
                    entry.root.clone(),
                    DiskMapMetrics::default(),
                    entry.estimate_source,
                    entry.estimate_provenance.clone(),
                )
            });
        let parent = self.ensure_parent_chain(&entry.path, &entry.root, root_id);
        let metrics = DiskMapMetrics {
            logical_bytes: entry.logical_bytes,
            allocated_bytes: entry.allocated_bytes,
            unique_logical_bytes: entry.unique_logical_bytes,
            unique_allocated_bytes: entry.unique_allocated_bytes,
            files: entry.files,
            directories: entry.directories,
        };

        if let Some(id) = self.path_to_id.get(&entry.path).copied() {
            if let Some(node) = self.nodes.get_mut(id.0) {
                node.kind = entry.kind;
                node.depth = entry.depth;
                node.metrics = metrics;
                node.estimate_source = entry.estimate_source;
                node.estimate_provenance = entry.estimate_provenance;
                node.cleanup_advice = entry.cleanup_advice;
                node.synthetic = false;
            }
            return id;
        }

        self.push_child_node(
            parent,
            entry.path,
            entry.root,
            entry.kind,
            entry.depth,
            metrics,
            entry.estimate_source,
            entry.estimate_provenance,
            entry.cleanup_advice,
            false,
        )
    }

    fn ensure_parent_chain(
        &mut self,
        path: &Path,
        root: &Path,
        root_id: DiskMapNodeId,
    ) -> DiskMapNodeId {
        let Some(parent_path) = path.parent() else {
            return root_id;
        };
        if same_path(parent_path, root) {
            return root_id;
        }
        if let Some(id) = self.path_to_id.get(parent_path).copied() {
            return id;
        }

        let parent_id = self.ensure_parent_chain(parent_path, root, root_id);
        let depth = parent_path
            .strip_prefix(root)
            .ok()
            .map(|relative| relative.components().count())
            .unwrap_or(0);
        self.push_child_node(
            parent_id,
            parent_path.to_path_buf(),
            root.to_path_buf(),
            DiskMapEntryKind::Directory,
            depth,
            DiskMapMetrics::default(),
            EstimateSource::NotMeasured,
            EstimateProvenance::default(),
            None,
            true,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "session nodes are a direct projection of disk-map entries"
    )]
    fn push_child_node(
        &mut self,
        parent: DiskMapNodeId,
        path: PathBuf,
        root: PathBuf,
        kind: DiskMapEntryKind,
        depth: usize,
        metrics: DiskMapMetrics,
        estimate_source: EstimateSource,
        estimate_provenance: EstimateProvenance,
        cleanup_advice: Option<CleanupAdvice>,
        synthetic: bool,
    ) -> DiskMapNodeId {
        let id = self.push_node(DiskMapSessionNode {
            id: DiskMapNodeId(self.nodes.len()),
            parent: Some(parent),
            path,
            root,
            kind,
            depth,
            metrics,
            estimate_source,
            estimate_provenance,
            cleanup_advice,
            children: Vec::new(),
            synthetic,
        });
        self.nodes[parent.0].children.push(id);
        id
    }

    fn push_node(&mut self, node: DiskMapSessionNode) -> DiskMapNodeId {
        let id = node.id;
        self.path_to_id.insert(node.path.clone(), id);
        self.nodes.push(node);
        id
    }

    fn finish(self, totals: DiskMapMetrics, groups: Vec<DiskMapGroup>) -> DiskMapSession {
        DiskMapSession {
            nodes: self.nodes,
            root_ids: self.root_ids,
            totals,
            groups,
            freshness: DiskMapSessionFreshness::default(),
        }
    }
}

fn append_refreshed_subtree(
    builder: &mut DiskMapSessionBuilder,
    refreshed: &DiskMapSession,
    anchor_path: &Path,
    target_root: &Path,
    target_depth: usize,
) -> usize {
    let Some(anchor_id) = refreshed.node_id_by_path(anchor_path) else {
        return 0;
    };
    let mut refreshed_ids = Vec::new();
    refreshed.collect_subtree_ids(anchor_id, &mut refreshed_ids);
    let mut inserted = 0;
    for id in refreshed_ids {
        let Some(node) = refreshed.node(id) else {
            continue;
        };
        if node.synthetic {
            continue;
        }
        let depth = remapped_depth(anchor_path, target_depth, &node.path);
        if same_path(&node.path, target_root) {
            builder.ensure_root(
                node.path.clone(),
                node.metrics,
                node.estimate_source,
                node.estimate_provenance.clone(),
            );
        } else {
            builder.insert_entry(entry_from_node(node, target_root.to_path_buf(), depth));
        }
        inserted += 1;
    }
    inserted
}

fn entry_from_node(node: &DiskMapSessionNode, root: PathBuf, depth: usize) -> DiskMapEntry {
    DiskMapEntry {
        path: node.path.clone(),
        root,
        kind: node.kind,
        depth,
        logical_bytes: node.metrics.logical_bytes,
        allocated_bytes: node.metrics.allocated_bytes,
        unique_logical_bytes: node.metrics.unique_logical_bytes,
        unique_allocated_bytes: node.metrics.unique_allocated_bytes,
        files: node.metrics.files,
        directories: node.metrics.directories,
        estimate_source: node.estimate_source,
        estimate_provenance: node.estimate_provenance.clone(),
        cleanup_advice: node.cleanup_advice.clone(),
    }
}

fn remapped_depth(anchor_path: &Path, anchor_depth: usize, path: &Path) -> usize {
    if same_path(anchor_path, path) {
        return anchor_depth;
    }
    anchor_depth.saturating_add(
        path.strip_prefix(anchor_path)
            .ok()
            .map(|relative| relative.components().count())
            .unwrap_or(0),
    )
}

fn depth_relative_to_root(path: &Path, root: &Path) -> usize {
    if same_path(path, root) {
        return 0;
    }
    path.strip_prefix(root)
        .ok()
        .map(|relative| relative.components().count())
        .unwrap_or(0)
}

fn subtree_refresh_caveat(path: &Path) -> DiskMapSessionCaveat {
    DiskMapSessionCaveat {
        code: SUBTREE_REFRESH_AGGREGATE_STALE_CAVEAT.to_string(),
        message: "subtree refresh updated a local branch; session-level totals and groups may be stale until a full root refresh".to_string(),
        path: Some(path.to_path_buf()),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        windows_path_key(left).eq_ignore_ascii_case(&windows_path_key(right))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(windows)]
fn windows_path_key(path: &Path) -> String {
    path.as_os_str().to_string_lossy().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup_advice::{CleanupAdviceRelation, CleanupAdviceSource, CleanupAdviceStatus};
    use crate::disk_map::DiskMapRoot;

    #[test]
    fn replace_subtree_updates_local_branch_and_preserves_siblings() {
        let root = path("workspace");
        let big = root.join("big");
        let old_file = big.join("old.bin");
        let new_file = big.join("new.bin");
        let sibling = root.join("small.txt");
        let mut session = DiskMapSession::from_report(report(
            &root,
            metrics(1_000, 2, 2),
            vec![
                entry(&root, &big, DiskMapEntryKind::Directory, metrics(900, 1, 1)),
                entry(&root, &old_file, DiskMapEntryKind::File, metrics(900, 1, 0)),
                entry(&root, &sibling, DiskMapEntryKind::File, metrics(100, 1, 0)),
            ],
        ));
        let mut refreshed_file = entry(
            &big,
            &new_file,
            DiskMapEntryKind::File,
            metrics(1_200, 1, 0),
        );
        refreshed_file.cleanup_advice = Some(cleanup_advice(&new_file));
        refreshed_file.estimate_provenance = EstimateProvenance {
            estimate_backend_source: Some("refreshed-fixture".to_string()),
            ..EstimateProvenance::default()
        };

        let outcome = session.replace_subtree_by_path(DiskMapSubtreePatch::from_report(
            big.clone(),
            report(&big, metrics(1_200, 1, 1), vec![refreshed_file]),
        ));

        assert_eq!(outcome.restored_anchor_path.as_deref(), Some(big.as_path()));
        assert!(!outcome.anchor_missing);
        assert_eq!(outcome.replaced_node_count, 2);
        assert_eq!(outcome.inserted_node_count, 2);
        assert!(session.node_id_by_path(&old_file).is_none());
        assert!(session.node_id_by_path(&sibling).is_some());
        let big_node = session
            .node_id_by_path(&big)
            .and_then(|id| session.node(id))
            .unwrap();
        assert_eq!(big_node.metrics.logical_bytes, 1_200);
        assert_eq!(big_node.depth, 1);
        let new_node = session
            .node_id_by_path(&new_file)
            .and_then(|id| session.node(id))
            .unwrap();
        assert_eq!(
            new_node
                .cleanup_advice
                .as_ref()
                .and_then(|advice| advice.rule_id.as_deref()),
            Some("fixture.cache")
        );
        assert_eq!(
            new_node
                .estimate_provenance
                .estimate_backend_source
                .as_deref(),
            Some("refreshed-fixture")
        );
        assert_eq!(
            session
                .freshness()
                .caveats
                .first()
                .map(|caveat| caveat.code.as_str()),
            Some(SUBTREE_REFRESH_AGGREGATE_STALE_CAVEAT)
        );
    }

    #[test]
    fn replace_subtree_removes_missing_anchor_and_restores_nearest_ancestor() {
        let root = path("workspace");
        let big = root.join("big");
        let old_file = big.join("old.bin");
        let sibling = root.join("small.txt");
        let mut session = DiskMapSession::from_report(report(
            &root,
            metrics(1_000, 2, 2),
            vec![
                entry(&root, &big, DiskMapEntryKind::Directory, metrics(900, 1, 1)),
                entry(&root, &old_file, DiskMapEntryKind::File, metrics(900, 1, 0)),
                entry(&root, &sibling, DiskMapEntryKind::File, metrics(100, 1, 0)),
            ],
        ));

        let outcome = session.replace_subtree_by_path(DiskMapSubtreePatch::from_report(
            big.clone(),
            skipped_report(&big),
        ));

        assert!(outcome.anchor_missing);
        assert_eq!(outcome.restored_anchor_path, None);
        assert_eq!(
            outcome.nearest_existing_ancestor_path.as_deref(),
            Some(root.as_path())
        );
        assert_eq!(outcome.replaced_node_count, 2);
        assert_eq!(outcome.inserted_node_count, 0);
        assert!(session.node_id_by_path(&big).is_none());
        assert!(session.node_id_by_path(&old_file).is_none());
        assert!(session.node_id_by_path(&sibling).is_some());
        assert_eq!(
            outcome.aggregate_caveat.path.as_deref(),
            Some(big.as_path())
        );
    }

    #[test]
    fn replace_subtree_can_replace_a_root() {
        let root = path("workspace");
        let old_file = root.join("old.bin");
        let new_file = root.join("new.bin");
        let mut session = DiskMapSession::from_report(report(
            &root,
            metrics(100, 1, 0),
            vec![entry(
                &root,
                &old_file,
                DiskMapEntryKind::File,
                metrics(100, 1, 0),
            )],
        ));

        let outcome = session.replace_subtree_by_path(DiskMapSubtreePatch::from_report(
            root.clone(),
            report(
                &root,
                metrics(500, 1, 0),
                vec![entry(
                    &root,
                    &new_file,
                    DiskMapEntryKind::File,
                    metrics(500, 1, 0),
                )],
            ),
        ));

        assert_eq!(
            outcome.restored_anchor_path.as_deref(),
            Some(root.as_path())
        );
        assert!(!outcome.anchor_missing);
        assert!(session.node_id_by_path(&old_file).is_none());
        assert!(session.node_id_by_path(&new_file).is_some());
        assert_eq!(session.root_ids().len(), 1);
        let root_node = session.node(session.root_ids()[0]).unwrap();
        assert_eq!(root_node.path, root);
        assert_eq!(root_node.metrics.logical_bytes, 500);
    }

    fn report(
        root: &Path,
        root_metrics: DiskMapMetrics,
        entries: Vec<DiskMapEntry>,
    ) -> DiskMapReport {
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.to_path_buf(),
                status: DiskMapRootStatus::Scanned,
                metrics: root_metrics,
                estimate_source: EstimateSource::FreshScan,
                estimate_provenance: EstimateProvenance::default(),
                reason: None,
            }],
            totals: root_metrics,
            top_entries: entries,
            ..DiskMapReport::default()
        }
    }

    fn skipped_report(root: &Path) -> DiskMapReport {
        DiskMapReport {
            roots: vec![DiskMapRoot {
                path: root.to_path_buf(),
                status: DiskMapRootStatus::Skipped,
                metrics: DiskMapMetrics::default(),
                estimate_source: EstimateSource::NotMeasured,
                estimate_provenance: EstimateProvenance::default(),
                reason: Some("missing".to_string()),
            }],
            ..DiskMapReport::default()
        }
    }

    fn entry(
        root: &Path,
        path: &Path,
        kind: DiskMapEntryKind,
        metrics: DiskMapMetrics,
    ) -> DiskMapEntry {
        DiskMapEntry {
            path: path.to_path_buf(),
            root: root.to_path_buf(),
            kind,
            depth: depth_relative_to_root(path, root),
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

    fn metrics(logical_bytes: u64, files: u64, directories: u64) -> DiskMapMetrics {
        DiskMapMetrics {
            logical_bytes,
            allocated_bytes: Some(logical_bytes),
            unique_logical_bytes: Some(logical_bytes),
            unique_allocated_bytes: Some(logical_bytes),
            files,
            directories,
        }
    }

    fn cleanup_advice(path: &Path) -> CleanupAdvice {
        CleanupAdvice {
            status: CleanupAdviceStatus::MaybeCleanable,
            source: Some(CleanupAdviceSource::CleanupRule),
            relation: Some(CleanupAdviceRelation::Exact),
            rule_id: Some("fixture.cache".to_string()),
            category: Some("fixture".to_string()),
            safety_level: None,
            required_flags: Vec::new(),
            required_warnings: Vec::new(),
            protection_kind: None,
            matched_path: Some(path.to_path_buf()),
            app_leftover: None,
            evidence: Vec::new(),
            reason: "fixture cache".to_string(),
            suggested_command: None,
        }
    }

    fn path(value: &str) -> PathBuf {
        PathBuf::from(value)
    }
}
