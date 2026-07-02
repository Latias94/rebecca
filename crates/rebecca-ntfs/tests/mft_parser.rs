use rebecca_ntfs::{
    MftIndex, MftRecordReader, NtfsFileReference, NtfsParseError, NtfsParsedRecord,
};

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
fn valid_fixture_record_parses_name_parent_and_stream_size() {
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

    let record = NtfsParsedRecord::parse_mft_record(42, &raw, SECTOR_SIZE).unwrap();

    assert!(record.in_use);
    assert!(!record.is_directory);
    assert_eq!(record.cleanup_logical_size(), 1234);
    let name = record.primary_file_name().unwrap();
    assert_eq!(name.parent.record_id, 5);
    assert_eq!(name.parent.sequence_number, Some(0));
    assert_eq!(name.name, "cache.bin");
}

#[test]
fn parsed_record_preserves_owned_references_and_stream_shape() {
    let raw = mft_record(
        42,
        true,
        false,
        vec![
            file_name_attr_with_parent_reference(file_reference(5, 7), "cache.bin", 0, 1),
            nonresident_data_attr(1234),
        ],
    );

    let parsed = NtfsParsedRecord::parse_mft_record(42, &raw, SECTOR_SIZE).unwrap();

    assert_eq!(parsed.reference, NtfsFileReference::known(42, 42));
    let name = parsed.primary_file_name().unwrap();
    assert_eq!(name.parent, NtfsFileReference::known(5, 7));
    assert_eq!(name.name, "cache.bin");
    assert_eq!(parsed.cleanup_logical_size(), 1234);
    assert_eq!(parsed.data_streams.len(), 1);
    let stream = &parsed.data_streams[0];
    assert_eq!(stream.attribute_id, 0);
    assert_eq!(stream.name, None);
    assert_eq!(stream.lowest_vcn, Some(0));
    assert_eq!(stream.highest_vcn, Some(0));
    assert_eq!(stream.logical_size, 1234);
    assert_eq!(stream.allocated_size, Some(1234));
    assert_eq!(stream.initialized_size, Some(1234));
    assert_eq!(stream.data_runs.len(), 1);
    assert_eq!(stream.data_runs[0].starting_vcn, 0);
    assert_eq!(stream.data_runs[0].cluster_count, 1);
    assert_eq!(stream.data_runs[0].lcn, Some(32));
    assert!(parsed.attributes.iter().any(|attribute| {
        attribute.attribute_id == 0
            && attribute.attribute_type == rebecca_ntfs::AttributeType::Data
            && attribute.non_resident
            && attribute.lowest_vcn == Some(0)
            && attribute.highest_vcn == Some(0)
    }));
}

