use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adapter::{NtfsDirectoryEntrySource, NtfsFileReference, NtfsParsedRecord};
use crate::record::{FileNameNamespace, ParseCaveat};
use crate::record_set::NtfsRecordSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftIndex {
    entries: BTreeMap<u64, MftIndexEntry>,
    edges: Vec<DirectoryEdge>,
    edges_by_parent: BTreeMap<u64, Vec<DirectoryEdge>>,
    child_edges: BTreeMap<u64, Vec<DirectoryEdge>>,
    pub caveats: Vec<ParseCaveat>,
}

impl MftIndex {
    pub fn from_parsed_records(records: impl IntoIterator<Item = NtfsParsedRecord>) -> Self {
        let record_set = NtfsRecordSet::resolve_attribute_lists(records.into_iter().collect());
        Self::from_resolved_records(record_set.records, record_set.caveats)
    }

    pub fn from_record_set(record_set: NtfsRecordSet) -> Self {
        Self::from_resolved_records(record_set.records, record_set.caveats)
    }

    fn from_resolved_records(
        records: impl IntoIterator<Item = NtfsParsedRecord>,
        mut caveats: Vec<ParseCaveat>,
    ) -> Self {
        let records = records.into_iter().collect::<Vec<_>>();
        let references = records
            .iter()
            .map(|record| (record.reference.record_id, record.reference))
            .collect::<BTreeMap<_, _>>();
        let mut directory_entries_by_parent = BTreeMap::new();
        let mut entries = BTreeMap::new();
        let mut edge_state = DirectoryEdgeState::default();

        for mut record in records {
            let directory_entries = std::mem::take(&mut record.directory_entries);
            if !directory_entries.is_empty() {
                directory_entries_by_parent.insert(record.reference.record_id, directory_entries);
            }

            if !record.in_use {
                caveats.extend(record.caveats);
                continue;
            }

            let Some(file_name) = record.primary_file_name() else {
                caveats.extend(record.caveats);
                continue;
            };

            let mut entry_caveats = record.caveats.clone();
            let path_candidates = path_candidates(&record);
            if path_candidates.len() > 1 {
                entry_caveats.push(ParseCaveat::new(
                    "hardlink-path-candidates",
                    format!(
                        "record {} has multiple non-DOS file names; path-ranked metrics preserve visible names and unique metrics count the physical record once",
                        record.reference.record_id
                    ),
                ));
            }

            caveats.extend(entry_caveats.clone());
            let child_record_id = record.reference.record_id;
            let entry = MftIndexEntry {
                reference: record.reference,
                parent_reference: file_name.parent,
                name: file_name.name.clone(),
                path_candidates,
                modified_windows_filetime: file_name.modified_windows_filetime,
                logical_size: record.cleanup_logical_size(),
                allocated_size: record.cleanup_allocated_size(),
                is_directory: record.is_directory,
                is_reparse_point: record.is_reparse_point,
                caveats: entry_caveats,
            };

            for candidate in &entry.path_candidates {
                let parent_record_id = candidate.parent_reference.record_id;
                if parent_record_id == child_record_id {
                    continue;
                }

                let edge = DirectoryEdge::from_path_candidate(entry.reference, candidate);
                if parent_sequence_mismatches(&references, candidate.parent_reference) {
                    edge_state.push_fact(edge.rejected(
                        DirectoryEdgeSequenceStatus::ParentMismatch,
                        ParseCaveat::new(
                            "parent-sequence-mismatch",
                            format!(
                                "record {} references parent {} with stale sequence {:?}",
                                child_record_id,
                                parent_record_id,
                                candidate.parent_reference.sequence_number
                            ),
                        ),
                    ));
                    continue;
                }

                edge_state.push_traversal(edge);
            }
            entries.insert(child_record_id, entry);
        }

        cross_check_directory_entries(
            &mut entries,
            &references,
            &directory_entries_by_parent,
            &mut edge_state,
        );

        Self {
            entries,
            edges: edge_state.edges,
            edges_by_parent: edge_state.edges_by_parent,
            child_edges: edge_state.child_edges,
            caveats,
        }
    }

    pub fn get(&self, record_id: u64) -> Option<&MftIndexEntry> {
        self.entries.get(&record_id)
    }

    pub fn find_child(&self, parent_record_id: u64, name: &str) -> Option<&MftIndexEntry> {
        self.child_edges
            .get(&parent_record_id)?
            .iter()
            .filter(|edge| edge.name.eq_ignore_ascii_case(name))
            .filter_map(|edge| self.entries.get(&edge.child.record_id))
            .next()
    }

