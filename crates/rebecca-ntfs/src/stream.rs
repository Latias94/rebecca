use serde::{Deserialize, Serialize};

use crate::adapter::NtfsDataRun;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsStreamGeometry {
    pub bytes_per_cluster: u64,
    pub bytes_per_sector: usize,
}

impl NtfsStreamGeometry {
    pub const fn new(bytes_per_cluster: u64, bytes_per_sector: usize) -> Self {
        Self {
            bytes_per_cluster,
            bytes_per_sector,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparseRunPolicy {
    Reject,
    ZeroFill,
}

pub trait NtfsStreamSource {
    type Error: std::fmt::Display;

    fn read_bytes_at(
        &mut self,
        volume_offset: u64,
        len: usize,
    ) -> std::result::Result<Vec<u8>, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum NtfsStreamReadError {
    #[error("cluster size must be greater than zero")]
    InvalidClusterSize,

    #[error("stream offset overflowed")]
    OffsetOverflow,

    #[error("data run order has a gap: expected VCN {expected_vcn}, got {actual_vcn}")]
    VcnGap { expected_vcn: u64, actual_vcn: u64 },

    #[error("data run order moved backwards from VCN {expected_vcn} to {actual_vcn}")]
    VcnBacktrack { expected_vcn: u64, actual_vcn: u64 },

    #[error("sparse run at VCN {starting_vcn} is not supported for this stream")]
    SparseRun { starting_vcn: u64 },

    #[error("source read returned {actual} bytes, expected {expected}")]
    ShortRead { expected: usize, actual: usize },

    #[error("stream source read failed: {0}")]
    Source(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtfsStreamReader {
    bytes_per_cluster: u64,
    sparse_policy: SparseRunPolicy,
}

impl NtfsStreamReader {
    pub const fn new(bytes_per_cluster: u64, sparse_policy: SparseRunPolicy) -> Self {
        Self {
            bytes_per_cluster,
            sparse_policy,
        }
    }

    pub fn read_range<S>(
        &self,
        source: &mut S,
        runs: &[NtfsDataRun],
        logical_offset: u64,
        len: usize,
    ) -> Result<Vec<u8>, NtfsStreamReadError>
    where
        S: NtfsStreamSource,
    {
        if self.bytes_per_cluster == 0 {
            return Err(NtfsStreamReadError::InvalidClusterSize);
        }
        if len == 0 {
            return Ok(Vec::new());
        }

        let wanted_start = logical_offset;
        let wanted_end = logical_offset
            .checked_add(u64::try_from(len).map_err(|_| NtfsStreamReadError::OffsetOverflow)?)
            .ok_or(NtfsStreamReadError::OffsetOverflow)?;
        let mut expected_vcn = 0_u64;
        let mut output = Vec::with_capacity(len);

        for run in runs {
            if run.starting_vcn > expected_vcn {
                return Err(NtfsStreamReadError::VcnGap {
                    expected_vcn,
                    actual_vcn: run.starting_vcn,
                });
            }
            if run.starting_vcn < expected_vcn {
                return Err(NtfsStreamReadError::VcnBacktrack {
                    expected_vcn,
                    actual_vcn: run.starting_vcn,
                });
            }

            let run_start = run
                .starting_vcn
                .checked_mul(self.bytes_per_cluster)
                .ok_or(NtfsStreamReadError::OffsetOverflow)?;
            let run_len = run
                .cluster_count
                .checked_mul(self.bytes_per_cluster)
                .ok_or(NtfsStreamReadError::OffsetOverflow)?;
            let run_end = run_start
                .checked_add(run_len)
                .ok_or(NtfsStreamReadError::OffsetOverflow)?;
            expected_vcn = run
                .starting_vcn
                .checked_add(run.cluster_count)
                .ok_or(NtfsStreamReadError::OffsetOverflow)?;

            let overlap_start = wanted_start.max(run_start);
            let overlap_end = wanted_end.min(run_end);
            if overlap_start >= overlap_end {
                continue;
            }
            let overlap_len = usize::try_from(overlap_end - overlap_start)
                .map_err(|_| NtfsStreamReadError::OffsetOverflow)?;

            let mut bytes = match run.lcn {
                Some(lcn) => {
                    let run_relative_offset = overlap_start
                        .checked_sub(run_start)
                        .ok_or(NtfsStreamReadError::OffsetOverflow)?;
                    let volume_offset = lcn
                        .checked_mul(self.bytes_per_cluster)
                        .and_then(|offset| offset.checked_add(run_relative_offset))
                        .ok_or(NtfsStreamReadError::OffsetOverflow)?;
                    let bytes = source
                        .read_bytes_at(volume_offset, overlap_len)
                        .map_err(|err| NtfsStreamReadError::Source(err.to_string()))?;
                    if bytes.len() != overlap_len {
                        return Err(NtfsStreamReadError::ShortRead {
                            expected: overlap_len,
                            actual: bytes.len(),
                        });
                    }
                    bytes
                }
                None => match self.sparse_policy {
                    SparseRunPolicy::Reject => {
                        return Err(NtfsStreamReadError::SparseRun {
                            starting_vcn: run.starting_vcn,
                        });
                    }
                    SparseRunPolicy::ZeroFill => vec![0_u8; overlap_len],
                },
            };
            output.append(&mut bytes);
        }

        if output.len() != len {
            return Err(NtfsStreamReadError::ShortRead {
                expected: len,
                actual: output.len(),
            });
        }

        Ok(output)
    }
}
