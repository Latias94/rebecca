use std::path::PathBuf;

use rebecca_core::scan::{ScanBackendKind, ScanEstimateConfidence, ScanReport};
use rebecca_core::scan_cache::{
    SCAN_CACHE_VERSION, ScanCacheFileType, ScanCacheFingerprint, ScanCacheIdentity,
    ScanCacheRecord, ScanCacheUsnChange, ScanCacheUsnCheckpoint, ScanCacheUsnInvalidationReason,
    ScanCacheUsnJournalState, ScanCacheUsnValidation, ScanCacheUsnValidator,
};
use rebecca_windows::usn_cache::{
    SnapshotUsnCacheValidator, USN_CACHE_VALIDATION_LABEL, live_usn_cache_validation_unavailable,
};

#[test]
fn unsupported_usn_validation_falls_back_to_normal_cache_policy() {
    let record = usn_record();
    let validator = SnapshotUsnCacheValidator::unsupported();

    assert_eq!(
        validator.validate_record(&record),
        ScanCacheUsnValidation::Unsupported
    );
}

#[test]
fn changed_target_subtree_invalidates_cache_record() {
    let record = usn_record();
    let validator = SnapshotUsnCacheValidator::readable(
        ScanCacheUsnJournalState {
            journal_id: 9,
            first_usn: 1,
            next_usn: 200,
        },
        vec![ScanCacheUsnChange::new(77, Some(76), 120).with_ancestor_file_ids(vec![42])],
    );

    assert_eq!(
        validator.validate_record(&record),
        ScanCacheUsnValidation::Invalidated(ScanCacheUsnInvalidationReason::TargetChanged)
    );
}

#[test]
fn journal_id_mismatch_invalidates_cache_record() {
    let record = usn_record();
    let validator = SnapshotUsnCacheValidator::readable(
        ScanCacheUsnJournalState {
            journal_id: 10,
            first_usn: 1,
            next_usn: 200,
        },
        Vec::new(),
    );

    assert_eq!(
        validator.validate_record(&record),
        ScanCacheUsnValidation::Invalidated(ScanCacheUsnInvalidationReason::JournalChanged)
    );
}

#[test]
fn unreadable_usn_range_is_conservative_miss() {
    let record = usn_record();
    let validator = SnapshotUsnCacheValidator::unreadable_range(ScanCacheUsnJournalState {
        journal_id: 9,
        first_usn: 1,
        next_usn: 200,
    });

    assert_eq!(
        validator.validate_record(&record),
        ScanCacheUsnValidation::Invalidated(ScanCacheUsnInvalidationReason::RangeUnavailable)
    );
}

#[test]
fn live_usn_cache_validation_unavailable_names_fallback_boundary() {
    let err = live_usn_cache_validation_unavailable(std::path::Path::new("C:\\Temp"));

    assert!(err.to_string().contains(USN_CACHE_VALIDATION_LABEL));
    assert!(err.to_string().contains("normal scan-cache"));
}

fn usn_record() -> ScanCacheRecord {
    ScanCacheRecord {
        version: SCAN_CACHE_VERSION,
        root: PathBuf::from("C:\\Temp"),
        backend: ScanBackendKind::WindowsNative,
        backend_source: None,
        confidence: ScanEstimateConfidence::Exact,
        identity: ScanCacheIdentity {
            volume_serial: Some(5),
            file_id: Some(42),
            usn_checkpoint: Some(ScanCacheUsnCheckpoint {
                journal_id: 9,
                next_usn: 100,
            }),
        },
        fingerprint: ScanCacheFingerprint {
            file_type: ScanCacheFileType::Directory,
            len: 0,
            modified_unix_seconds: Some(1),
        },
        report: ScanReport {
            bytes_scanned: 1,
            files_scanned: 1,
            directories_scanned: 1,
        },
        written_at_unix_seconds: 1,
    }
}