    pub fn find_path<'a>(
        &self,
        root_record_id: u64,
        components: impl IntoIterator<Item = &'a str>,
    ) -> Option<&MftIndexEntry> {
        let mut current = self.entries.get(&root_record_id)?;
        for component in components {
            current = self.find_child(current.reference.record_id, component)?;
        }
        Some(current)
    }

    pub fn entries(&self) -> impl Iterator<Item = &MftIndexEntry> {
        self.entries.values()
    }

    pub fn edges(&self) -> impl Iterator<Item = &DirectoryEdge> {
        self.edges.iter()
    }

    pub fn directory_edges(&self, parent_record_id: u64) -> impl Iterator<Item = &DirectoryEdge> {
        self.edges_by_parent
            .get(&parent_record_id)
            .into_iter()
            .flat_map(|edges| edges.iter())
    }

    pub fn child_edges(&self, parent_record_id: u64) -> impl Iterator<Item = &DirectoryEdge> {
        self.child_edges
            .get(&parent_record_id)
            .into_iter()
            .flat_map(|edges| edges.iter())
    }

    pub fn child_entries(&self, parent_record_id: u64) -> impl Iterator<Item = &MftIndexEntry> {
        self.child_edges(parent_record_id)
            .filter_map(|edge| self.entries.get(&edge.child.record_id))
    }

    pub fn aggregate_subtree(&self, root_record_id: u64) -> SubtreeSummary {
        let physical = self.aggregate_physical_subtree(root_record_id);
        SubtreeSummary {
            bytes: physical.unique_logical_bytes,
            allocated_bytes: physical.unique_allocated_bytes,
            files: physical.files,
            directories: physical.directories,
            caveats: physical.caveats,
        }
    }

    pub fn aggregate_physical_subtree(&self, root_record_id: u64) -> PhysicalMetrics {
        let mut accumulator = PhysicalMetricsAccumulator::default();
        let mut caveats = Vec::new();
        let mut stack = vec![root_record_id];
        let mut visited_directories = BTreeSet::new();

        while let Some(record_id) = stack.pop() {
            let Some(entry) = self.entries.get(&record_id) else {
                caveats.push(ParseCaveat::new(
                    "missing-record",
                    format!("record {record_id} is not present in the MFT index"),
                ));
                continue;
            };
            caveats.extend(entry.caveats.clone());

            if entry.is_reparse_point {
                caveats.push(ParseCaveat::new(
                    "reparse-point-skipped",
                    format!("record {record_id} is a reparse point"),
                ));
                continue;
            }

            if entry.is_directory {
                if !visited_directories.insert(record_id) {
                    caveats.push(ParseCaveat::new(
                        "mft-index-cycle-skipped",
                        format!(
                            "directory record {record_id} appeared more than once in a subtree"
                        ),
                    ));
                    continue;
                }
                accumulator.record_directory();
                if let Some(edges) = self.edges_by_parent.get(&record_id) {
                    for edge in edges {
                        caveats.extend(edge.caveats.clone());
                    }
                }
                if let Some(child_edges) = self.child_edges.get(&record_id) {
                    stack.extend(child_edges.iter().map(|edge| edge.child.record_id));
                }
            } else {
                accumulator.record_file_path(record_id, entry.logical_size, entry.allocated_size);
            }
        }

        let mut summary = accumulator.into_metrics();
        summary.caveats = caveats;
        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftIndexEntry {
    pub reference: NtfsFileReference,
    pub parent_reference: NtfsFileReference,
    pub name: String,
    pub path_candidates: Vec<MftPathCandidate>,
    pub modified_windows_filetime: u64,
    pub logical_size: u64,
    pub allocated_size: Option<u64>,
    pub is_directory: bool,
    pub is_reparse_point: bool,
    pub caveats: Vec<ParseCaveat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftPathCandidate {
    pub parent_reference: NtfsFileReference,
    pub namespace: FileNameNamespace,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEdge {
    pub parent: NtfsFileReference,
    pub child: NtfsFileReference,
    pub name: String,
    pub namespace: FileNameNamespace,
    pub source: DirectoryEdgeSource,
    pub sequence_status: DirectoryEdgeSequenceStatus,
    pub confidence: DirectoryEdgeConfidence,
    pub caveats: Vec<ParseCaveat>,
}

impl DirectoryEdge {
    fn from_path_candidate(child: NtfsFileReference, candidate: &MftPathCandidate) -> Self {
        Self {
            parent: candidate.parent_reference,
            child,
            name: candidate.name.clone(),
            namespace: candidate.namespace,
            source: DirectoryEdgeSource::FileName,
            sequence_status: DirectoryEdgeSequenceStatus::Matched,
            confidence: DirectoryEdgeConfidence::Trusted,
            caveats: Vec::new(),
        }
    }

    fn from_directory_entry(entry: &crate::NtfsDirectoryEntry) -> Self {
        Self {
            parent: entry.parent,
            child: entry.child,
            name: entry.name.clone(),
            namespace: entry.namespace,
            source: match entry.source {
                NtfsDirectoryEntrySource::IndexRoot => DirectoryEdgeSource::I30Root,
                NtfsDirectoryEntrySource::IndexAllocation => DirectoryEdgeSource::I30Allocation,
            },
            sequence_status: DirectoryEdgeSequenceStatus::Matched,
            confidence: DirectoryEdgeConfidence::Trusted,
            caveats: Vec::new(),
        }
    }

    fn fallback(mut self, caveat: ParseCaveat) -> Self {
        self.confidence = DirectoryEdgeConfidence::Fallback;
        self.caveats.push(caveat);
        self
    }

    fn rejected(mut self, status: DirectoryEdgeSequenceStatus, caveat: ParseCaveat) -> Self {
        self.sequence_status = status;
        self.confidence = DirectoryEdgeConfidence::Rejected;
        self.caveats.push(caveat);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DirectoryEdgeSource {
    FileName,
    I30Root,
    I30Allocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DirectoryEdgeSequenceStatus {
    Matched,
    ParentMismatch,
    ChildMismatch,
    MissingRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DirectoryEdgeConfidence {
    Trusted,
    Fallback,
    Rejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtreeSummary {
    pub bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub files: u64,
    pub directories: u64,
    pub caveats: Vec<ParseCaveat>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalMetrics {
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub unique_logical_bytes: u64,
    pub unique_allocated_bytes: Option<u64>,
    pub files: u64,
    pub directories: u64,
    pub caveats: Vec<ParseCaveat>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhysicalMetricsAccumulator {
    metrics: PhysicalMetrics,
    unique_files: BTreeMap<u64, PhysicalFileMetrics>,
}

impl PhysicalMetricsAccumulator {
    pub fn record_directory(&mut self) {
        self.metrics.directories = self.metrics.directories.saturating_add(1);
    }

    pub fn record_file_path(
        &mut self,
        record_id: u64,
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
    ) {
        let files_before = self.metrics.files;
        self.metrics.files = self.metrics.files.saturating_add(1);
        self.metrics.logical_bytes = self.metrics.logical_bytes.saturating_add(logical_bytes);
        self.metrics.allocated_bytes =
            add_file_allocated_bytes(self.metrics.allocated_bytes, files_before, allocated_bytes);

        let unique_files_before = self.unique_files.len() as u64;
        if self
            .unique_files
            .insert(
                record_id,
                PhysicalFileMetrics {
                    logical_bytes,
                    allocated_bytes,
                },
            )
            .is_none()
        {
            self.metrics.unique_logical_bytes = self
                .metrics
                .unique_logical_bytes
                .saturating_add(logical_bytes);
            self.metrics.unique_allocated_bytes = add_file_allocated_bytes(
                self.metrics.unique_allocated_bytes,
                unique_files_before,
                allocated_bytes,
            );
        }
    }

    pub fn absorb_child(&mut self, child: Self) {
        let files_before = self.metrics.files;
        self.metrics.files = self.metrics.files.saturating_add(child.metrics.files);
        self.metrics.directories = self
            .metrics
            .directories
            .saturating_add(child.metrics.directories);
        self.metrics.logical_bytes = self
            .metrics
            .logical_bytes
            .saturating_add(child.metrics.logical_bytes);
        self.metrics.allocated_bytes = add_metric_allocated_bytes(
            self.metrics.allocated_bytes,
            files_before,
            child.metrics.allocated_bytes,
            child.metrics.files,
        );

        for (record_id, file) in child.unique_files {
            let unique_files_before = self.unique_files.len() as u64;
            if self.unique_files.insert(record_id, file).is_none() {
                self.metrics.unique_logical_bytes = self
                    .metrics
                    .unique_logical_bytes
                    .saturating_add(file.logical_bytes);
                self.metrics.unique_allocated_bytes = add_file_allocated_bytes(
                    self.metrics.unique_allocated_bytes,
                    unique_files_before,
                    file.allocated_bytes,
                );
            }
        }
    }

    pub fn metrics(&self) -> PhysicalMetrics {
        self.metrics.clone()
    }

    pub fn into_metrics(self) -> PhysicalMetrics {
        self.metrics
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhysicalFileMetrics {
    logical_bytes: u64,
    allocated_bytes: Option<u64>,
}

#[derive(Debug, Default)]
struct DirectoryEdgeState {
    edges: Vec<DirectoryEdge>,
    edges_by_parent: BTreeMap<u64, Vec<DirectoryEdge>>,
    child_edges: BTreeMap<u64, Vec<DirectoryEdge>>,
    fact_memberships: BTreeSet<DirectoryEdgeFactKey>,
    traversal_memberships: BTreeMap<u64, BTreeSet<DirectoryEdgeKey>>,
    parent_child_memberships: BTreeMap<u64, BTreeSet<u64>>,
}

impl DirectoryEdgeState {
    fn push_fact(&mut self, edge: DirectoryEdge) {
        if self
            .fact_memberships
            .insert(DirectoryEdgeFactKey::from(&edge))
        {
            self.edges_by_parent
                .entry(edge.parent.record_id)
                .or_default()
                .push(edge.clone());
            self.edges.push(edge);
        }
    }

    fn push_traversal(&mut self, edge: DirectoryEdge) {
        self.push_fact(edge.clone());
        let parent_id = edge.parent.record_id;
        self.parent_child_memberships
            .entry(parent_id)
            .or_default()
            .insert(edge.child.record_id);
        if self
            .traversal_memberships
            .entry(parent_id)
            .or_default()
            .insert(DirectoryEdgeKey::from(&edge))
        {
            self.child_edges.entry(parent_id).or_default().push(edge);
        }
    }

    fn parent_child_exists(&self, parent_id: u64, child_id: u64) -> bool {
        self.parent_child_memberships
            .get(&parent_id)
            .is_some_and(|children| children.contains(&child_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DirectoryEdgeFactKey {
    parent_id: u64,
    child_id: u64,
    name: String,
    namespace: FileNameNamespace,
    source: DirectoryEdgeSource,
    sequence_status: DirectoryEdgeSequenceStatus,
    confidence: DirectoryEdgeConfidence,
    caveats: Vec<(String, String)>,
}

impl From<&DirectoryEdge> for DirectoryEdgeFactKey {
    fn from(edge: &DirectoryEdge) -> Self {
        Self {
            parent_id: edge.parent.record_id,
            child_id: edge.child.record_id,
            name: edge.name.to_ascii_lowercase(),
            namespace: edge.namespace,
            source: edge.source,
            sequence_status: edge.sequence_status,
            confidence: edge.confidence,
            caveats: edge
                .caveats
                .iter()
                .map(|caveat| (caveat.code.clone(), caveat.message.clone()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DirectoryEdgeKey {
    parent_id: u64,
    child_id: u64,
    name: String,
    namespace: FileNameNamespace,
    source: DirectoryEdgeSource,
}

impl From<&DirectoryEdge> for DirectoryEdgeKey {
    fn from(edge: &DirectoryEdge) -> Self {
        Self {
            parent_id: edge.parent.record_id,
            child_id: edge.child.record_id,
            name: edge.name.to_ascii_lowercase(),
            namespace: edge.namespace,
            source: edge.source,
        }
    }
}

fn add_file_allocated_bytes(
    current: Option<u64>,
    files_before: u64,
    file_allocated: Option<u64>,
) -> Option<u64> {
    match (current, file_allocated) {
        (None, Some(right)) if files_before == 0 => Some(right),
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    }
}

fn add_metric_allocated_bytes(
    left: Option<u64>,
    left_files: u64,
    right: Option<u64>,
    right_files: u64,
) -> Option<u64> {
    if right_files == 0 {
        return left;
    }

    match (left, right) {
        (None, Some(right)) if left_files == 0 => Some(right),
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    }
}

fn path_candidates(record: &NtfsParsedRecord) -> Vec<MftPathCandidate> {
    let mut candidates = record
        .names
        .iter()
        .filter(|name| !matches!(name.namespace, FileNameNamespace::Dos))
        .map(|name| MftPathCandidate {
            parent_reference: name.parent,
            namespace: name.namespace,
            name: name.name.clone(),
        })
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        candidates.extend(record.primary_file_name().map(|name| MftPathCandidate {
            parent_reference: name.parent,
            namespace: name.namespace,
            name: name.name.clone(),
        }));
    }

    candidates
}

fn parent_sequence_mismatches(
    references: &BTreeMap<u64, NtfsFileReference>,
    parent_reference: NtfsFileReference,
) -> bool {
    reference_sequence_mismatches(references, parent_reference)
}

fn reference_sequence_mismatches(
    references: &BTreeMap<u64, NtfsFileReference>,
    reference: NtfsFileReference,
) -> bool {
    let Some(actual) = references.get(&reference.record_id) else {
        return false;
    };
    matches!(
        (reference.sequence_number, actual.sequence_number),
        (Some(expected), Some(actual)) if sequence_number_mismatches(expected, actual)
    )
}

fn cross_check_directory_entries(
    entries: &mut BTreeMap<u64, MftIndexEntry>,
    references: &BTreeMap<u64, NtfsFileReference>,
    directory_entries_by_parent: &BTreeMap<u64, Vec<crate::NtfsDirectoryEntry>>,
    edge_state: &mut DirectoryEdgeState,
) {
    for (parent_record_id, directory_entries) in directory_entries_by_parent {
        for directory_entry in directory_entries {
            let mut edge = DirectoryEdge::from_directory_entry(directory_entry);
            if matches!(directory_entry.namespace, FileNameNamespace::Dos) {
                edge_state.push_fact(edge);
                continue;
            }

            if directory_entry.parent.record_id != *parent_record_id {
                edge_state.push_fact(edge.rejected(
                    DirectoryEdgeSequenceStatus::ParentMismatch,
                    ParseCaveat::new(
                        "directory-index-parent-mismatch",
                        format!(
                            "$I30 entry '{}' declares parent {}, but was stored on directory {}",
                            directory_entry.name,
                            directory_entry.parent.record_id,
                            parent_record_id
                        ),
                    ),
                ));
                continue;
            }

            if reference_sequence_mismatches(references, directory_entry.parent) {
                edge_state.push_fact(edge.rejected(
                    DirectoryEdgeSequenceStatus::ParentMismatch,
                    ParseCaveat::new(
                        "parent-sequence-mismatch",
                        format!(
                            "$I30 entry '{}' references parent {} sequence {:?}, but current sequence is {:?}",
                            directory_entry.name,
                            directory_entry.parent.record_id,
                            directory_entry.parent.sequence_number,
                            references
                                .get(&directory_entry.parent.record_id)
                                .and_then(|reference| reference.sequence_number)
                        ),
                    ),
                ));
                continue;
            }

            let Some(child_entry) = entries.get(&directory_entry.child.record_id) else {
                edge_state.push_fact(edge.rejected(
                    DirectoryEdgeSequenceStatus::MissingRecord,
                    ParseCaveat::new(
                        "directory-index-child-missing-record",
                        format!(
                            "$I30 entry '{}' references missing record {}",
                            directory_entry.name, directory_entry.child.record_id
                        ),
                    ),
                ));
                continue;
            };

            if matches!(
                (
                    directory_entry.child.sequence_number,
                    child_entry.reference.sequence_number
                ),
                (Some(expected), Some(actual)) if sequence_number_mismatches(expected, actual)
            ) {
                edge_state.push_fact(edge.rejected(
                    DirectoryEdgeSequenceStatus::ChildMismatch,
                    ParseCaveat::new(
                        "directory-index-child-sequence-mismatch",
                        format!(
                            "$I30 entry '{}' references record {} sequence {:?}, but current sequence is {:?}",
                            directory_entry.name,
                            directory_entry.child.record_id,
                            directory_entry.child.sequence_number,
                            child_entry.reference.sequence_number
                        ),
                    ),
                ));
                continue;
            }

            if let Some(child_entry) = entries.get_mut(&directory_entry.child.record_id) {
                push_directory_path_candidate(child_entry, directory_entry);
            }

            if edge_state.parent_child_exists(*parent_record_id, directory_entry.child.record_id) {
                edge_state.push_fact(edge);
            } else {
                let caveat = ParseCaveat::new(
                    "directory-index-parent-map-fallback",
                    format!(
                        "$I30 entry '{}' was used because it is not present in $FILE_NAME parent edges for directory {}",
                        directory_entry.name, parent_record_id
                    ),
                );
                edge = edge.fallback(caveat.clone());
                if let Some(child_entry) = entries.get_mut(&directory_entry.child.record_id) {
                    child_entry.caveats.push(caveat);
                }
                edge_state.push_traversal(edge);
            }
        }
    }
}

fn sequence_number_mismatches(expected: u16, actual: u16) -> bool {
    expected != 0 && actual != 0 && expected != actual
}

fn push_directory_path_candidate(
    child_entry: &mut MftIndexEntry,
    directory_entry: &crate::NtfsDirectoryEntry,
) {
    if matches!(directory_entry.namespace, FileNameNamespace::Dos) {
        return;
    }

    let candidate = MftPathCandidate {
        parent_reference: directory_entry.parent,
        namespace: directory_entry.namespace,
        name: directory_entry.name.clone(),
    };
    if !child_entry.path_candidates.contains(&candidate) {
        child_entry.path_candidates.push(candidate);
    }
}
