use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adapter::{NtfsFileReference, NtfsParsedRecord};
use crate::record::ParseCaveat;
use crate::record_set::NtfsRecordSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftIndex {
    entries: BTreeMap<u64, MftIndexEntry>,
    children: BTreeMap<u64, Vec<u64>>,
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
        let mut entries = BTreeMap::new();
        let mut children: BTreeMap<u64, Vec<u64>> = BTreeMap::new();

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
            if record.non_dos_file_name_count() > 1 {
                entry_caveats.push(ParseCaveat::new(
                    "multiple-file-names",
                    format!(
                        "record {} has multiple non-DOS file names; hardlink accounting may be ambiguous",
                        record.reference.record_id
                    ),
                ));
            }

            caveats.extend(entry_caveats.clone());
            let child_record_id = record.reference.record_id;
            let parent_record_id = file_name.parent.record_id;
            let entry = MftIndexEntry {
                reference: record.reference,
                parent_reference: file_name.parent,
                name: file_name.name.clone(),
                logical_size: record.cleanup_logical_size(),
                is_directory: record.is_directory,
                is_reparse_point: record.is_reparse_point,
                caveats: entry_caveats,
            };
            if parent_record_id != child_record_id {
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
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
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
            if !visited.insert(record_id) {
                summary.caveats.push(ParseCaveat::new(
                    "mft-index-cycle-skipped",
                    format!("record {record_id} appeared more than once in a subtree"),
                ));
                continue;
            }

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
    pub logical_size: u64,
    pub is_directory: bool,
    pub is_reparse_point: bool,
    pub caveats: Vec<ParseCaveat>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtreeSummary {
    pub bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub caveats: Vec<ParseCaveat>,
}
