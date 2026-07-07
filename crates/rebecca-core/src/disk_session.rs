use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cleanup_advice::CleanupAdvice;
use crate::disk_map::{
    DiskMapEntry, DiskMapEntryKind, DiskMapGroup, DiskMapGroupKind, DiskMapMetrics, DiskMapReport,
    DiskMapRootStatus, DiskMapSortField,
};
use crate::plan::{EstimateProvenance, EstimateSource};

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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskMapSessionFilter<'a> {
    pub path_contains: Option<&'a str>,
}

impl DiskMapSessionFilter<'_> {
    fn matches(self, node: &DiskMapSessionNode) -> bool {
        let Some(needle) = self.path_contains else {
            return true;
        };
        let needle = needle.trim();
        if needle.is_empty() {
            return true;
        }
        node.path
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    }
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
        }
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    } else {
        left == right
    }
}
