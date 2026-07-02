use serde::{Deserialize, Serialize};
use std::ops::Range;

use crate::parse::{read_u8, read_u16, read_u32, read_u64, slice};
use crate::{NtfsParseError, Result};

pub const ATTRIBUTE_END: u32 = 0xFFFF_FFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttributeType {
    StandardInformation,
    AttributeList,
    FileName,
    Data,
    IndexRoot,
    IndexAllocation,
    ReparsePoint,
    Other(u32),
}

impl AttributeType {
    pub const fn from_code(code: u32) -> Self {
        match code {
            0x10 => Self::StandardInformation,
            0x20 => Self::AttributeList,
            0x30 => Self::FileName,
            0x80 => Self::Data,
            0x90 => Self::IndexRoot,
            0xA0 => Self::IndexAllocation,
            0xC0 => Self::ReparsePoint,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeHeader {
    pub attribute_type: AttributeType,
    pub offset: usize,
    pub length: usize,
    pub non_resident: bool,
    pub name_length: u8,
    pub name_offset: u16,
    pub flags: u16,
    pub attribute_id: u16,
    pub resident_value_range: Option<Range<usize>>,
    pub lowest_vcn: Option<u64>,
    pub highest_vcn: Option<u64>,
    pub non_resident_runlist_range: Option<Range<usize>>,
    pub non_resident_allocated_size: Option<u64>,
    pub non_resident_logical_size: Option<u64>,
    pub non_resident_initialized_size: Option<u64>,
}

impl AttributeHeader {
    pub fn parse(record: &[u8], offset: usize) -> Result<Option<Self>> {
        if offset.checked_add(4).is_none_or(|end| end > record.len()) {
            return Err(NtfsParseError::TruncatedAttribute { offset });
        }

        let type_code = read_u32(record, offset)?;
        if type_code == ATTRIBUTE_END {
            return Ok(None);
        }

        if offset.checked_add(16).is_none_or(|end| end > record.len()) {
            return Err(NtfsParseError::TruncatedAttribute { offset });
        }

        let length = read_u32(record, offset + 4)? as usize;
        if length < 16 {
            return Err(NtfsParseError::InvalidAttribute { offset });
        }
        let attr_end = offset
            .checked_add(length)
            .ok_or(NtfsParseError::TruncatedAttribute { offset })?;
        if attr_end > record.len() {
            return Err(NtfsParseError::TruncatedAttribute { offset });
        }

        let non_resident = read_u8(record, offset + 8)? != 0;
        let name_length = read_u8(record, offset + 9)?;
        let name_offset = read_u16(record, offset + 10)?;
        let flags = read_u16(record, offset + 12)?;
        let attribute_id = read_u16(record, offset + 14)?;
        let (
            resident_value_range,
            lowest_vcn,
            highest_vcn,
            non_resident_runlist_range,
            non_resident_allocated_size,
            non_resident_logical_size,
            non_resident_initialized_size,
        ) = if non_resident {
            if length < 64 {
                return Err(NtfsParseError::InvalidAttribute { offset });
            }
            let runlist_offset = usize::from(read_u16(record, offset + 32)?);
            let runlist_start = offset
                .checked_add(runlist_offset)
                .ok_or(NtfsParseError::InvalidAttribute { offset })?;
            if runlist_offset < 64 || runlist_start > attr_end {
                return Err(NtfsParseError::InvalidAttribute { offset });
            }
            (
                None,
                Some(read_u64(record, offset + 16)?),
                Some(read_u64(record, offset + 24)?),
                Some(runlist_start..attr_end),
                Some(read_u64(record, offset + 40)?),
                Some(read_u64(record, offset + 48)?),
                Some(read_u64(record, offset + 56)?),
            )
        } else {
            if length < 24 {
                return Err(NtfsParseError::InvalidAttribute { offset });
            }
            let value_length = read_u32(record, offset + 16)? as usize;
            let value_offset = usize::from(read_u16(record, offset + 20)?);
            let value_start = offset
                .checked_add(value_offset)
                .ok_or(NtfsParseError::TruncatedResidentValue { offset })?;
            let value_end = value_start
                .checked_add(value_length)
                .ok_or(NtfsParseError::TruncatedResidentValue { offset })?;
            if value_end > attr_end {
                return Err(NtfsParseError::TruncatedResidentValue { offset });
            }
            (
                Some(value_start..value_end),
                None,
                None,
                None,
                None,
                None,
                None,
            )
        };

        Ok(Some(Self {
            attribute_type: AttributeType::from_code(type_code),
            offset,
            length,
            non_resident,
            name_length,
            name_offset,
            flags,
            attribute_id,
            resident_value_range,
            lowest_vcn,
            highest_vcn,
            non_resident_runlist_range,
            non_resident_allocated_size,
            non_resident_logical_size,
            non_resident_initialized_size,
        }))
    }

    pub fn name<'a>(&self, record: &'a [u8]) -> Result<Option<&'a [u8]>> {
        if self.name_length == 0 {
            return Ok(None);
        }

        let name_offset = self.offset + usize::from(self.name_offset);
        let name_len = usize::from(self.name_length) * 2;
        Ok(Some(slice(record, name_offset, name_len)?))
    }

    pub fn is_named(&self) -> bool {
        self.name_length != 0
    }

    pub fn resident_value<'a>(&self, record: &'a [u8]) -> Option<&'a [u8]> {
        self.resident_value_range
            .as_ref()
            .and_then(|range| record.get(range.clone()))
    }

    pub fn name_string(&self, record: &[u8]) -> Result<Option<String>> {
        let Some(name) = self.name(record)? else {
            return Ok(None);
        };
        let utf16 = name
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        Ok(Some(String::from_utf16_lossy(&utf16)))
    }

    pub fn non_resident_runlist<'a>(&self, record: &'a [u8]) -> Option<&'a [u8]> {
        self.non_resident_runlist_range
            .as_ref()
            .and_then(|range| record.get(range.clone()))
    }
}
