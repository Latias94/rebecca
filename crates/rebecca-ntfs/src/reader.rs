use serde::{Deserialize, Serialize};

use crate::NtfsParseError;
use crate::adapter::NtfsParsedRecord;
use crate::record::ParseCaveat;

const MFT_MIRROR_RECORD_USED_CAVEAT: &str = "mft-mirror-record-used";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MftRecordReader {
    record_size: usize,
    sector_size: usize,
}

impl MftRecordReader {
    pub const fn new(record_size: usize, sector_size: usize) -> Self {
        Self {
            record_size,
            sector_size,
        }
    }

    pub const fn record_size(&self) -> usize {
        self.record_size
    }

    pub const fn sector_size(&self) -> usize {
        self.sector_size
    }

    pub fn parse_records(&self, bytes: &[u8]) -> MftRecordBatch {
        self.parse_records_from(0, bytes)
    }

    pub fn parse_records_from(&self, base_record_id: u64, bytes: &[u8]) -> MftRecordBatch {
        let mut records = Vec::new();
        let mut errors = Vec::new();

        if self.record_size == 0 || self.sector_size == 0 {
            errors.push(MftRecordError {
                record_id: base_record_id,
                error: NtfsParseError::InvalidUpdateSequence,
            });
            return MftRecordBatch { records, errors };
        }

        for (record_index, raw_record) in bytes.chunks_exact(self.record_size).enumerate() {
            let record_id = base_record_id.saturating_add(record_index as u64);
            match NtfsParsedRecord::parse(record_id, raw_record, self.sector_size) {
                Ok(record) => records.push(record),
                Err(error) => errors.push(MftRecordError { record_id, error }),
            }
        }

        let remainder = bytes.chunks_exact(self.record_size).remainder();
        if !remainder.is_empty() {
            errors.push(MftRecordError {
                record_id: base_record_id.saturating_add((bytes.len() / self.record_size) as u64),
                error: NtfsParseError::Truncated {
                    expected: self.record_size,
                    actual: remainder.len(),
                },
            });
        }

        MftRecordBatch { records, errors }
    }

    pub fn parse_records_with_mirror(
        &self,
        primary_bytes: &[u8],
        mirror_bytes: &[u8],
    ) -> MftRecordBatch {
        self.parse_records_from_with_mirror(0, primary_bytes, 0, mirror_bytes)
    }

    pub fn parse_records_from_with_mirror(
        &self,
        primary_base_record_id: u64,
        primary_bytes: &[u8],
        mirror_base_record_id: u64,
        mirror_bytes: &[u8],
    ) -> MftRecordBatch {
        let mut records = Vec::new();
        let mut errors = Vec::new();

        if self.record_size == 0 || self.sector_size == 0 {
            errors.push(MftRecordError {
                record_id: primary_base_record_id,
                error: NtfsParseError::InvalidUpdateSequence,
            });
            return MftRecordBatch { records, errors };
        }

        for (record_index, raw_record) in primary_bytes.chunks_exact(self.record_size).enumerate() {
            let record_id = primary_base_record_id.saturating_add(record_index as u64);
            match NtfsParsedRecord::parse(record_id, raw_record, self.sector_size) {
                Ok(record) => records.push(record),
                Err(primary_error) => match self.parse_mirror_record(
                    record_id,
                    mirror_base_record_id,
                    mirror_bytes,
                    &primary_error,
                ) {
                    Some(Ok(record)) => records.push(record),
                    Some(Err(_)) | None => errors.push(MftRecordError {
                        record_id,
                        error: primary_error,
                    }),
                },
            }
        }

        let remainder = primary_bytes.chunks_exact(self.record_size).remainder();
        if !remainder.is_empty() {
            let record_id = primary_base_record_id
                .saturating_add((primary_bytes.len() / self.record_size) as u64);
            let primary_error = NtfsParseError::Truncated {
                expected: self.record_size,
                actual: remainder.len(),
            };
            match self.parse_mirror_record(
                record_id,
                mirror_base_record_id,
                mirror_bytes,
                &primary_error,
            ) {
                Some(Ok(record)) => records.push(record),
                Some(Err(_)) | None => errors.push(MftRecordError {
                    record_id,
                    error: primary_error,
                }),
            }
        }

        MftRecordBatch { records, errors }
    }

    fn parse_mirror_record(
        &self,
        record_id: u64,
        mirror_base_record_id: u64,
        mirror_bytes: &[u8],
        primary_error: &NtfsParseError,
    ) -> Option<Result<NtfsParsedRecord, NtfsParseError>> {
        let raw_record =
            self.mirror_record_slice(record_id, mirror_base_record_id, mirror_bytes)?;
        Some(
            NtfsParsedRecord::parse(record_id, raw_record, self.sector_size).map(|mut record| {
                record
                    .caveats
                    .push(mft_mirror_record_used_caveat(record_id, primary_error));
                record
            }),
        )
    }

    fn mirror_record_slice<'a>(
        &self,
        record_id: u64,
        mirror_base_record_id: u64,
        mirror_bytes: &'a [u8],
    ) -> Option<&'a [u8]> {
        let record_index = record_id.checked_sub(mirror_base_record_id)?;
        let offset = usize::try_from(record_index)
            .ok()?
            .checked_mul(self.record_size)?;
        let end = offset.checked_add(self.record_size)?;
        mirror_bytes.get(offset..end)
    }
}

fn mft_mirror_record_used_caveat(record_id: u64, primary_error: &NtfsParseError) -> ParseCaveat {
    ParseCaveat::new(
        MFT_MIRROR_RECORD_USED_CAVEAT,
        format!(
            "record {record_id} was recovered from $MFTMirr after primary $MFT parse failed: {primary_error}"
        ),
    )
}

impl Default for MftRecordReader {
    fn default() -> Self {
        Self::new(1024, 512)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftRecordBatch {
    pub records: Vec<NtfsParsedRecord>,
    pub errors: Vec<MftRecordError>,
}

impl MftRecordBatch {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MftRecordError {
    pub record_id: u64,
    pub error: NtfsParseError,
}
