#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::runlist::parse_data_runs;

fuzz_target!(|data: &[u8]| {
    let bytes = support::corpus_bytes(data);
    if let Ok(runs) = parse_data_runs(&bytes, 0) {
        let reparsed = parse_data_runs(&bytes, 0).unwrap();
        assert_eq!(runs, reparsed);
        let mut expected_vcn = 0_u64;
        for run in runs {
            assert_eq!(run.starting_vcn, expected_vcn);
            assert!(run.cluster_count > 0);
            expected_vcn = expected_vcn.checked_add(run.cluster_count).unwrap();
        }
    }
});
