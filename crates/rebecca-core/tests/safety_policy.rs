use std::path::PathBuf;

use rebecca_core::safety::{PathDisposition, assess_existing_path, assess_path};

#[test]
fn allows_user_cache_subdirectories() {
    let disposition = assess_path(&PathBuf::from(
        "C:/Users/Alice/AppData/Local/Temp/rebecca-test",
    ));

    assert!(matches!(disposition, PathDisposition::Allowed));
}

#[test]
fn blocks_traversal_drive_roots_and_system_paths() {
    let cases = [
        "../Windows",
        "C:/",
        "C:/Windows/System32",
        "C:/Program Files/App",
        "C:/Users/Alice",
    ];

    for case in cases {
        let disposition = assess_path(&PathBuf::from(case));
        assert!(
            matches!(disposition, PathDisposition::Blocked(_)),
            "{case} should be blocked, got {disposition:?}"
        );
    }
}

#[test]
fn missing_existing_path_is_skipped() {
    let disposition = assess_existing_path(&PathBuf::from("C:/Rebecca/definitely-missing"));

    assert!(matches!(disposition, PathDisposition::Skipped(_)));
}
