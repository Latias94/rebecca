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
    pub in_use: bool,
    pub is_directory: bool,
    pub is_reparse_point: bool,
    pub attributes: Vec<NtfsParsedAttribute>,
    pub names: Vec<NtfsFileName>,
    pub data_streams: Vec<NtfsDataStream>,
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

        self.data_streams
            .iter()
            .filter(|stream| stream.name.is_none())
            .map(|stream| stream.logical_size)
            .max()
            .unwrap_or(0)
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
pub struct NtfsFileName {
    pub parent: NtfsFileReference,
    pub namespace: FileNameNamespace,
    pub name: String,
    pub allocated_size: u64,
    pub real_size: u64,
    pub file_attributes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsDataStream {
    pub attribute_id: u16,
    pub name: Option<String>,
    pub lowest_vcn: Option<u64>,
    pub highest_vcn: Option<u64>,
    pub logical_size: u64,
    pub allocated_size: Option<u64>,
    pub initialized_size: Option<u64>,
    pub data_runs: Vec<NtfsDataRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsDataRun {
    pub starting_vcn: u64,
    pub cluster_count: u64,
    pub lcn: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsDirectoryEntry {
    pub child: NtfsFileReference,
    pub parent: NtfsFileReference,
    pub namespace: FileNameNamespace,
    pub name: String,
    pub file_attributes: u32,
}
