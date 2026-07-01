use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::record::{MftRecord, ParseCaveat};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftTree {
    entries: BTreeMap<u64, MftTreeEntry>,
    children: BTreeMap<u64, Vec<u64>>,
    pub caveats: Vec<ParseCaveat>,
}

impl MftTree {
    pub fn from_records(records: impl IntoIterator<Item = MftRecord>) -> Self {
        let mut entries = BTreeMap::new();
        let mut children: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        let mut caveats = Vec::new();

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
            if non_dos_file_name_count(&record) > 1 {
                entry_caveats.push(ParseCaveat::new(
                    "multiple-file-names",
                    format!(
                        "record {} has multiple non-DOS file names; hardlink accounting may be ambiguous",
                        record.record_id
                    ),
                ));
            }

            caveats.extend(entry_caveats.clone());
            let entry = MftTreeEntry {
                record_id: record.record_id,
                parent_record_id: file_name.parent_record_id,
                name: file_name.name.clone(),
                logical_size: if record.is_directory {
                    0
                } else {
                    record.data_size
                },
                is_directory: record.is_directory,
                is_reparse_point: record.is_reparse_point,
                caveats: entry_caveats,
            };
            if entry.parent_record_id != entry.record_id {
                children
                    .entry(entry.parent_record_id)
                    .or_default()
                    .push(entry.record_id);
            }
            entries.insert(entry.record_id, entry);
        }

        Self {
            entries,
            children,
            caveats,
        }
    }

    pub fn get(&self, record_id: u64) -> Option<&MftTreeEntry> {
        self.entries.get(&record_id)
    }

    pub fn find_child(&self, parent_record_id: u64, name: &str) -> Option<&MftTreeEntry> {
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
    ) -> Option<&MftTreeEntry> {
        let mut current = self.entries.get(&root_record_id)?;
        for component in components {
            current = self.find_child(current.record_id, component)?;
        }
        Some(current)
    }

    pub fn entries(&self) -> impl Iterator<Item = &MftTreeEntry> {
        self.entries.values()
    }

    pub fn aggregate_subtree(&self, root_record_id: u64) -> SubtreeSummary {
        let mut summary = SubtreeSummary::default();
        let mut stack = vec![root_record_id];
        let mut visited = BTreeSet::new();

        while let Some(record_id) = stack.pop() {
            if !visited.insert(record_id) {
                summary.caveats.push(ParseCaveat::new(
                    "tree-cycle-skipped",
                    format!("record {record_id} appeared more than once in a subtree"),
                ));
                continue;
            }

            let Some(entry) = self.entries.get(&record_id) else {
                summary.caveats.push(ParseCaveat::new(
                    "missing-record",
                    format!("record {record_id} is not present in the tree"),
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

fn non_dos_file_name_count(record: &MftRecord) -> usize {
    record
        .file_names
        .iter()
        .filter(|name| !matches!(name.namespace, crate::record::FileNameNamespace::Dos))
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftTreeEntry {
    pub record_id: u64,
    pub parent_record_id: u64,
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
