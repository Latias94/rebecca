//! Read-only NTFS Master File Table parsing primitives.
//!
//! This crate intentionally parses exported record bytes and in-memory fixtures.
//! It does not open volumes, require elevation, or provide deletion authority.

pub mod adapter;
pub mod attribute_list;
pub mod attrs;
pub mod dir_index;
pub mod fixup;
pub mod index;
pub mod reader;
pub mod record;
pub mod record_set;
pub mod runlist;
pub mod stream;

mod parse;

pub use adapter::{
    NtfsAttributeListEntry, NtfsAttributeStream, NtfsDataRun, NtfsDirectoryEntry,
    NtfsDirectoryIndex, NtfsFileName, NtfsFileReference, NtfsIndexEntry, NtfsParsedAttribute,
    NtfsParsedRecord,
};
pub use attrs::{AttributeHeader, AttributeType};
pub use index::{MftIndex, MftIndexEntry, MftPathCandidate, SubtreeSummary};
pub use reader::{MftRecordBatch, MftRecordError, MftRecordReader};
pub use record::{FileNameNamespace, ParseCaveat};
pub use record_set::NtfsRecordSet;
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
