//! Read-only NTFS Master File Table parsing primitives.
//!
//! This crate intentionally parses exported record bytes and in-memory fixtures.
//! It does not open volumes, require elevation, or provide deletion authority.

mod adapter;
mod attribute_list;
mod attrs;
mod dir_index;
mod fixup;
mod index;
mod reader;
mod record;
mod record_set;
mod runlist;
mod stream;

mod parse;

#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzzing {
    pub use crate::attribute_list::parse_attribute_list;
    pub use crate::dir_index::{parse_i30_index_allocation_record, parse_i30_index_root};
    pub use crate::runlist::parse_data_runs;
}

pub use adapter::{
    NtfsAttributeListEntry, NtfsAttributeStream, NtfsDataRun, NtfsDirectoryEntry,
    NtfsDirectoryEntrySource, NtfsDirectoryIndex, NtfsFileName, NtfsFileReference, NtfsIndexEntry,
    NtfsParsedAttribute, NtfsParsedRecord,
};
pub use attrs::{AttributeHeader, AttributeType};
pub use index::{
    DirectoryEdge, DirectoryEdgeConfidence, DirectoryEdgeSequenceStatus, DirectoryEdgeSource,
    MftIndex, MftIndexEntry, MftPathCandidate, PhysicalMetrics, PhysicalMetricsAccumulator,
    SubtreeSummary,
};
pub use reader::{MftRecordBatch, MftRecordError, MftRecordReader};
pub use record::{FileNameNamespace, ParseCaveat};
pub use record_set::{NtfsRecordResolver, NtfsRecordSet, resolve_record_with_stream_source};
pub use stream::{
    NtfsStreamGeometry, NtfsStreamReadError, NtfsStreamReader, NtfsStreamSource, SparseRunPolicy,
};

pub type Result<T> = std::result::Result<T, NtfsParseError>;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum NtfsParseError {
    #[error("record is too small: expected at least {expected} bytes, got {actual}")]
    Truncated { expected: usize, actual: usize },

    #[error("record signature is not FILE")]
    InvalidSignature,

    #[error("invalid update sequence array")]
    InvalidUpdateSequence,

    #[error(
        "record attribute bounds are invalid: first attribute offset {first_attribute_offset}, used size {used_size}, record size {record_size}"
    )]
    InvalidRecordBounds {
        first_attribute_offset: usize,
        used_size: usize,
        record_size: usize,
    },

    #[error("invalid attribute header at offset {offset}")]
    InvalidAttribute { offset: usize },

    #[error("attribute at offset {offset} is truncated")]
    TruncatedAttribute { offset: usize },

    #[error("resident attribute value at offset {offset} is truncated")]
    TruncatedResidentValue { offset: usize },

    #[error("file name attribute is invalid")]
    InvalidFileName,

    #[error("data run list is invalid")]
    InvalidRunlist,

    #[error("attribute list is invalid")]
    InvalidAttributeList,

    #[error("directory index is invalid")]
    InvalidDirectoryIndex,
}
