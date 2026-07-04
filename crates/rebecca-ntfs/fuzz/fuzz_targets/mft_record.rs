#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::NtfsParsedRecord;

fuzz_target!(|data: &[u8]| {
    let bytes = support::corpus_bytes(data);
    for sector_size in [512, 4096] {
        if let Ok(record) = NtfsParsedRecord::parse(0, &bytes, sector_size) {
            let reparsed = NtfsParsedRecord::parse(0, &bytes, sector_size).unwrap();
            assert_eq!(record.reference, reparsed.reference);
            assert_eq!(
                record.cleanup_logical_size(),
                reparsed.cleanup_logical_size()
            );
            for stream in &record.attribute_streams {
                if let (Some(lowest), Some(highest)) = (stream.lowest_vcn, stream.highest_vcn) {
                    assert!(lowest <= highest);
                }
            }
        }
    }
});
