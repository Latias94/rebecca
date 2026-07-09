#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::fuzzing::parse_attribute_list;

fuzz_target!(|data: &[u8]| {
    let bytes = support::corpus_bytes(data);
    if let Ok(entries) = parse_attribute_list(&bytes) {
        let reparsed = parse_attribute_list(&bytes).unwrap();
        assert_eq!(entries, reparsed);
        for entry in entries {
            assert!(entry.file_reference.record_id <= 0x0000_FFFF_FFFF_FFFF);
        }
    }
});
