use serde::{Deserialize, Serialize};

use crate::adapter::{NtfsDataStream, NtfsFileName, NtfsFileReference, NtfsParsedRecord};
use crate::attrs::{AttributeHeader, AttributeType};
use crate::fixup::apply_update_sequence;
use crate::parse::{
    file_reference_sequence_number, low_file_reference_id, read_u16, read_u32, read_u64,
};
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
    const fn from_raw(value: u8) -> Self {
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
        in_use: (record_flags & RECORD_FLAG_IN_USE) != 0,
        is_directory: (record_flags & RECORD_FLAG_DIRECTORY) != 0,
        is_reparse_point: false,
        names: Vec::new(),
        data_streams: Vec::new(),
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
        AttributeType::AttributeList => parsed.caveats.push(ParseCaveat::new(
            "attribute-list-present",
            "record uses an attribute list; external attributes are not expanded by the parser",
        )),
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
            if header.is_named() {
                parsed.caveats.push(ParseCaveat::new(
                    "named-data-stream",
                    "named data streams are not counted in cleanup size estimates",
                ));
                return Ok(());
            }

            let size = header
                .non_resident_data_size
                .or_else(|| {
                    header
                        .resident_value_range
                        .as_ref()
                        .map(|range| range.len() as u64)
                })
                .unwrap_or(0);
            push_unnamed_data_stream(parsed, size);
        }
        AttributeType::ReparsePoint => {
            parsed.is_reparse_point = true;
        }
        AttributeType::Other(_) => {}
    }

    Ok(())
}

fn parse_file_name(value: &[u8]) -> Result<NtfsFileName> {
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

    let mut utf16 = Vec::with_capacity(name_len);
    for chunk in value[66..66 + name_bytes].chunks_exact(2) {
        utf16.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }

    let parent_reference = read_u64(value, 0)?;
    Ok(NtfsFileName {
        parent: NtfsFileReference::known(
            low_file_reference_id(parent_reference),
            file_reference_sequence_number(parent_reference),
        ),
        namespace: FileNameNamespace::from_raw(value[65]),
        name: String::from_utf16_lossy(&utf16),
        allocated_size: read_u64(value, 40)?,
        real_size: read_u64(value, 48)?,
        file_attributes: read_u32(value, 56)?,
    })
}

fn push_unnamed_data_stream(record: &mut NtfsParsedRecord, logical_size: u64) {
    if let Some(stream) = record
        .data_streams
        .iter_mut()
        .find(|stream| stream.name.is_none())
    {
        stream.logical_size = stream.logical_size.max(logical_size);
        return;
    }

    record.data_streams.push(NtfsDataStream {
        name: None,
        logical_size,
        allocated_size: None,
        initialized_size: None,
    });
}
