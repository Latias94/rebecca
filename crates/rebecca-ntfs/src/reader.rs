use serde::{Deserialize, Serialize};

use crate::NtfsParseError;
use crate::adapter::NtfsParsedRecord;

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
