#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::dir_index::{parse_i30_index_allocation_record, parse_i30_index_root};

fuzz_target!(|data: &[u8]| {
    let bytes = support::corpus_bytes(data);
    if let Ok(root) = parse_i30_index_root(&bytes) {
        let reparsed = parse_i30_index_root(&bytes).unwrap();
        assert_eq!(root, reparsed);
    }
    if let Ok(record) = parse_i30_index_allocation_record(&bytes, 512, 0) {
        let reparsed = parse_i30_index_allocation_record(&bytes, 512, 0).unwrap();
        assert_eq!(record, reparsed);
    }
});
