use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::error::{RebeccaError, Result};

#[derive(Debug, Clone)]
pub struct ScanCancellationToken {
    state: Arc<ScanCancellationState>,
}

#[derive(Debug, Default)]
struct ScanCancellationState {
    cancelled: AtomicBool,
    parent: Option<ScanCancellationToken>,
}

impl Default for ScanCancellationToken {
    fn default() -> Self {
        Self {
            state: Arc::new(ScanCancellationState::default()),
        }
    }
}

impl ScanCancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn child_token(&self) -> Self {
        Self {
            state: Arc::new(ScanCancellationState {
                cancelled: AtomicBool::new(false),
                parent: Some(self.clone()),
            }),
        }
    }

    pub fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
            || self
                .state
                .parent
                .as_ref()
                .is_some_and(ScanCancellationToken::is_cancelled)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_cancellation_token_follows_parent_without_cancelling_parent() {
        let parent = ScanCancellationToken::new();
        let child = parent.child_token();

        assert!(!parent.is_cancelled());
        assert!(!child.is_cancelled());

        child.cancel();

        assert!(!parent.is_cancelled());
        assert!(child.is_cancelled());

        let sibling = parent.child_token();
        parent.cancel();

        assert!(parent.is_cancelled());
        assert!(sibling.is_cancelled());
    }
}
