use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adapter::{NtfsFileReference, NtfsParsedRecord};
use crate::record::{FileNameNamespace, ParseCaveat};
use crate::record_set::NtfsRecordSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftIndex {
    entries: BTreeMap<u64, MftIndexEntry>,
    children: BTreeMap<u64, Vec<u64>>,
    skipped_child_caveats: BTreeMap<u64, Vec<ParseCaveat>>,
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
        let mut entries = BTreeMap::new();
        let mut children: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        let mut skipped_child_caveats: BTreeMap<u64, Vec<ParseCaveat>> = BTreeMap::new();

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
                    skipped_child_caveats
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
                children
                    .entry(parent_record_id)
                    .or_default()
                    .push(child_record_id);
            }
            entries.insert(child_record_id, entry);
        }

        Self {
            entries,
            children,
            skipped_child_caveats,
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
                if let Some(caveats) = self.skipped_child_caveats.get(&record_id) {
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
    let Some(actual_parent) = references.get(&parent_reference.record_id) else {
        return false;
    };
    matches!(
        (parent_reference.sequence_number, actual_parent.sequence_number),
        (Some(expected), Some(actual)) if expected != 0 && actual != 0 && expected != actual
    )
}
