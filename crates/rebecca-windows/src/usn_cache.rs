use std::path::Path;

use rebecca_core::RebeccaError;
use rebecca_core::scan_cache::{
    ScanCacheRecord, ScanCacheUsnChange, ScanCacheUsnInvalidationReason, ScanCacheUsnJournalState,
    ScanCacheUsnValidation, ScanCacheUsnValidator,
};

pub const USN_CACHE_VALIDATION_LABEL: &str = "windows-usn-cache-validation";

pub fn live_usn_cache_validation_unavailable(path: &Path) -> RebeccaError {
    RebeccaError::PlatformUnavailable(format!(
        "{USN_CACHE_VALIDATION_LABEL} is not enabled for {}; falling back to normal scan-cache fingerprint and identity policy",
        path.display()
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotUsnCacheValidator {
    journal_state: Option<ScanCacheUsnJournalState>,
    changes: Vec<ScanCacheUsnChange>,
    range_readable: bool,
}

impl SnapshotUsnCacheValidator {
    pub fn unsupported() -> Self {
        Self {
            journal_state: None,
            changes: Vec::new(),
            range_readable: false,
        }
    }

    pub fn readable(
        journal_state: ScanCacheUsnJournalState,
        changes: Vec<ScanCacheUsnChange>,
    ) -> Self {
        Self {
            journal_state: Some(journal_state),
            changes,
            range_readable: true,
        }
    }

    pub fn unreadable_range(journal_state: ScanCacheUsnJournalState) -> Self {
        Self {
            journal_state: Some(journal_state),
            changes: Vec::new(),
            range_readable: false,
        }
    }
}

impl ScanCacheUsnValidator for SnapshotUsnCacheValidator {
    fn validate_record(&self, record: &ScanCacheRecord) -> ScanCacheUsnValidation {
        if record.identity.usn_checkpoint.is_none() {
            return ScanCacheUsnValidation::Unsupported;
        }

        let Some(journal_state) = &self.journal_state else {
            return ScanCacheUsnValidation::Unsupported;
        };

        if !self.range_readable {
            return ScanCacheUsnValidation::Invalidated(
                ScanCacheUsnInvalidationReason::RangeUnavailable,
            );
        }

        record.validate_usn_journal(journal_state, &self.changes)
    }
}
