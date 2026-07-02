use serde::{Deserialize, Serialize};

use crate::adapter::{
    NtfsAttributeStream, NtfsDirectoryIndex, NtfsFileName, NtfsFileReference, NtfsParsedAttribute,
    NtfsParsedRecord, merge_attribute_stream,
};
use crate::attribute_list::parse_attribute_list;
use crate::attrs::{AttributeHeader, AttributeType};
use crate::dir_index::parse_i30_index_root;
use crate::fixup::apply_update_sequence;
use crate::parse::{
    file_reference_sequence_number, low_file_reference_id, read_u16, read_u32, read_u64,
    utf16_lossy,
};
use crate::runlist::parse_data_runs;
use crate::{NtfsParseError, Result};

const RECORD_HEADER_MIN_LEN: usize = 48;
const FILE_NAME_MIN_LEN: usize = 66;
const RECORD_FLAG_IN_USE: u16 = 0x0001;
const RECORD_FLAG_DIRECTORY: u16 = 0x0002;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

impl NtfsParsedRecord {
    pub fn parse(record_id: u64, raw_record: &[u8], sector_size: usize) -> Result<Self> {
        parse_record(record_id, raw_record, sector_size)
    }

    pub fn parse_mft_record(record_id: u64, raw_record: &[u8], sector_size: usize) -> Result<Self> {
        Self::parse(record_id, raw_record, sector_size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileNameNamespace {
    Posix,
    Win32,
    Dos,
    Win32AndDos,
    Unknown(u8),
}

impl FileNameNamespace {
    pub(crate) const fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Posix,
            1 => Self::Win32,
            2 => Self::Dos,
            3 => Self::Win32AndDos,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseCaveat {
    pub code: String,
    pub message: String,
}

impl ParseCaveat {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

fn parse_record(record_id: u64, raw_record: &[u8], sector_size: usize) -> Result<NtfsParsedRecord> {
    if raw_record.len() < RECORD_HEADER_MIN_LEN {
        return Err(NtfsParseError::Truncated {
            expected: RECORD_HEADER_MIN_LEN,
            actual: raw_record.len(),
        });
    }

    let record = apply_update_sequence(raw_record, sector_size)?;
    if record.get(0..4) != Some(b"FILE") {
        return Err(NtfsParseError::InvalidSignature);
    }

    let sequence_number = read_u16(&record, 16)?;
    let base_reference = parse_optional_file_reference(read_u64(&record, 32)?);
    let first_attribute_offset = usize::from(read_u16(&record, 20)?);
    let record_flags = read_u16(&record, 22)?;
    let used_size = read_u32(&record, 24)? as usize;
    let attr_limit = if used_size >= first_attribute_offset && used_size <= record.len() {
        used_size
    } else {
        record.len()
    };

    let mut parsed = NtfsParsedRecord {
        reference: NtfsFileReference::known(record_id, sequence_number),
        base_reference,
        in_use: (record_flags & RECORD_FLAG_IN_USE) != 0,
        is_directory: (record_flags & RECORD_FLAG_DIRECTORY) != 0,
        is_reparse_point: false,
        attributes: Vec::new(),
        attribute_list_entries: Vec::new(),
        names: Vec::new(),
        attribute_streams: Vec::new(),
        directory_indexes: Vec::new(),
        directory_entries: Vec::new(),
        caveats: Vec::new(),
    };

    let mut offset = first_attribute_offset;
    while offset < attr_limit {
        let Some(header) = AttributeHeader::parse(&record, offset)? else {
            break;
        };

        parse_attribute(&record, &header, &mut parsed)?;
        offset = offset
            .checked_add(header.length)
            .ok_or(NtfsParseError::InvalidAttribute {
                offset: header.offset,
            })?;
    }

    if !parsed.in_use {
        parsed.caveats.push(ParseCaveat::new(
            "deleted-record",
            "record is not marked in use",
        ));
    }
    if parsed.in_use && parsed.names.is_empty() {
        parsed.caveats.push(ParseCaveat::new(
            "pathless-record",
            "record has no resident file-name attribute",
        ));
    }

    Ok(parsed)
}

fn parse_attribute(
    record: &[u8],
    header: &AttributeHeader,
    parsed: &mut NtfsParsedRecord,
) -> Result<()> {
    let attribute_name = attribute_name(record, header)?;
    parsed.attributes.push(NtfsParsedAttribute {
        attribute_type: header.attribute_type,
        attribute_id: header.attribute_id,
        name: attribute_name.clone(),
        non_resident: header.non_resident,
        lowest_vcn: header.lowest_vcn,
        highest_vcn: header.highest_vcn,
    });

    match header.attribute_type {
        AttributeType::StandardInformation => {
            if let Some(value) = header.resident_value(record)
                && value.len() >= 36
            {
                let flags = read_u32(value, 32)?;
                parsed.is_reparse_point |= (flags & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
                parsed.is_directory |= (flags & FILE_ATTRIBUTE_DIRECTORY) != 0;
            }
        }
        AttributeType::AttributeList => parse_attribute_list_attribute(record, header, parsed)?,
        AttributeType::FileName => {
            if header.non_resident {
                parsed.caveats.push(ParseCaveat::new(
                    "nonresident-file-name",
                    "nonresident file-name attributes are not expanded",
                ));
                return Ok(());
            }
            let Some(value) = header.resident_value(record) else {
                return Err(NtfsParseError::InvalidFileName);
            };
            let file_name = parse_file_name(value)?;
            parsed.is_reparse_point |=
                (file_name.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
            parsed.is_directory |= (file_name.file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
            parsed.names.push(file_name);
        }
        AttributeType::Data => {
            let stream = parse_attribute_stream(record, header, attribute_name.clone())?;
            let is_named = stream.name.is_some();
            merge_attribute_stream(&mut parsed.attribute_streams, stream);

            if is_named {
                parsed.caveats.push(ParseCaveat::new(
                    "named-data-stream",
                    "named data streams are not counted in cleanup size estimates",
                ));
            }
        }
        AttributeType::IndexRoot => {
            if attribute_name.as_deref() == Some("$I30") {
                let Some(value) = header.resident_value(record) else {
                    return Err(NtfsParseError::InvalidDirectoryIndex);
                };
                let index_root = parse_i30_index_root(value)?;
                parsed.directory_indexes.push(NtfsDirectoryIndex {
                    name: "$I30".to_string(),
                    attribute_id: header.attribute_id,
                    indexed_attribute: index_root.indexed_attribute,
                    index_record_size: index_root.index_record_size,
                });
                parsed.directory_entries.extend(index_root.entries);
            }
        }
        AttributeType::IndexAllocation => {
            if attribute_name.as_deref() == Some("$I30") {
                let stream = parse_attribute_stream(record, header, attribute_name.clone())?;
                merge_attribute_stream(&mut parsed.attribute_streams, stream);
            }
        }
        AttributeType::ReparsePoint => {
            parsed.is_reparse_point = true;
        }
        AttributeType::Other(_) => {}
    }

    Ok(())
}

fn attribute_name(record: &[u8], header: &AttributeHeader) -> Result<Option<String>> {
    header.name_string(record)
}

fn parse_attribute_list_attribute(
    record: &[u8],
    header: &AttributeHeader,
    parsed: &mut NtfsParsedRecord,
) -> Result<()> {
    parsed.caveats.push(ParseCaveat::new(
        "attribute-list-present",
        "record uses an attribute list; extension attributes require record-set resolution",
    ));

    if header.non_resident {
        let stream = parse_attribute_stream(record, header, None)?;
        merge_attribute_stream(&mut parsed.attribute_streams, stream);
        parsed.caveats.push(ParseCaveat::new(
            "nonresident-attribute-list",
            "nonresident attribute lists require runlist-backed record-set resolution",
        ));
        return Ok(());
    }

    let Some(value) = header.resident_value(record) else {
        return Err(NtfsParseError::InvalidAttributeList);
    };
    let entries = parse_attribute_list(value)?;
    if entries
        .iter()
        .any(|entry| entry.attribute_type == AttributeType::AttributeList)
    {
        parsed.caveats.push(ParseCaveat::new(
            "recursive-attribute-list-unsupported",
            "attribute list points at another attribute list; recursive expansion is refused",
        ));
    }
    parsed.attribute_list_entries.extend(entries);
    Ok(())
}

fn parse_attribute_stream(
    record: &[u8],
    header: &AttributeHeader,
    name: Option<String>,
) -> Result<NtfsAttributeStream> {
    if header.non_resident {
        let lowest_vcn = header.lowest_vcn.unwrap_or(0);
        let runlist = header
            .non_resident_runlist(record)
            .ok_or(NtfsParseError::InvalidRunlist)?;
        return Ok(NtfsAttributeStream {
            attribute_type: header.attribute_type,
            attribute_id: header.attribute_id,
            name,
            non_resident: true,
            flags: header.flags,
            lowest_vcn: header.lowest_vcn,
            highest_vcn: header.highest_vcn,
            logical_size: header.non_resident_logical_size.unwrap_or(0),
            allocated_size: header.non_resident_allocated_size,
            initialized_size: header.non_resident_initialized_size,
            data_runs: parse_data_runs(runlist, lowest_vcn)?,
        });
    }

    let logical_size = header
        .resident_value_range
        .as_ref()
        .and_then(|range| record.get(range.clone()))
        .map(|value| value.len() as u64)
        .unwrap_or(0);
    Ok(NtfsAttributeStream {
        attribute_type: header.attribute_type,
        attribute_id: header.attribute_id,
        name,
        non_resident: false,
        flags: header.flags,
        lowest_vcn: None,
        highest_vcn: None,
        logical_size,
        allocated_size: Some(logical_size),
        initialized_size: Some(logical_size),
        data_runs: Vec::new(),
    })
}

pub(crate) fn parse_file_name(value: &[u8]) -> Result<NtfsFileName> {
    if value.len() < FILE_NAME_MIN_LEN {
        return Err(NtfsParseError::InvalidFileName);
    }

    let name_len = usize::from(value[64]);
    let name_bytes = name_len
        .checked_mul(2)
        .ok_or(NtfsParseError::InvalidFileName)?;
    let expected_len = FILE_NAME_MIN_LEN
        .checked_add(name_bytes)
        .ok_or(NtfsParseError::InvalidFileName)?;
    if value.len() < expected_len {
        return Err(NtfsParseError::InvalidFileName);
    }

    let parent_reference = read_u64(value, 0)?;
    Ok(NtfsFileName {
        parent: NtfsFileReference::known(
            low_file_reference_id(parent_reference),
            file_reference_sequence_number(parent_reference),
        ),
        namespace: FileNameNamespace::from_raw(value[65]),
        name: utf16_lossy(&value[66..66 + name_bytes]),
        allocated_size: read_u64(value, 40)?,
        real_size: read_u64(value, 48)?,
        file_attributes: read_u32(value, 56)?,
    })
}

fn parse_optional_file_reference(raw_reference: u64) -> Option<NtfsFileReference> {
    if raw_reference == 0 {
        return None;
    }
    Some(NtfsFileReference::known(
        low_file_reference_id(raw_reference),
        file_reference_sequence_number(raw_reference),
    ))
}
