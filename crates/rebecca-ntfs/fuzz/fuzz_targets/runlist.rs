#![no_main]

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::runlist::parse_data_runs;

fuzz_target!(|data: &[u8]| {
    let _ = parse_data_runs(data, 0);
});
