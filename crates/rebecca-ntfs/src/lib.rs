//! Read-only NTFS Master File Table parsing primitives.
//!
//! This crate intentionally parses exported record bytes and in-memory fixtures.
//! It does not open volumes, require elevation, or provide deletion authority.

pub mod adapter;
pub mod attrs;
pub mod fixup;
pub mod index;
pub mod reader;
pub mod record;
pub mod runlist;

mod parse;

pub use adapter::{
    NtfsDataRun, NtfsDataStream, NtfsDirectoryEntry, NtfsFileName, NtfsFileReference,
    NtfsParsedAttribute, NtfsParsedRecord,
};
pub use attrs::{AttributeHeader, AttributeType};
pub use index::{MftIndex, MftIndexEntry, SubtreeSummary};
pub use reader::{MftRecordBatch, MftRecordError, MftRecordReader};
pub use record::{FileNameNamespace, ParseCaveat};

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
}