#[test]
fn resident_and_named_data_streams_keep_cleanup_size_conservative() {
    let record = NtfsParsedRecord::parse(
        43,
        &mft_record(
            43,
            true,
            false,
            vec![
                file_name_attr(5, "streams.bin", 0),
                resident_data_attr(b"abc"),
                named_resident_data_attr("secret", b"hidden"),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    assert_eq!(record.cleanup_logical_size(), 3);
    assert_eq!(record.data_streams.len(), 2);
    let unnamed = record
        .data_streams
        .iter()
        .find(|stream| stream.name.is_none())
        .unwrap();
    assert_eq!(unnamed.logical_size, 3);
    assert_eq!(unnamed.allocated_size, Some(3));
    assert_eq!(unnamed.initialized_size, Some(3));
    assert!(unnamed.data_runs.is_empty());

    let named = record
        .data_streams
        .iter()
        .find(|stream| stream.name.as_deref() == Some("secret"))
        .unwrap();
    assert_eq!(named.logical_size, 6);
    assert!(named.data_runs.is_empty());
    assert!(record.caveats.iter().any(|c| c.code == "named-data-stream"));
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

    let err = NtfsParsedRecord::parse_mft_record(9, &raw, SECTOR_SIZE).unwrap_err();

    assert_eq!(err, NtfsParseError::InvalidUpdateSequence);
}

#[test]
fn parent_child_index_aggregation_sums_subtree_bytes() {
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
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 15);
    assert_eq!(summary.files, 2);
    assert_eq!(summary.directories, 2);
    assert!(summary.caveats.is_empty());
}

#[test]
fn index_resolves_child_paths_case_insensitively() {
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
                true,
                vec![file_name_attr(5, "Cache", FILE_ATTRIBUTE_DIRECTORY)],
            ),
        ),
        (
            7,
            mft_record(
                7,
                true,
                false,
                vec![file_name_attr(6, "DATA.BIN", 0), resident_data_attr(b"abc")],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let entry = index.find_path(5, ["cache", "data.bin"]).unwrap();

    assert_eq!(entry.reference.record_id, 7);
}

#[test]
fn deleted_and_pathless_records_are_reported_as_caveats() {
    let deleted = NtfsParsedRecord::parse_mft_record(
        11,
        &mft_record(11, false, false, vec![file_name_attr(5, "deleted.bin", 0)]),
        SECTOR_SIZE,
    )
    .unwrap();
    let pathless = NtfsParsedRecord::parse_mft_record(
        12,
        &mft_record(12, true, false, vec![nonresident_data_attr(99)]),
        SECTOR_SIZE,
    )
    .unwrap();

    assert!(deleted.caveats.iter().any(|c| c.code == "deleted-record"));
    assert!(pathless.caveats.iter().any(|c| c.code == "pathless-record"));
}

#[test]
fn reparse_records_are_identifiable_and_skipped_by_index_aggregation() {
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
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap())
    .collect::<Vec<_>>();

    assert!(records[1].is_reparse_point);
    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);

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
fn resident_attribute_list_entries_are_structured_and_caveated() {
    let record = NtfsParsedRecord::parse_mft_record(
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

    assert_eq!(record.attribute_list_entries.len(), 1);
    let entry = &record.attribute_list_entries[0];
    assert_eq!(entry.attribute_type, rebecca_ntfs::AttributeType::Data);
    assert_eq!(entry.lowest_vcn, 4);
    assert_eq!(entry.file_reference, NtfsFileReference::known(99, 3));
    assert_eq!(entry.attribute_id, 9);
    assert!(
        record
            .caveats
            .iter()
            .any(|c| c.code == "attribute-list-present")
    );
}

#[test]
fn recursive_attribute_list_entries_are_refused() {
    let record = NtfsParsedRecord::parse_mft_record(
        13,
        &mft_record(
            13,
            true,
            false,
            vec![
                file_name_attr(5, "recursive.bin", 0),
                attribute_list_attr_with_entry(
                    ATTR_ATTRIBUTE_LIST,
                    None,
                    0,
                    file_reference(13, 13),
                    2,
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    assert!(
        record
            .caveats
            .iter()
            .any(|c| { c.code == "recursive-attribute-list-unsupported" })
    );
}

#[test]
fn attribute_list_extension_data_is_resolved_for_index_aggregation() {
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
            20,
            mft_record(
                20,
                true,
                false,
                vec![
                    file_name_attr(5, "split.bin", 0),
                    attribute_list_attr_with_entry(ATTR_DATA, None, 0, file_reference(21, 21), 0),
                ],
            ),
        ),
        (
            21,
            mft_record_with_base(
                21,
                file_reference(20, 20),
                true,
                false,
                vec![nonresident_data_attr(123)],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 123);
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "attribute-list-present")
    );
    assert!(
        !summary
            .caveats
            .iter()
            .any(|c| c.code == "attribute-list-extension-records-unexpanded")
    );
}

#[test]
fn subtree_aggregation_surfaces_record_caveats() {
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
            13,
            mft_record(
                13,
                true,
                false,
                vec![
                    file_name_attr(5, "listed.bin", 0),
                    attribute_list_attr(),
                    nonresident_data_attr(7),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 7);
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "attribute-list-present")
    );
}

#[test]
fn subtree_aggregation_caveats_multiple_non_dos_file_names() {
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
            14,
            mft_record(
                14,
                true,
                false,
                vec![
                    file_name_attr(5, "first.bin", 0),
                    file_name_attr_with_namespace(5, "second.bin", 0, 0),
                    nonresident_data_attr(11),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 11);
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "multiple-file-names")
    );
}

#[test]
fn truncated_data_returns_error_without_panic() {
    let err = NtfsParsedRecord::parse_mft_record(1, b"FILE", SECTOR_SIZE).unwrap_err();

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

#[test]
fn reader_parses_batches_from_nonzero_base_record_id() {
    let mut bytes = mft_record(100, true, false, vec![file_name_attr(5, "a.bin", 0)]);
    bytes.extend_from_slice(&mft_record(
        101,
        true,
        false,
        vec![file_name_attr(5, "b.bin", 0)],
    ));

    let batch = MftRecordReader::default().parse_records_from(100, &bytes);

    assert!(batch.errors.is_empty());
    assert_eq!(
        batch
            .records
            .iter()
            .map(|record| record.reference.record_id)
            .collect::<Vec<_>>(),
        vec![100, 101]
    );
}

#[test]
fn reader_reports_remainder_error_at_base_record_offset() {
    let mut bytes = mft_record(100, true, false, vec![file_name_attr(5, "a.bin", 0)]);
    bytes.extend_from_slice(&[0_u8; 16]);

    let batch = MftRecordReader::default().parse_records_from(100, &bytes);

    assert_eq!(batch.records.len(), 1);
    assert_eq!(batch.errors.len(), 1);
    assert_eq!(batch.errors[0].record_id, 101);
    assert!(matches!(
        batch.errors[0].error,
        NtfsParseError::Truncated { .. }
    ));
}

fn mft_record(record_id: u64, in_use: bool, directory: bool, attrs: Vec<Vec<u8>>) -> Vec<u8> {
    mft_record_with_base(record_id, 0, in_use, directory, attrs)
}

fn mft_record_with_base(
    record_id: u64,
    base_reference: u64,
    in_use: bool,
    directory: bool,
    attrs: Vec<Vec<u8>>,
) -> Vec<u8> {
    let mut record = vec![0_u8; RECORD_SIZE];
    record[0..4].copy_from_slice(b"FILE");
    put_u16(&mut record, 4, USA_OFFSET as u16);
    put_u16(&mut record, 6, 3);
    put_u16(&mut record, 16, record_id as u16);
    put_u16(&mut record, 20, FIRST_ATTR_OFFSET as u16);
    let flags = u16::from(in_use) | if directory { 0x0002 } else { 0 };
    put_u16(&mut record, 22, flags);
    put_u32(&mut record, 28, RECORD_SIZE as u32);
    put_u64(&mut record, 32, base_reference);
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
    file_name_attr_with_namespace(parent_id, name, file_attributes, 1)
}

fn file_name_attr_with_namespace(
    parent_id: u64,
    name: &str,
    file_attributes: u32,
    namespace: u8,
) -> Vec<u8> {
    file_name_attr_with_parent_reference(parent_id, name, file_attributes, namespace)
}

fn file_name_attr_with_parent_reference(
    parent_reference: u64,
    name: &str,
    file_attributes: u32,
    namespace: u8,
) -> Vec<u8> {
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let mut value = vec![0_u8; 66 + (name_utf16.len() * 2)];
    put_u64(&mut value, 0, parent_reference);
    put_u64(&mut value, 40, 0);
    put_u64(&mut value, 48, 0);
    put_u32(&mut value, 56, file_attributes);
    value[64] = name_utf16.len() as u8;
    value[65] = namespace;
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut value, 66 + (index * 2), *character);
    }
    resident_attr(ATTR_FILE_NAME, &value)
}

fn file_reference(record_id: u64, sequence_number: u16) -> u64 {
    ((sequence_number as u64) << 48) | (record_id & 0x0000_FFFF_FFFF_FFFF)
}

fn resident_data_attr(bytes: &[u8]) -> Vec<u8> {
    resident_attr(ATTR_DATA, bytes)
}

fn named_resident_data_attr(name: &str, bytes: &[u8]) -> Vec<u8> {
    resident_named_attr(ATTR_DATA, name, bytes)
}

fn attribute_list_attr() -> Vec<u8> {
    attribute_list_attr_with_entry(ATTR_DATA, None, 4, file_reference(99, 3), 9)
}

fn attribute_list_attr_with_entry(
    attribute_type: u32,
    name: Option<&str>,
    lowest_vcn: u64,
    file_reference: u64,
    attribute_id: u16,
) -> Vec<u8> {
    resident_attr(
        ATTR_ATTRIBUTE_LIST,
        &attribute_list_entry(
            attribute_type,
            name,
            lowest_vcn,
            file_reference,
            attribute_id,
        ),
    )
}

fn attribute_list_entry(
    attribute_type: u32,
    name: Option<&str>,
    lowest_vcn: u64,
    file_reference: u64,
    attribute_id: u16,
) -> Vec<u8> {
    let name_utf16 = name
        .map(|name| name.encode_utf16().collect::<Vec<_>>())
        .unwrap_or_default();
    let name_len = name_utf16.len() * 2;
    let name_offset = 26;
    let entry_len = align8(name_offset + name_len);
    let mut entry = vec![0_u8; entry_len];
    put_u32(&mut entry, 0, attribute_type);
    put_u16(&mut entry, 4, entry_len as u16);
    entry[6] = name_utf16.len() as u8;
    entry[7] = name_offset as u8;
    put_u64(&mut entry, 8, lowest_vcn);
    put_u64(&mut entry, 16, file_reference);
    put_u16(&mut entry, 24, attribute_id);
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut entry, name_offset + (index * 2), *character);
    }
    entry
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

