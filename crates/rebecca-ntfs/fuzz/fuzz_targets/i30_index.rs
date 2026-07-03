#![no_main]

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::dir_index::{parse_i30_index_allocation_record, parse_i30_index_root};

fuzz_target!(|data: &[u8]| {
    let _ = parse_i30_index_root(data);
    let _ = parse_i30_index_allocation_record(data, 512, 0);
});
