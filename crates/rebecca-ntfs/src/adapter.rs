use serde::{Deserialize, Serialize};

use crate::attrs::AttributeType;
use crate::record::{FileNameNamespace, ParseCaveat};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NtfsFileReference {
    pub record_id: u64,
    pub sequence_number: Option<u16>,
}

impl NtfsFileReference {
    pub const fn known(record_id: u64, sequence_number: u16) -> Self {
        Self {
            record_id,
            sequence_number: Some(sequence_number),
        }
    }

    pub const fn unknown_sequence(record_id: u64) -> Self {
        Self {
            record_id,
            sequence_number: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsParsedRecord {
    pub reference: NtfsFileReference,
    pub base_reference: Option<NtfsFileReference>,
    pub in_use: bool,
    pub is_directory: bool,
    pub is_reparse_point: bool,
    pub attributes: Vec<NtfsParsedAttribute>,
    pub attribute_list_entries: Vec<NtfsAttributeListEntry>,
    pub names: Vec<NtfsFileName>,
    pub attribute_streams: Vec<NtfsAttributeStream>,
    pub directory_indexes: Vec<NtfsDirectoryIndex>,
    pub directory_entries: Vec<NtfsDirectoryEntry>,
    pub caveats: Vec<ParseCaveat>,
}

impl NtfsParsedRecord {
    pub fn primary_file_name(&self) -> Option<&NtfsFileName> {
        self.names
            .iter()
            .find(|name| matches!(name.namespace, FileNameNamespace::Win32))
            .or_else(|| {
                self.names
                    .iter()
                    .find(|name| matches!(name.namespace, FileNameNamespace::Win32AndDos))
            })
            .or_else(|| {
                self.names
                    .iter()
                    .find(|name| !matches!(name.namespace, FileNameNamespace::Dos))
            })
            .or_else(|| self.names.first())
    }

    pub fn cleanup_logical_size(&self) -> u64 {
        if self.is_directory {
            return 0;
        }

        self.attribute_streams
            .iter()
            .filter(|stream| stream.attribute_type == AttributeType::Data && stream.name.is_none())
            .map(|stream| stream.logical_size)
            .max()
            .unwrap_or(0)
    }

    pub fn cleanup_allocated_size(&self) -> Option<u64> {
        if self.is_directory {
            return None;
        }

        self.attribute_streams
            .iter()
            .filter(|stream| stream.attribute_type == AttributeType::Data && stream.name.is_none())
            .filter_map(|stream| stream.allocated_size)
            .max()
    }

    pub fn non_dos_file_name_count(&self) -> usize {
        self.names
            .iter()
            .filter(|name| !matches!(name.namespace, FileNameNamespace::Dos))
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsParsedAttribute {
    pub attribute_type: AttributeType,
    pub attribute_id: u16,
    pub name: Option<String>,
    pub non_resident: bool,
    pub lowest_vcn: Option<u64>,
    pub highest_vcn: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsAttributeListEntry {
    pub attribute_type: AttributeType,
    pub name: Option<String>,
    pub lowest_vcn: u64,
    pub file_reference: NtfsFileReference,
    pub attribute_id: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsFileName {
    pub parent: NtfsFileReference,
    pub namespace: FileNameNamespace,
    pub name: String,
    pub allocated_size: u64,
    pub real_size: u64,
    pub file_attributes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsAttributeStream {
    pub attribute_type: AttributeType,
    pub attribute_id: u16,
    pub name: Option<String>,
    pub non_resident: bool,
    pub flags: u16,
    pub lowest_vcn: Option<u64>,
    pub highest_vcn: Option<u64>,
    pub logical_size: u64,
    pub allocated_size: Option<u64>,
    pub initialized_size: Option<u64>,
    pub data_runs: Vec<NtfsDataRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsDirectoryIndex {
    pub name: String,
    pub attribute_id: u16,
    pub indexed_attribute: AttributeType,
    pub index_record_size: u32,
    pub root_entries: Vec<NtfsIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsIndexEntry {
    pub directory_entry: Option<NtfsDirectoryEntry>,
    pub child_vcn: Option<u64>,
    pub is_last: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsDataRun {
    pub starting_vcn: u64,
    pub cluster_count: u64,
    pub lcn: Option<u64>,
}

pub(crate) fn merge_attribute_stream(
    streams: &mut Vec<NtfsAttributeStream>,
    mut incoming: NtfsAttributeStream,
) {
    if let Some(existing) = streams.iter_mut().find(|stream| {
        stream.attribute_type == incoming.attribute_type
            && stream.attribute_id == incoming.attribute_id
            && stream.name == incoming.name
    }) {
        existing.non_resident |= incoming.non_resident;
        existing.flags |= incoming.flags;
        existing.lowest_vcn = min_optional_vcn(existing.lowest_vcn, incoming.lowest_vcn);
        existing.logical_size = existing.logical_size.max(incoming.logical_size);
        existing.allocated_size = existing.allocated_size.max(incoming.allocated_size);
        existing.initialized_size = existing.initialized_size.max(incoming.initialized_size);
        existing.highest_vcn = existing.highest_vcn.max(incoming.highest_vcn);
        existing.data_runs.append(&mut incoming.data_runs);
        existing.data_runs.sort_by_key(|run| {
            (
                run.starting_vcn,
                run.lcn.unwrap_or(u64::MAX),
                run.cluster_count,
            )
        });
        existing.data_runs.dedup();
        return;
    }

    streams.push(incoming);
}

fn min_optional_vcn(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsDirectoryEntry {
    pub child: NtfsFileReference,
    pub parent: NtfsFileReference,
    pub namespace: FileNameNamespace,
    pub name: String,
    pub file_attributes: u32,
}