fn resident_named_attr(attribute_type: u32, name: &str, value: &[u8]) -> Vec<u8> {
    let header_len = 24;
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let name_len = name_utf16.len() * 2;
    let value_offset = align8(header_len + name_len);
    let total_len = align8(value_offset + value.len());
    let mut attr = vec![0_u8; total_len];
    put_u32(&mut attr, 0, attribute_type);
    put_u32(&mut attr, 4, total_len as u32);
    attr[8] = 0;
    attr[9] = name_utf16.len() as u8;
    put_u16(&mut attr, 10, header_len as u16);
    put_u32(&mut attr, 16, value.len() as u32);
    put_u16(&mut attr, 20, value_offset as u16);
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut attr, header_len + (index * 2), *character);
    }
    attr[value_offset..value_offset + value.len()].copy_from_slice(value);
    attr
}

fn nonresident_data_attr(data_size: u64) -> Vec<u8> {
    nonresident_data_attr_with_runlist(data_size, 0, 0, &[0x11, 0x01, 0x20, 0x00])
}

fn nonresident_data_attr_with_runlist(
    data_size: u64,
    lowest_vcn: u64,
    highest_vcn: u64,
    runlist: &[u8],
) -> Vec<u8> {
    let runlist_offset = 64;
    let total_len = align8(runlist_offset + runlist.len());
    let mut attr = vec![0_u8; total_len];
    put_u32(&mut attr, 0, ATTR_DATA);
    put_u32(&mut attr, 4, total_len as u32);
    attr[8] = 1;
    put_u64(&mut attr, 16, lowest_vcn);
    put_u64(&mut attr, 24, highest_vcn);
    put_u16(&mut attr, 32, runlist_offset as u16);
    put_u64(&mut attr, 40, data_size);
    put_u64(&mut attr, 48, data_size);
    put_u64(&mut attr, 56, data_size);
    attr[runlist_offset..runlist_offset + runlist.len()].copy_from_slice(runlist);
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
