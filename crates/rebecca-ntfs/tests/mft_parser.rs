use rebecca_ntfs::record::MftRecord;
use rebecca_ntfs::{MftRecordReader, MftTree, NtfsParseError};

const RECORD_SIZE: usize = 1024;
const SECTOR_SIZE: usize = 512;
const USA_OFFSET: usize = 0x30;
const FIRST_ATTR_OFFSET: usize = 0x38;
const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_REPARSE_POINT: u32 = 0xC0;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

#[test]
fn valid_fixture_record_parses_name_parent_and_data_size() {
    let raw = mft_record(
        42,
        true,
        false,
        vec![
            standard_information_attr(0),
            file_name_attr(5, "cache.bin", 0),
            nonresident_data_attr(1234),
        ],
    );

    let record = MftRecord::parse(42, &raw, SECTOR_SIZE).unwrap();

    assert!(record.in_use);
    assert!(!record.is_directory);
    assert_eq!(record.data_size, 1234);
    let name = record.primary_file_name().unwrap();
    assert_eq!(name.parent_record_id, 5);
    assert_eq!(name.name, "cache.bin");
}

#[test]
fn invalid_fixups_fail_safely() {
    let mut raw = mft_record(
        9,
        true,
        false,
        vec![
            file_name_attr(5, "broken.bin", 0),
            resident_data_attr(b"abc"),
        ],
    );
    raw[SECTOR_SIZE - 2] = 0;
    raw[SECTOR_SIZE - 1] = 0;

    let err = MftRecord::parse(9, &raw, SECTOR_SIZE).unwrap_err();

    assert_eq!(err, NtfsParseError::InvalidUpdateSequence);
}

