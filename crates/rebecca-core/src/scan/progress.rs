use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::error::{RebeccaError, Result};

#[derive(Debug, Clone, Default)]
pub struct ScanCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl ScanCancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ScanProgressEvent<'a> {
    FileMeasured {
        path: &'a Path,
        file_size: u64,
        files_scanned: u64,
        bytes_scanned: u64,
    },
}

pub(crate) fn check_not_cancelled(cancellation: &ScanCancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(RebeccaError::OperationCancelled(
            "scan was cancelled".to_string(),
        ));
    }

    Ok(())
}
