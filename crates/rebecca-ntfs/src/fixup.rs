use crate::parse::read_u16;
use crate::{NtfsParseError, Result};

pub fn apply_update_sequence(record: &[u8], sector_size: usize) -> Result<Vec<u8>> {
    if sector_size < 2 || record.len() < sector_size || !record.len().is_multiple_of(sector_size) {
        return Err(NtfsParseError::InvalidUpdateSequence);
    }

    let usa_offset = usize::from(read_u16(record, 4)?);
    let usa_count = usize::from(read_u16(record, 6)?);
    let sector_count = record.len() / sector_size;
    if usa_count != sector_count.saturating_add(1) {
        return Err(NtfsParseError::InvalidUpdateSequence);
    }

    let usa_len = usa_count
        .checked_mul(2)
        .ok_or(NtfsParseError::InvalidUpdateSequence)?;
    if usa_offset
        .checked_add(usa_len)
        .is_none_or(|end| end > record.len())
    {
        return Err(NtfsParseError::InvalidUpdateSequence);
    }

    let update_sequence = read_u16(record, usa_offset)?;
    let mut fixed = record.to_vec();
    let mut raw_fixups = 0_usize;
    let mut applied_fixups = 0_usize;
    for sector_index in 0..sector_count {
        let sector_tail = (sector_index + 1)
            .checked_mul(sector_size)
            .and_then(|end| end.checked_sub(2))
            .ok_or(NtfsParseError::InvalidUpdateSequence)?;
        let observed = read_u16(record, sector_tail)?;
        let replacement = read_u16(record, usa_offset + ((sector_index + 1) * 2))?;
        if observed == update_sequence {
            raw_fixups = raw_fixups.saturating_add(1);
            fixed[sector_tail..sector_tail + 2].copy_from_slice(&replacement.to_le_bytes());
        } else if observed == replacement {
            applied_fixups = applied_fixups.saturating_add(1);
        } else {
            return Err(NtfsParseError::InvalidUpdateSequence);
        }
    }

    if raw_fixups > 0 && applied_fixups > 0 {
        return Err(NtfsParseError::InvalidUpdateSequence);
    }

    Ok(fixed)
}
