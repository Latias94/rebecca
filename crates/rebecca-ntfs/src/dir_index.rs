use crate::adapter::{NtfsDirectoryEntry, NtfsFileReference};
use crate::attrs::AttributeType;
use crate::fixup::apply_update_sequence;
use crate::parse::{
    file_reference_sequence_number, low_file_reference_id, read_u16, read_u32, read_u64,
};
use crate::record::parse_file_name;
use crate::{NtfsParseError, Result};

const INDEX_ROOT_HEADER_LEN: usize = 16;
const INDEX_HEADER_LEN: usize = 16;
const INDEX_ALLOCATION_HEADER_LEN: usize = 24;
const INDEX_ENTRY_HEADER_LEN: usize = 16;
const INDEX_ENTRY_FLAG_NODE: u16 = 0x0001;
const INDEX_ENTRY_FLAG_LAST: u16 = 0x0002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtfsIndexRoot {
    pub indexed_attribute: AttributeType,
    pub index_record_size: u32,
    pub entries: Vec<NtfsDirectoryEntry>,
}

pub fn parse_i30_index_root(value: &[u8]) -> Result<NtfsIndexRoot> {
    if value.len() < INDEX_ROOT_HEADER_LEN + INDEX_HEADER_LEN {
        return Err(NtfsParseError::InvalidDirectoryIndex);
    }
    let indexed_attribute = AttributeType::from_code(read_u32(value, 0)?);
    let index_record_size = read_u32(value, 8)?;
    if indexed_attribute != AttributeType::FileName {
        return Ok(NtfsIndexRoot {
            indexed_attribute,
            index_record_size,
            entries: Vec::new(),
        });
    }

    let index_header_offset = INDEX_ROOT_HEADER_LEN;
    let entry_offset = read_u32(value, index_header_offset)? as usize;
    let total_size = read_u32(value, index_header_offset + 4)? as usize;
    let entries_start = index_header_offset
        .checked_add(entry_offset)
        .ok_or(NtfsParseError::InvalidDirectoryIndex)?;
    let entries_end = index_header_offset
        .checked_add(total_size)
        .ok_or(NtfsParseError::InvalidDirectoryIndex)?;
    if entries_start > entries_end || entries_end > value.len() {
        return Err(NtfsParseError::InvalidDirectoryIndex);
    }

    Ok(NtfsIndexRoot {
        indexed_attribute,
        index_record_size,
        entries: parse_i30_entries(value, entries_start, entries_end)?,
    })
}

pub fn parse_i30_index_allocation_record(
    raw_record: &[u8],
    sector_size: usize,
    expected_vcn: u64,
) -> Result<Vec<NtfsDirectoryEntry>> {
    if raw_record.len() < INDEX_ALLOCATION_HEADER_LEN + INDEX_HEADER_LEN {
        return Err(NtfsParseError::InvalidDirectoryIndex);
    }
    let record = apply_update_sequence(raw_record, sector_size)?;
    if record.get(0..4) != Some(b"INDX") {
        return Err(NtfsParseError::InvalidDirectoryIndex);
    }
    if read_u64(&record, 16)? != expected_vcn {
        return Err(NtfsParseError::InvalidDirectoryIndex);
    }

    let index_header_offset = INDEX_ALLOCATION_HEADER_LEN;
    let entry_offset = read_u32(&record, index_header_offset)? as usize;
    let total_size = read_u32(&record, index_header_offset + 4)? as usize;
    let allocated_size = read_u32(&record, index_header_offset + 8)? as usize;
    if total_size > allocated_size {
        return Err(NtfsParseError::InvalidDirectoryIndex);
    }

    let entries_start = index_header_offset
        .checked_add(entry_offset)
        .ok_or(NtfsParseError::InvalidDirectoryIndex)?;
    let entries_end = index_header_offset
        .checked_add(total_size)
        .ok_or(NtfsParseError::InvalidDirectoryIndex)?;
    let allocated_end = index_header_offset
        .checked_add(allocated_size)
        .ok_or(NtfsParseError::InvalidDirectoryIndex)?;
    if entries_start > entries_end || entries_end > allocated_end || allocated_end > record.len() {
        return Err(NtfsParseError::InvalidDirectoryIndex);
    }

    parse_i30_entries(&record, entries_start, entries_end)
}

fn parse_i30_entries(
    value: &[u8],
    entries_start: usize,
    entries_end: usize,
) -> Result<Vec<NtfsDirectoryEntry>> {
    let mut entries = Vec::new();
    let mut offset = entries_start;
    while offset < entries_end {
        if offset
            .checked_add(INDEX_ENTRY_HEADER_LEN)
            .is_none_or(|end| end > entries_end)
        {
            return Err(NtfsParseError::InvalidDirectoryIndex);
        }

        let raw_child_reference = read_u64(value, offset)?;
        let entry_len = usize::from(read_u16(value, offset + 8)?);
        let value_len = usize::from(read_u16(value, offset + 10)?);
        let flags = read_u16(value, offset + 12)?;
        if entry_len < INDEX_ENTRY_HEADER_LEN
            || offset
                .checked_add(entry_len)
                .is_none_or(|end| end > entries_end)
        {
            return Err(NtfsParseError::InvalidDirectoryIndex);
        }
        if (flags & INDEX_ENTRY_FLAG_LAST) != 0 {
            break;
        }

        let value_limit = if (flags & INDEX_ENTRY_FLAG_NODE) != 0 {
            entry_len
                .checked_sub(8)
                .ok_or(NtfsParseError::InvalidDirectoryIndex)?
        } else {
            entry_len
        };
        let file_name_start = offset + INDEX_ENTRY_HEADER_LEN;
        let file_name_end = file_name_start
            .checked_add(value_len)
            .ok_or(NtfsParseError::InvalidDirectoryIndex)?;
        if file_name_end > offset + value_limit {
            return Err(NtfsParseError::InvalidDirectoryIndex);
        }
        entries.push(parse_index_file_name(
            raw_child_reference,
            &value[file_name_start..file_name_end],
        )?);
        offset += entry_len;
    }

    Ok(entries)
}

fn parse_index_file_name(raw_child_reference: u64, value: &[u8]) -> Result<NtfsDirectoryEntry> {
    let file_name = parse_file_name(value).map_err(|_| NtfsParseError::InvalidDirectoryIndex)?;
    Ok(NtfsDirectoryEntry {
        child: parse_file_reference(raw_child_reference),
        parent: file_name.parent,
        namespace: file_name.namespace,
        name: file_name.name,
        file_attributes: file_name.file_attributes,
    })
}

fn parse_file_reference(raw_reference: u64) -> NtfsFileReference {
    NtfsFileReference::known(
        low_file_reference_id(raw_reference),
        file_reference_sequence_number(raw_reference),
    )
}
