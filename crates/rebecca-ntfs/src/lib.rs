//! Read-only NTFS Master File Table parsing primitives.
//!
//! This crate intentionally parses exported record bytes and in-memory fixtures.
//! It does not open volumes, require elevation, or provide deletion authority.

pub mod attrs;
pub mod fixup;
pub mod reader;
pub mod record;
pub mod tree;

mod parse;

pub use attrs::{AttributeHeader, AttributeType};
pub use reader::{MftRecordBatch, MftRecordError, MftRecordReader};
pub use record::{FileName, FileNameNamespace, MftRecord, ParseCaveat};
pub use tree::{MftTree, MftTreeEntry, SubtreeSummary};

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
}
