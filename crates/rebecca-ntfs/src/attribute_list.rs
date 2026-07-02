use crate::adapter::{NtfsAttributeListEntry, NtfsFileReference};
use crate::attrs::AttributeType;
use crate::parse::{
    file_reference_sequence_number, low_file_reference_id, read_u16, read_u32, read_u64, slice,
    utf16_lossy,
};
use crate::{NtfsParseError, Result};

const ATTRIBUTE_LIST_ENTRY_MIN_LEN: usize = 26;

pub fn parse_attribute_list(bytes: &[u8]) -> Result<Vec<NtfsAttributeListEntry>> {
    let mut entries = Vec::new();
    let mut offset = 0;

    while offset < bytes.len() {
        if bytes[offset..].iter().all(|byte| *byte == 0) {
            break;
        }
        if offset
            .checked_add(ATTRIBUTE_LIST_ENTRY_MIN_LEN)
            .is_none_or(|end| end > bytes.len())
        {
            return Err(NtfsParseError::InvalidAttributeList);
        }

        let attribute_type = AttributeType::from_code(read_u32(bytes, offset)?);
        let entry_len = usize::from(read_u16(bytes, offset + 4)?);
        if entry_len < ATTRIBUTE_LIST_ENTRY_MIN_LEN
            || offset
                .checked_add(entry_len)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(NtfsParseError::InvalidAttributeList);
        }

        let name_length = usize::from(bytes[offset + 6]);
        let name_offset = usize::from(bytes[offset + 7]);
        let name = if name_length == 0 {
            None
        } else {
            let name_bytes = name_length
                .checked_mul(2)
                .ok_or(NtfsParseError::InvalidAttributeList)?;
            let name_start = offset
                .checked_add(name_offset)
                .ok_or(NtfsParseError::InvalidAttributeList)?;
            let name_end = name_start
                .checked_add(name_bytes)
                .ok_or(NtfsParseError::InvalidAttributeList)?;
            if name_offset < ATTRIBUTE_LIST_ENTRY_MIN_LEN || name_end > offset + entry_len {
                return Err(NtfsParseError::InvalidAttributeList);
            }
            Some(utf16_lossy(slice(bytes, name_start, name_bytes)?))
        };

        let raw_file_reference = read_u64(bytes, offset + 16)?;
        entries.push(NtfsAttributeListEntry {
            attribute_type,
            name,
            lowest_vcn: read_u64(bytes, offset + 8)?,
            file_reference: NtfsFileReference::known(
                low_file_reference_id(raw_file_reference),
                file_reference_sequence_number(raw_file_reference),
            ),
            attribute_id: read_u16(bytes, offset + 24)?,
        });

        offset += entry_len;
    }

    Ok(entries)
}
