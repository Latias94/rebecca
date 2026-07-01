use crate::{NtfsParseError, Result};

pub(crate) fn read_u8(data: &[u8], offset: usize) -> Result<u8> {
    data.get(offset).copied().ok_or(NtfsParseError::Truncated {
        expected: offset.saturating_add(1),
        actual: data.len(),
    })
}

pub(crate) fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = slice(data, offset, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub(crate) fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = slice(data, offset, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub(crate) fn read_u64(data: &[u8], offset: usize) -> Result<u64> {
    let bytes = slice(data, offset, 8)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

pub(crate) fn slice(data: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    let end = offset.checked_add(len).ok_or(NtfsParseError::Truncated {
        expected: usize::MAX,
        actual: data.len(),
    })?;
    data.get(offset..end).ok_or(NtfsParseError::Truncated {
        expected: end,
        actual: data.len(),
    })
}

pub(crate) fn low_file_reference_id(reference: u64) -> u64 {
    reference & 0x0000_FFFF_FFFF_FFFF
}
