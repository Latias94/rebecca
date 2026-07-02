use crate::adapter::NtfsDataRun;
use crate::{NtfsParseError, Result};

pub fn parse_data_runs(bytes: &[u8], starting_vcn: u64) -> Result<Vec<NtfsDataRun>> {
    let mut runs = Vec::new();
    let mut offset = 0;
    let mut current_vcn = starting_vcn;
    let mut current_lcn = 0_i128;

    while offset < bytes.len() {
        let header = bytes[offset];
        offset += 1;

        if header == 0 {
            return Ok(runs);
        }

        let count_len = usize::from(header & 0x0F);
        let lcn_delta_len = usize::from(header >> 4);
        if count_len == 0 || count_len > 8 || lcn_delta_len > 8 {
            return Err(NtfsParseError::InvalidRunlist);
        }

        let needed = count_len
            .checked_add(lcn_delta_len)
            .ok_or(NtfsParseError::InvalidRunlist)?;
        if offset
            .checked_add(needed)
            .is_none_or(|end| end > bytes.len())
        {
            return Err(NtfsParseError::InvalidRunlist);
        }

        let cluster_count = read_unsigned_le(&bytes[offset..offset + count_len]);
        offset += count_len;
        if cluster_count == 0 {
            return Err(NtfsParseError::InvalidRunlist);
        }

        let lcn = if lcn_delta_len == 0 {
            None
        } else {
            let delta = i128::from(read_signed_le(&bytes[offset..offset + lcn_delta_len]));
            offset += lcn_delta_len;
            current_lcn = current_lcn
                .checked_add(delta)
                .ok_or(NtfsParseError::InvalidRunlist)?;
            if current_lcn < 0 {
                return Err(NtfsParseError::InvalidRunlist);
            }
            Some(u64::try_from(current_lcn).map_err(|_| NtfsParseError::InvalidRunlist)?)
        };

        runs.push(NtfsDataRun {
            starting_vcn: current_vcn,
            cluster_count,
            lcn,
        });
        current_vcn = current_vcn
            .checked_add(cluster_count)
            .ok_or(NtfsParseError::InvalidRunlist)?;
    }

    Err(NtfsParseError::InvalidRunlist)
}

fn read_unsigned_le(bytes: &[u8]) -> u64 {
    let mut value = 0_u64;
    for (index, byte) in bytes.iter().copied().enumerate() {
        value |= u64::from(byte) << (index * 8);
    }
    value
}

fn read_signed_le(bytes: &[u8]) -> i64 {
    let mut padded = if bytes.last().is_some_and(|last| (last & 0x80) != 0) {
        [0xFF_u8; 8]
    } else {
        [0_u8; 8]
    };
    padded[..bytes.len()].copy_from_slice(bytes);
    i64::from_le_bytes(padded)
}

#[cfg(test)]
mod tests {
    use super::parse_data_runs;

    #[test]
    fn parses_data_sparse_and_negative_delta_runs() {
        let runs =
            parse_data_runs(&[0x11, 0x03, 0x0A, 0x01, 0x02, 0x11, 0x04, 0xFE, 0x00], 0).unwrap();

        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].starting_vcn, 0);
        assert_eq!(runs[0].cluster_count, 3);
        assert_eq!(runs[0].lcn, Some(10));
        assert_eq!(runs[1].starting_vcn, 3);
        assert_eq!(runs[1].cluster_count, 2);
        assert_eq!(runs[1].lcn, None);
        assert_eq!(runs[2].starting_vcn, 5);
        assert_eq!(runs[2].cluster_count, 4);
        assert_eq!(runs[2].lcn, Some(8));
    }

    #[test]
    fn rejects_missing_terminator_and_truncated_run() {
        assert!(parse_data_runs(&[0x11, 0x01, 0x02], 0).is_err());
        assert!(parse_data_runs(&[0x21, 0x01, 0x02], 0).is_err());
    }
}