#[test]
fn parent_child_tree_aggregation_sums_subtree_bytes() {
    let records = [
        (
            5,
            mft_record(
                5,
                true,
                true,
                vec![file_name_attr(5, "root", FILE_ATTRIBUTE_DIRECTORY)],
            ),
        ),
        (
            6,
            mft_record(
                6,
                true,
                false,
                vec![file_name_attr(5, "a.bin", 0), nonresident_data_attr(10)],
            ),
        ),
        (
            7,
            mft_record(
                7,
                true,
                true,
                vec![file_name_attr(5, "nested", FILE_ATTRIBUTE_DIRECTORY)],
            ),
        ),
        (
            8,
            mft_record(
                8,
                true,
                false,
                vec![file_name_attr(7, "b.bin", 0), resident_data_attr(b"12345")],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| MftRecord::parse(id, &raw, SECTOR_SIZE).unwrap());

    let tree = MftTree::from_records(records);
    let summary = tree.aggregate_subtree(5);

    assert_eq!(summary.bytes, 15);
    assert_eq!(summary.files, 2);
    assert_eq!(summary.directories, 2);
    assert!(summary.caveats.is_empty());
}

#[test]
fn deleted_and_pathless_records_are_reported_as_caveats() {
    let deleted = MftRecord::parse(
        11,
        &mft_record(11, false, false, vec![file_name_attr(5, "deleted.bin", 0)]),
        SECTOR_SIZE,
    )
    .unwrap();
    let pathless = MftRecord::parse(
        12,
        &mft_record(12, true, false, vec![nonresident_data_attr(99)]),
        SECTOR_SIZE,
    )
    .unwrap();

    assert!(deleted.caveats.iter().any(|c| c.code == "deleted-record"));
    assert!(pathless.caveats.iter().any(|c| c.code == "pathless-record"));
}

#[test]
fn reparse_records_are_identifiable_and_skipped_by_tree_aggregation() {
    let records = [
        (
            5,
            mft_record(
                5,
                true,
                true,
                vec![file_name_attr(5, "root", FILE_ATTRIBUTE_DIRECTORY)],
            ),
        ),
        (
            6,
            mft_record(
                6,
                true,
                false,
                vec![
                    file_name_attr(5, "junction", FILE_ATTRIBUTE_REPARSE_POINT),
                    reparse_point_attr(),
                    nonresident_data_attr(123),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| MftRecord::parse(id, &raw, SECTOR_SIZE).unwrap())
    .collect::<Vec<_>>();

    assert!(records[1].is_reparse_point);
    let tree = MftTree::from_records(records);
    let summary = tree.aggregate_subtree(5);

    assert_eq!(summary.bytes, 0);
    assert_eq!(summary.files, 0);
    assert_eq!(summary.directories, 1);
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "reparse-point-skipped")
    );
}

#[test]
fn attribute_list_records_report_caveat() {
    let record = MftRecord::parse(
        13,
        &mft_record(
            13,
            true,
            false,
            vec![
                file_name_attr(5, "listed.bin", 0),
                attribute_list_attr(),
                nonresident_data_attr(7),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    assert!(
        record
            .caveats
            .iter()
            .any(|c| c.code == "attribute-list-present")
    );
}

#[test]
fn truncated_data_returns_error_without_panic() {
    let err = MftRecord::parse(1, b"FILE", SECTOR_SIZE).unwrap_err();

    assert!(matches!(err, NtfsParseError::Truncated { .. }));
}

#[test]
fn reader_collects_records_and_truncated_remainder_errors() {
    let mut bytes = mft_record(0, true, false, vec![file_name_attr(5, "a.bin", 0)]);
    bytes.extend_from_slice(&[0_u8; 16]);

    let batch = MftRecordReader::default().parse_records(&bytes);

    assert_eq!(batch.records.len(), 1);
    assert_eq!(batch.errors.len(), 1);
    assert!(matches!(
        batch.errors[0].error,
        NtfsParseError::Truncated { .. }
    ));
}

fn mft_record(record_id: u64, in_use: bool, directory: bool, attrs: Vec<Vec<u8>>) -> Vec<u8> {
    let mut record = vec![0_u8; RECORD_SIZE];
    record[0..4].copy_from_slice(b"FILE");
    put_u16(&mut record, 4, USA_OFFSET as u16);
    put_u16(&mut record, 6, 3);
    put_u16(&mut record, 16, record_id as u16);
    put_u16(&mut record, 20, FIRST_ATTR_OFFSET as u16);
    let flags = u16::from(in_use) | if directory { 0x0002 } else { 0 };
    put_u16(&mut record, 22, flags);
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

fn standard_information_attr(file_attributes: u32) -> Vec<u8> {
    let mut value = vec![0_u8; 72];
    put_u32(&mut value, 32, file_attributes);
    resident_attr(ATTR_STANDARD_INFORMATION, &value)
}

fn file_name_attr(parent_id: u64, name: &str, file_attributes: u32) -> Vec<u8> {
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let mut value = vec![0_u8; 66 + (name_utf16.len() * 2)];
    put_u64(&mut value, 0, parent_id);
    put_u64(&mut value, 40, 0);
    put_u64(&mut value, 48, 0);
    put_u32(&mut value, 56, file_attributes);
    value[64] = name_utf16.len() as u8;
    value[65] = 1;
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut value, 66 + (index * 2), *character);
    }
    resident_attr(ATTR_FILE_NAME, &value)
}

fn resident_data_attr(bytes: &[u8]) -> Vec<u8> {
    resident_attr(ATTR_DATA, bytes)
}

fn attribute_list_attr() -> Vec<u8> {
    resident_attr(ATTR_ATTRIBUTE_LIST, &[0_u8; 32])
}

fn reparse_point_attr() -> Vec<u8> {
    resident_attr(ATTR_REPARSE_POINT, &[0_u8; 8])
}

fn resident_attr(attribute_type: u32, value: &[u8]) -> Vec<u8> {
    let header_len = 24;
    let total_len = align8(header_len + value.len());
    let mut attr = vec![0_u8; total_len];
    put_u32(&mut attr, 0, attribute_type);
    put_u32(&mut attr, 4, total_len as u32);
    attr[8] = 0;
    put_u32(&mut attr, 16, value.len() as u32);
    put_u16(&mut attr, 20, header_len as u16);
    attr[header_len..header_len + value.len()].copy_from_slice(value);
    attr
}

fn nonresident_data_attr(data_size: u64) -> Vec<u8> {
    let mut attr = vec![0_u8; 64];
    put_u32(&mut attr, 0, ATTR_DATA);
    put_u32(&mut attr, 4, 64);
    attr[8] = 1;
    put_u16(&mut attr, 32, 64);
    put_u64(&mut attr, 40, data_size);
    put_u64(&mut attr, 48, data_size);
    put_u64(&mut attr, 56, data_size);
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
