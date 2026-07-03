#![no_main]

use libfuzzer_sys::fuzz_target;
use rebecca_ntfs::NtfsParsedRecord;

fuzz_target!(|data: &[u8]| {
    let _ = NtfsParsedRecord::parse(0, data, 512);
    let _ = NtfsParsedRecord::parse(0, data, 4096);
});
