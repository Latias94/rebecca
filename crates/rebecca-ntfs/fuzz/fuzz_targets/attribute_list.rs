#![no_main]

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::attribute_list::parse_attribute_list;

fuzz_target!(|data: &[u8]| {
    let _ = parse_attribute_list(data);
});
