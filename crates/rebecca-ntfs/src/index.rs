use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adapter::{NtfsFileReference, NtfsParsedRecord};
use crate::record::{FileNameNamespace, ParseCaveat};
use crate::record_set::NtfsRecordSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftIndex {
    entries: BTreeMap<u64, MftIndexEntry>,
    children: BTreeMap<u64, Vec<u64>>,
    child_edge_caveats: BTreeMap<u64, Vec<ParseCaveat>>,
    pub caveats: Vec<ParseCaveat>,
}

impl MftIndex {
    pub fn from_parsed_records(records: impl IntoIterator<Item = NtfsParsedRecord>) -> Self {
        let record_set = NtfsRecordSet::resolve_attribute_lists(records.into_iter().collect());
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
        let directory_entries_by_parent = records
            .iter()
            .filter(|record| !record.directory_entries.is_empty())
            .map(|record| (record.reference.record_id, record.directory_entries.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut entries = BTreeMap::new();
        let mut children: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        let mut child_edge_caveats: BTreeMap<u64, Vec<ParseCaveat>> = BTreeMap::new();

        for record in records {
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
                        "record {} has multiple non-DOS file names; hardlink paths are preserved and counted once",
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
                logical_size: record.cleanup_logical_size(),
                is_directory: record.is_directory,
                is_reparse_point: record.is_reparse_point,
                caveats: entry_caveats,
            };
            for candidate in &entry.path_candidates {
                let parent_record_id = candidate.parent_reference.record_id;
                if parent_record_id == child_record_id {
                    continue;
                }
                if parent_sequence_mismatches(&references, candidate.parent_reference) {
                    child_edge_caveats
                        .entry(parent_record_id)
                        .or_default()
                        .push(ParseCaveat::new(
                            "parent-sequence-mismatch",
                            format!(
                                "record {} references parent {} with stale sequence {:?}",
                                child_record_id,
                                parent_record_id,
                                candidate.parent_reference.sequence_number
                            ),
                        ));
                    continue;
                }
                push_child(&mut children, parent_record_id, child_record_id);
            }
            entries.insert(child_record_id, entry);
        }
        cross_check_directory_entries(
            &mut entries,
            &mut children,
            &directory_entries_by_parent,
            &mut child_edge_caveats,
        );

        Self {
            entries,
            children,
            child_edge_caveats,
            caveats,
        }
    }

    pub fn get(&self, record_id: u64) -> Option<&MftIndexEntry> {
        self.entries.get(&record_id)
    }

    pub fn find_child(&self, parent_record_id: u64, name: &str) -> Option<&MftIndexEntry> {
        self.children
            .get(&parent_record_id)?
            .iter()
            .filter_map(|record_id| self.entries.get(record_id))
            .find(|entry| {
                entry.path_candidates.iter().any(|candidate| {
                    candidate.parent_reference.record_id == parent_record_id
                        && candidate.name.eq_ignore_ascii_case(name)
                })
            })
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

    pub fn aggregate_subtree(&self, root_record_id: u64) -> SubtreeSummary {
        let mut summary = SubtreeSummary::default();
        let mut stack = vec![root_record_id];
        let mut visited = BTreeSet::new();

        while let Some(record_id) = stack.pop() {
            if visited.contains(&record_id) {
                if self
                    .entries
                    .get(&record_id)
                    .is_some_and(|entry| entry.is_directory)
                {
                    summary.caveats.push(ParseCaveat::new(
                        "mft-index-cycle-skipped",
                        format!(
                            "directory record {record_id} appeared more than once in a subtree"
                        ),
                    ));
                }
                continue;
            }
            visited.insert(record_id);

            let Some(entry) = self.entries.get(&record_id) else {
                summary.caveats.push(ParseCaveat::new(
                    "missing-record",
                    format!("record {record_id} is not present in the MFT index"),
                ));
                continue;
            };
            summary.caveats.extend(entry.caveats.clone());

            if entry.is_reparse_point {
                summary.caveats.push(ParseCaveat::new(
                    "reparse-point-skipped",
                    format!("record {record_id} is a reparse point"),
                ));
                continue;
            }

            if entry.is_directory {
                summary.directories = summary.directories.saturating_add(1);
                if let Some(caveats) = self.child_edge_caveats.get(&record_id) {
                    summary.caveats.extend(caveats.clone());
                }
                if let Some(child_ids) = self.children.get(&record_id) {
                    stack.extend(child_ids.iter().copied());
                }
            } else {
                summary.files = summary.files.saturating_add(1);
                summary.bytes = summary.bytes.saturating_add(entry.logical_size);
            }
        }

        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftIndexEntry {
    pub reference: NtfsFileReference,
    pub parent_reference: NtfsFileReference,
    pub name: String,
    pub path_candidates: Vec<MftPathCandidate>,
    pub logical_size: u64,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtreeSummary {
    pub bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub caveats: Vec<ParseCaveat>,
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

fn push_child(children: &mut BTreeMap<u64, Vec<u64>>, parent_id: u64, child_id: u64) {
    let child_ids = children.entry(parent_id).or_default();
    if !child_ids.contains(&child_id) {
        child_ids.push(child_id);
    }
}

fn cross_check_directory_entries(
    entries: &mut BTreeMap<u64, MftIndexEntry>,
    children: &mut BTreeMap<u64, Vec<u64>>,
    directory_entries_by_parent: &BTreeMap<u64, Vec<crate::NtfsDirectoryEntry>>,
    child_edge_caveats: &mut BTreeMap<u64, Vec<ParseCaveat>>,
) {
    for (parent_record_id, directory_entries) in directory_entries_by_parent {
        for directory_entry in directory_entries {
            if directory_entry.parent.record_id != *parent_record_id {
                push_directory_caveat(
                    child_edge_caveats,
                    *parent_record_id,
                    ParseCaveat::new(
                        "directory-index-parent-mismatch",
                        format!(
                            "$I30 entry '{}' declares parent {}, but was stored on directory {}",
                            directory_entry.name,
                            directory_entry.parent.record_id,
                            parent_record_id
                        ),
                    ),
                );
                continue;
            }

            let Some(child_entry) = entries.get(&directory_entry.child.record_id) else {
                push_directory_caveat(
                    child_edge_caveats,
                    *parent_record_id,
                    ParseCaveat::new(
                        "directory-index-child-missing-record",
                        format!(
                            "$I30 entry '{}' references missing record {}",
                            directory_entry.name, directory_entry.child.record_id
                        ),
                    ),
                );
                continue;
            };

            if matches!(
                (
                    directory_entry.child.sequence_number,
                    child_entry.reference.sequence_number
                ),
                (Some(expected), Some(actual)) if sequence_number_mismatches(expected, actual)
            ) {
                push_directory_caveat(
                    child_edge_caveats,
                    *parent_record_id,
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
                );
                continue;
            }

            let parent_edge_exists = children
                .get(parent_record_id)
                .is_some_and(|ids| ids.contains(&directory_entry.child.record_id));
            if !parent_edge_exists {
                let caveat = ParseCaveat::new(
                    "directory-index-parent-map-fallback",
                    format!(
                        "$I30 entry '{}' was used because it is not present in $FILE_NAME parent edges for directory {}",
                        directory_entry.name, parent_record_id
                    ),
                );
                push_directory_caveat(child_edge_caveats, *parent_record_id, caveat.clone());
                if let Some(child_entry) = entries.get_mut(&directory_entry.child.record_id) {
                    child_entry.caveats.push(caveat);
                }
                children
                    .entry(*parent_record_id)
                    .or_default()
                    .push(directory_entry.child.record_id);
            }
        }
    }
}

fn sequence_number_mismatches(expected: u16, actual: u16) -> bool {
    expected != 0 && actual != 0 && expected != actual
}

fn push_directory_caveat(
    child_edge_caveats: &mut BTreeMap<u64, Vec<ParseCaveat>>,
    parent_record_id: u64,
    caveat: ParseCaveat,
) {
    child_edge_caveats
        .entry(parent_record_id)
        .or_default()
        .push(caveat);
}
