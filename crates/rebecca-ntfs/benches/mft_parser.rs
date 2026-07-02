use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rebecca_ntfs::MftRecordReader;

const RECORD_SIZE: usize = 1024;
const SECTOR_SIZE: usize = 512;
const USA_OFFSET: usize = 0x30;
const FIRST_ATTR_OFFSET: usize = 0x38;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;

fn mft_parser(criterion: &mut Criterion) {
    let fixture = build_fixture(1024);
    let reader = MftRecordReader::default();

    criterion.bench_function("parse_generated_1024_mft_records", |bencher| {
        bencher.iter(|| {
            let batch = reader.parse_records(black_box(&fixture));
            assert_eq!(batch.records.len(), 1024);
            assert!(batch.errors.is_empty());
            black_box(batch);
        });
    });
}

fn build_fixture(records: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(records * RECORD_SIZE);
    for record_id in 0..records {
        bytes.extend_from_slice(&mft_record(
            record_id as u64,
            vec![
                file_name_attr(5, &format!("file-{record_id:04}.bin")),
                nonresident_data_attr(128),
            ],
        ));
    }
    bytes
}

fn mft_record(record_id: u64, attrs: Vec<Vec<u8>>) -> Vec<u8> {
    let mut record = vec![0_u8; RECORD_SIZE];
    record[0..4].copy_from_slice(b"FILE");
    put_u16(&mut record, 4, USA_OFFSET as u16);
    put_u16(&mut record, 6, 3);
    put_u16(&mut record, 16, record_id as u16);
    put_u16(&mut record, 20, FIRST_ATTR_OFFSET as u16);
    put_u16(&mut record, 22, 0x0001);
    put_u32(&mut record, 28, RECORD_SIZE as u32);
    put_u32(&mut record, 44, record_id as u32);

    let mut offset = FIRST_ATTR_OFFSET;
    for attr in attrs {
        record[offset..offset + attr.len()].copy_from_slice(&attr);
        offset += attr.len();
    }
    put_u32(&mut record, offset, 0xFFFF_FFFF);
    offset += 4;
    put_u32(&mut record, 24, offset as u32);
    apply_test_fixup(&mut record);
    record
}

fn file_name_attr(parent_id: u64, name: &str) -> Vec<u8> {
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let mut value = vec![0_u8; 66 + (name_utf16.len() * 2)];
    put_u64(&mut value, 0, parent_id);
    value[64] = name_utf16.len() as u8;
    value[65] = 1;
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut value, 66 + (index * 2), *character);
    }
    resident_attr(ATTR_FILE_NAME, &value)
}

fn resident_attr(attribute_type: u32, value: &[u8]) -> Vec<u8> {
    let header_len = 24;
    let total_len = align8(header_len + value.len());
    let mut attr = vec![0_u8; total_len];
    put_u32(&mut attr, 0, attribute_type);
    put_u32(&mut attr, 4, total_len as u32);
    put_u32(&mut attr, 16, value.len() as u32);
    put_u16(&mut attr, 20, header_len as u16);
    attr[header_len..header_len + value.len()].copy_from_slice(value);
    attr
}

fn nonresident_data_attr(data_size: u64) -> Vec<u8> {
    let runlist = [0x11, 0x01, 0x20, 0x00];
    let runlist_offset = 64;
    let total_len = align8(runlist_offset + runlist.len());
    let mut attr = vec![0_u8; total_len];
    put_u32(&mut attr, 0, ATTR_DATA);
    put_u32(&mut attr, 4, total_len as u32);
    attr[8] = 1;
    put_u64(&mut attr, 24, 0);
    put_u16(&mut attr, 32, runlist_offset as u16);
    put_u64(&mut attr, 40, data_size);
    put_u64(&mut attr, 48, data_size);
    put_u64(&mut attr, 56, data_size);
    attr[runlist_offset..runlist_offset + runlist.len()].copy_from_slice(&runlist);
    attr
}

fn apply_test_fixup(record: &mut [u8]) {
    let usn = 0xAAAA_u16;
    let first_tail = u16::from_le_bytes([record[SECTOR_SIZE - 2], record[SECTOR_SIZE - 1]]);
    let second_tail = u16::from_le_bytes([record[RECORD_SIZE - 2], record[RECORD_SIZE - 1]]);
    put_u16(record, USA_OFFSET, usn);
    put_u16(record, USA_OFFSET + 2, first_tail);
    put_u16(record, USA_OFFSET + 4, second_tail);
    put_u16(record, SECTOR_SIZE - 2, usn);
    put_u16(record, RECORD_SIZE - 2, usn);
}

fn align8(value: usize) -> usize {
    (value + 7) & !7
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

criterion_group!(benches, mft_parser);
criterion_main!(benches);
