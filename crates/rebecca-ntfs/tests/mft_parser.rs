use std::collections::BTreeMap;

use rebecca_ntfs::{
    AttributeType, MftIndex, MftRecordReader, NtfsDataRun, NtfsFileReference, NtfsParseError,
    NtfsParsedRecord, NtfsRecordSet, NtfsStreamGeometry, NtfsStreamReadError, NtfsStreamReader,
    NtfsStreamSource, SparseRunPolicy, resolve_record_with_stream_source,
};

const RECORD_SIZE: usize = 1024;
const SECTOR_SIZE: usize = 512;
const USA_OFFSET: usize = 0x30;
const FIRST_ATTR_OFFSET: usize = 0x38;
const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_INDEX_ROOT: u32 = 0x90;
const ATTR_INDEX_ALLOCATION: u32 = 0xA0;
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
    assert_eq!(record.cleanup_allocated_size(), Some(1234));
    let name = record.primary_file_name().unwrap();
    assert_eq!(name.parent.record_id, 5);
    assert_eq!(name.parent.sequence_number, Some(5));
    assert_eq!(name.name, "cache.bin");
}

#[test]
fn fsctl_applied_fixup_record_parses_name_parent_and_stream_size() {
    let mut raw = mft_record(
        42,
        true,
        false,
        vec![
            standard_information_attr(0),
            file_name_attr(5, "cache.bin", 0),
            nonresident_data_attr(1234),
        ],
    );
    mark_test_fixup_as_already_applied(&mut raw);

    let record = NtfsParsedRecord::parse_mft_record(42, &raw, SECTOR_SIZE).unwrap();

    assert!(record.in_use);
    assert_eq!(record.cleanup_logical_size(), 1234);
    assert_eq!(record.primary_file_name().unwrap().name, "cache.bin");
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
    assert_eq!(parsed.attribute_streams.len(), 1);
    let stream = &parsed.attribute_streams[0];
    assert_eq!(stream.attribute_type, AttributeType::Data);
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
fn resident_and_named_attribute_streams_keep_cleanup_size_conservative() {
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
    assert_eq!(record.attribute_streams.len(), 2);
    let unnamed = record
        .attribute_streams
        .iter()
        .find(|stream| stream.attribute_type == AttributeType::Data && stream.name.is_none())
        .unwrap();
    assert_eq!(unnamed.logical_size, 3);
    assert_eq!(unnamed.allocated_size, Some(3));
    assert_eq!(unnamed.initialized_size, Some(3));
    assert!(unnamed.data_runs.is_empty());

    let named = record
        .attribute_streams
        .iter()
        .find(|stream| {
            stream.attribute_type == AttributeType::Data && stream.name.as_deref() == Some("secret")
        })
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
    assert_eq!(summary.allocated_bytes, Some(15));
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
fn parent_sequence_mismatch_is_caveated_and_not_counted() {
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
                    file_name_attr_with_parent_reference(file_reference(5, 99), "stale.bin", 0, 1),
                    nonresident_data_attr(10),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 0);
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "parent-sequence-mismatch")
    );
}

#[test]
fn hardlink_path_candidates_resolve_without_double_counting() {
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
                vec![file_name_attr(5, "a", FILE_ATTRIBUTE_DIRECTORY)],
            ),
        ),
        (
            7,
            mft_record(
                7,
                true,
                true,
                vec![file_name_attr(5, "b", FILE_ATTRIBUTE_DIRECTORY)],
            ),
        ),
        (
            8,
            mft_record(
                8,
                true,
                false,
                vec![
                    file_name_attr_with_parent_reference(file_reference(6, 6), "left.bin", 0, 1),
                    file_name_attr_with_parent_reference(file_reference(7, 7), "right.bin", 0, 1),
                    nonresident_data_attr(10),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let left = index.find_path(5, ["a", "left.bin"]).unwrap();
    let right = index.find_path(5, ["b", "right.bin"]).unwrap();
    let summary = index.aggregate_subtree(5);

    assert_eq!(left.reference.record_id, 8);
    assert_eq!(right.reference.record_id, 8);
    assert_eq!(left.path_candidates.len(), 2);
    assert_eq!(summary.bytes, 10);
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "hardlink-path-candidates")
    );
}

#[test]
fn resident_i30_index_root_can_supply_verified_fallback_edge() {
    let records = [
        (
            5,
            mft_record(
                5,
                true,
                true,
                vec![
                    file_name_attr(5, "root", FILE_ATTRIBUTE_DIRECTORY),
                    index_root_attr(file_reference(6, 6), file_reference(5, 5), "indexed.bin", 0),
                ],
            ),
        ),
        (
            6,
            mft_record(
                6,
                true,
                false,
                vec![
                    file_name_attr_with_parent_reference(
                        file_reference(99, 99),
                        "indexed.bin",
                        0,
                        1,
                    ),
                    nonresident_data_attr(12),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap())
    .collect::<Vec<_>>();

    assert_eq!(records[0].directory_entries.len(), 1);
    assert_eq!(records[0].directory_entries[0].name, "indexed.bin");

    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);
    let child = index.get(6).unwrap();

    assert_eq!(summary.bytes, 12);
    assert!(
        child
            .caveats
            .iter()
            .any(|c| c.code == "directory-index-parent-map-fallback"),
        "{:?}",
        child.caveats
    );
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "directory-index-parent-map-fallback"),
        "{:?}",
        summary.caveats
    );
}

#[test]
fn i30_index_allocation_record_parses_valid_indx_entries() {
    let raw = index_allocation_record(
        0,
        vec![
            index_allocation_entry(file_reference(6, 6), file_reference(5, 5), "large.bin", 0),
            index_allocation_last_entry(false),
        ],
    );

    let record =
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&raw, SECTOR_SIZE, 0).unwrap();
    let entries = record.directory_entries().collect::<Vec<_>>();

    assert_eq!(record.vcn, 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].child, NtfsFileReference::known(6, 6));
    assert_eq!(entries[0].parent, NtfsFileReference::known(5, 5));
    assert_eq!(entries[0].name, "large.bin");
    assert_eq!(record.entries[0].child_vcn, None);
}

#[test]
fn i30_index_allocation_record_rejects_bad_fixup_signature_bounds_and_vcn() {
    let mut bad_fixup = index_allocation_record(
        0,
        vec![
            index_allocation_entry(file_reference(6, 6), file_reference(5, 5), "large.bin", 0),
            index_allocation_last_entry(false),
        ],
    );
    bad_fixup[SECTOR_SIZE - 2] = 0;
    assert!(
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&bad_fixup, SECTOR_SIZE, 0)
            .is_err()
    );

    let mut bad_signature = index_allocation_record(0, vec![index_allocation_last_entry(false)]);
    bad_signature[0..4].copy_from_slice(b"BAD!");
    assert!(
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&bad_signature, SECTOR_SIZE, 0)
            .is_err()
    );

    let mut bad_bounds = index_allocation_record(0, vec![index_allocation_last_entry(false)]);
    put_u32(&mut bad_bounds, 32, 8);
    assert!(
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&bad_bounds, SECTOR_SIZE, 0)
            .is_err()
    );

    let wrong_vcn = index_allocation_record(4, vec![index_allocation_last_entry(false)]);
    assert!(
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&wrong_vcn, SECTOR_SIZE, 0)
            .is_err()
    );
}

#[test]
fn i30_index_allocation_record_rejects_short_entries_and_skips_last_subnode() {
    let short_entry = index_allocation_record_with_raw_entries(0, vec![0_u8; 12]);
    assert!(
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&short_entry, SECTOR_SIZE, 0)
            .is_err()
    );

    let last_subnode = index_allocation_record(8, vec![index_allocation_last_entry(true)]);
    let record =
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&last_subnode, SECTOR_SIZE, 8)
            .unwrap();
    assert_eq!(record.directory_entries().count(), 0);
    assert_eq!(record.entries.len(), 1);
    assert!(record.entries[0].is_last);
    assert_eq!(record.entries[0].child_vcn, Some(16));
}

#[test]
fn i30_index_entries_preserve_child_vcns() {
    let raw = index_allocation_record(
        0,
        vec![
            index_allocation_entry_with_child_vcn(
                file_reference(6, 6),
                file_reference(5, 5),
                "branch.bin",
                0,
                Some(8),
            ),
            index_allocation_last_entry(true),
        ],
    );
    let record =
        rebecca_ntfs::dir_index::parse_i30_index_allocation_record(&raw, SECTOR_SIZE, 0).unwrap();

    assert_eq!(record.entries[0].child_vcn, Some(8));
    assert_eq!(
        record.entries[0].directory_entry.as_ref().unwrap().name,
        "branch.bin"
    );
    assert!(record.entries[1].is_last);
    assert_eq!(record.entries[1].child_vcn, Some(16));

    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 8),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let root_entry = &directory.directory_indexes[0].root_entries[0];
    assert!(root_entry.is_last);
    assert_eq!(root_entry.child_vcn, Some(8));
    assert!(directory.directory_entries.is_empty());
}

#[test]
fn nonresident_i30_index_allocation_is_preserved_as_attribute_stream() {
    let record = NtfsParsedRecord::parse_mft_record(
        15,
        &mft_record(
            15,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                index_allocation_attr(),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    assert!(record.directory_entries.is_empty());
    let stream = record
        .attribute_streams
        .iter()
        .find(|stream| {
            stream.attribute_type == AttributeType::IndexAllocation
                && stream.name.as_deref() == Some("$I30")
        })
        .unwrap();
    assert_eq!(stream.logical_size, 0);
    assert!(stream.data_runs.is_empty());
}

#[test]
fn record_set_expands_nonresident_i30_index_allocation() {
    let mut source = FakeStreamSource::default().with_bytes(
        0x80_000,
        &index_allocation_record(
            0,
            vec![
                index_allocation_entry(file_reference(6, 6), file_reference(5, 5), "large.bin", 0),
                index_allocation_last_entry(false),
            ],
        ),
    );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    RECORD_SIZE as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let child = NtfsParsedRecord::parse_mft_record(
        6,
        &mft_record(
            6,
            true,
            false,
            vec![
                file_name_attr(99, "large.bin", 0),
                resident_data_attr(b"abc"),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory, child],
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set
        .records
        .iter()
        .find(|record| record.reference.record_id == 5)
        .unwrap();
    assert_eq!(directory.directory_entries.len(), 1);
    assert_eq!(directory.directory_entries[0].name, "large.bin");
}

#[test]
fn record_set_caveats_invalid_i30_index_allocation_without_edges() {
    let mut source = FakeStreamSource::default().with_bytes(0x80_000, b"not-indx");
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    RECORD_SIZE as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory],
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set.records.first().unwrap();
    assert!(directory.directory_entries.is_empty());
    assert!(
        directory
            .caveats
            .iter()
            .any(|caveat| caveat.code == "invalid-index-allocation")
    );
}

#[test]
fn record_set_expands_attribute_list_i30_index_allocation_extension() {
    let mut source = FakeStreamSource::default().with_bytes(
        0x90_000,
        &index_allocation_record(
            0,
            vec![
                index_allocation_entry(file_reference(6, 6), file_reference(5, 5), "split.bin", 0),
                index_allocation_last_entry(false),
            ],
        ),
    );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                attribute_list_attr_with_entry(
                    ATTR_INDEX_ALLOCATION,
                    Some("$I30"),
                    0,
                    file_reference(50, 50),
                    0,
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let extension = NtfsParsedRecord::parse_mft_record(
        50,
        &mft_record_with_base(
            50,
            file_reference(5, 5),
            true,
            true,
            vec![nonresident_named_attr(
                ATTR_INDEX_ALLOCATION,
                "$I30",
                RECORD_SIZE as u64,
                0,
                &[0x21, 0x01, 0x90, 0x00, 0x00],
            )],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let child = NtfsParsedRecord::parse_mft_record(
        6,
        &mft_record(
            6,
            true,
            false,
            vec![
                file_name_attr(99, "split.bin", 0),
                resident_data_attr(b"abc"),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory, extension, child],
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set
        .records
        .iter()
        .find(|record| record.reference.record_id == 5)
        .unwrap();
    assert_eq!(directory.directory_entries.len(), 1);
    assert_eq!(directory.directory_entries[0].name, "split.bin");
    assert!(
        !directory
            .caveats
            .iter()
            .any(|caveat| caveat.code == "attribute-list-extension-records-unexpanded"),
        "{:?}",
        directory.caveats
    );
}

#[test]
fn single_record_resolution_lazily_merges_attribute_list_data_extension() {
    let base = NtfsParsedRecord::parse_mft_record(
        20,
        &mft_record(
            20,
            true,
            false,
            vec![
                file_name_attr(5, "split.bin", 0),
                attribute_list_attr_with_entry(ATTR_DATA, None, 0, file_reference(21, 21), 0),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let extension = NtfsParsedRecord::parse_mft_record(
        21,
        &mft_record_with_base(
            21,
            file_reference(20, 20),
            true,
            false,
            vec![nonresident_data_attr(123)],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let mut source = FakeStreamSource::default();
    let mut extension_reads = 0;

    let resolved = resolve_record_with_stream_source(
        base,
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
        |reference| {
            extension_reads += 1;
            assert_eq!(reference, NtfsFileReference::known(21, 21));
            Ok::<_, ()>(Some(extension.clone()))
        },
    )
    .unwrap();

    assert_eq!(extension_reads, 1);
    assert_eq!(resolved.cleanup_logical_size(), 123);
    assert!(
        !resolved
            .caveats
            .iter()
            .any(|caveat| caveat.code == "attribute-list-extension-records-unexpanded"),
        "{:?}",
        resolved.caveats
    );
}

#[test]
fn single_record_resolution_lazily_merges_attribute_list_i30_extension() {
    let mut source = FakeStreamSource::default().with_bytes(
        0x90_000,
        &index_allocation_record(
            0,
            vec![
                index_allocation_entry(file_reference(6, 6), file_reference(5, 5), "split.bin", 0),
                index_allocation_last_entry(false),
            ],
        ),
    );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                attribute_list_attr_with_entry(
                    ATTR_INDEX_ALLOCATION,
                    Some("$I30"),
                    0,
                    file_reference(50, 50),
                    0,
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let extension = NtfsParsedRecord::parse_mft_record(
        50,
        &mft_record_with_base(
            50,
            file_reference(5, 5),
            true,
            true,
            vec![nonresident_named_attr(
                ATTR_INDEX_ALLOCATION,
                "$I30",
                RECORD_SIZE as u64,
                0,
                &[0x21, 0x01, 0x90, 0x00, 0x00],
            )],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let resolved = resolve_record_with_stream_source(
        directory,
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
        |reference| {
            assert_eq!(reference, NtfsFileReference::known(50, 50));
            Ok::<_, ()>(Some(extension.clone()))
        },
    )
    .unwrap();

    assert_eq!(resolved.directory_entries.len(), 1);
    assert_eq!(resolved.directory_entries[0].name, "split.bin");
    assert!(
        !resolved
            .caveats
            .iter()
            .any(|caveat| caveat.code == "attribute-list-extension-records-unexpanded"),
        "{:?}",
        resolved.caveats
    );
}

#[test]
fn record_set_expands_fragmented_multi_record_i30_index_allocation() {
    let cluster_size = RECORD_SIZE as u64;
    let mut source = FakeStreamSource::default()
        .with_bytes(
            0x80 * cluster_size,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry_with_child_vcn(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "first.bin",
                        0,
                        Some(1),
                    ),
                    index_allocation_last_entry(false),
                ],
            ),
        )
        .with_bytes(
            0x90 * cluster_size,
            &index_allocation_record(
                1,
                vec![
                    index_allocation_entry(
                        file_reference(7, 7),
                        file_reference(5, 5),
                        "second.bin",
                        0,
                    ),
                    index_allocation_last_entry(false),
                ],
            ),
        );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    (RECORD_SIZE * 2) as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x11, 0x01, 0x10, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let first_child = NtfsParsedRecord::parse_mft_record(
        6,
        &mft_record(
            6,
            true,
            false,
            vec![
                file_name_attr(99, "first.bin", 0),
                resident_data_attr(b"abc"),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let second_child = NtfsParsedRecord::parse_mft_record(
        7,
        &mft_record(
            7,
            true,
            false,
            vec![
                file_name_attr(99, "second.bin", 0),
                resident_data_attr(b"wxyz"),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory, first_child, second_child],
        NtfsStreamGeometry::new(cluster_size, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set
        .records
        .iter()
        .find(|record| record.reference.record_id == 5)
        .unwrap();
    assert_eq!(directory.directory_entries.len(), 2);
    assert!(
        !directory
            .caveats
            .iter()
            .any(|caveat| caveat.code == "invalid-index-allocation"),
        "{:?}",
        directory.caveats
    );

    let index = MftIndex::from_record_set(record_set);
    let summary = index.aggregate_subtree(5);
    assert_eq!(summary.bytes, 7);
    assert_eq!(
        index
            .find_path(5, ["first.bin"])
            .unwrap()
            .reference
            .record_id,
        6
    );
    assert_eq!(
        index
            .find_path(5, ["second.bin"])
            .unwrap()
            .reference
            .record_id,
        7
    );
}

#[test]
fn record_set_ignores_unreachable_i30_index_allocation_records() {
    let cluster_size = RECORD_SIZE as u64;
    let mut source = FakeStreamSource::default()
        .with_bytes(
            0x80 * cluster_size,
            &index_allocation_record(
                0,
                vec![
                    index_allocation_entry(
                        file_reference(6, 6),
                        file_reference(5, 5),
                        "reachable.bin",
                        0,
                    ),
                    index_allocation_last_entry(false),
                ],
            ),
        )
        .with_bytes(
            0x90 * cluster_size,
            &index_allocation_record(
                1,
                vec![
                    index_allocation_entry(
                        file_reference(7, 7),
                        file_reference(5, 5),
                        "unreachable.bin",
                        0,
                    ),
                    index_allocation_last_entry(false),
                ],
            ),
        );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    (RECORD_SIZE * 2) as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x11, 0x01, 0x10, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory],
        NtfsStreamGeometry::new(cluster_size, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set
        .records
        .iter()
        .find(|record| record.reference.record_id == 5)
        .unwrap();
    assert_eq!(directory.directory_entries.len(), 1);
    assert_eq!(directory.directory_entries[0].name, "reachable.bin");
    assert!(
        !directory
            .directory_entries
            .iter()
            .any(|entry| entry.name == "unreachable.bin")
    );
}

#[test]
fn record_set_caveats_repeated_i30_child_vcn_without_looping() {
    let cluster_size = RECORD_SIZE as u64;
    let mut source = FakeStreamSource::default().with_bytes(
        0x80 * cluster_size,
        &index_allocation_record(0, vec![index_allocation_last_entry_with_child_vcn(0)]),
    );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    RECORD_SIZE as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory],
        NtfsStreamGeometry::new(cluster_size, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set.records.first().unwrap();
    assert!(directory.directory_entries.is_empty());
    assert!(directory.caveats.iter().any(|caveat| {
        caveat.code == "invalid-index-allocation" && caveat.message.contains("already visited")
    }));
}

#[test]
fn record_set_caveats_i30_child_vcn_out_of_range() {
    let mut source = FakeStreamSource::default();
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 1),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    RECORD_SIZE as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory],
        NtfsStreamGeometry::new(RECORD_SIZE as u64, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set.records.first().unwrap();
    assert!(directory.directory_entries.is_empty());
    assert!(directory.caveats.iter().any(|caveat| {
        caveat.code == "invalid-index-allocation" && caveat.message.contains("beyond stream size")
    }));
}

#[test]
fn record_set_caveats_i30_child_vcn_geometry_requiring_multi_record_clusters() {
    let mut source = FakeStreamSource::default();
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 1),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    (RECORD_SIZE * 2) as u64,
                    0,
                    &[0x21, 0x02, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory],
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
    );

    let directory = record_set.records.first().unwrap();
    assert!(directory.directory_entries.is_empty());
    assert!(directory.caveats.iter().any(|caveat| {
        caveat.code == "invalid-index-allocation" && caveat.message.contains("unsupported geometry")
    }));
}

#[test]
fn nonresident_i30_index_allocation_supplies_mft_index_fallback_edge() {
    let mut source = FakeStreamSource::default().with_bytes(
        0x80_000,
        &index_allocation_record(
            0,
            vec![
                index_allocation_entry(file_reference(6, 6), file_reference(5, 5), "large.bin", 0),
                index_allocation_last_entry(false),
            ],
        ),
    );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    RECORD_SIZE as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let child = NtfsParsedRecord::parse_mft_record(
        6,
        &mft_record(
            6,
            true,
            false,
            vec![
                file_name_attr(99, "large.bin", 0),
                resident_data_attr(b"abc"),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory, child],
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
    );

    let index = MftIndex::from_record_set(record_set);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 3);
    assert_eq!(
        index
            .find_child(5, "large.bin")
            .unwrap()
            .reference
            .record_id,
        6
    );
    assert_eq!(
        index
            .find_path(5, ["large.bin"])
            .unwrap()
            .reference
            .record_id,
        6
    );
    assert!(
        summary
            .caveats
            .iter()
            .any(|caveat| caveat.code == "directory-index-parent-map-fallback"),
        "{:?}",
        summary.caveats
    );
}

#[test]
fn nonresident_i30_index_allocation_does_not_duplicate_existing_parent_edge() {
    let mut source = FakeStreamSource::default().with_bytes(
        0x80_000,
        &index_allocation_record(
            0,
            vec![
                index_allocation_entry(file_reference(6, 6), file_reference(5, 5), "large.bin", 0),
                index_allocation_last_entry(false),
            ],
        ),
    );
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    RECORD_SIZE as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let child = NtfsParsedRecord::parse_mft_record(
        6,
        &mft_record(
            6,
            true,
            false,
            vec![
                file_name_attr(5, "large.bin", 0),
                resident_data_attr(b"abc"),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory, child],
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
    );

    let index = MftIndex::from_record_set(record_set);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 3);
    assert!(
        !summary
            .caveats
            .iter()
            .any(|caveat| caveat.code == "directory-index-parent-map-fallback"),
        "{:?}",
        summary.caveats
    );
}

#[test]
fn invalid_nonresident_i30_index_allocation_caveat_surfaces_in_subtree() {
    let mut source = FakeStreamSource::default().with_bytes(0x80_000, b"not-indx");
    let directory = NtfsParsedRecord::parse_mft_record(
        5,
        &mft_record(
            5,
            true,
            true,
            vec![
                file_name_attr(5, "large-dir", FILE_ATTRIBUTE_DIRECTORY),
                empty_index_root_attr_with_child_vcn(RECORD_SIZE as u32, 0),
                nonresident_named_attr(
                    ATTR_INDEX_ALLOCATION,
                    "$I30",
                    RECORD_SIZE as u64,
                    0,
                    &[0x21, 0x01, 0x80, 0x00, 0x00],
                ),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();
    let record_set = NtfsRecordSet::resolve_with_stream_source(
        vec![directory],
        NtfsStreamGeometry::new(4096, SECTOR_SIZE),
        &mut source,
    );

    let index = MftIndex::from_record_set(record_set);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 0);
    assert!(
        summary
            .caveats
            .iter()
            .any(|caveat| caveat.code == "invalid-index-allocation"),
        "{:?}",
        summary.caveats
    );
}

#[test]
fn nonresident_attribute_list_is_preserved_as_attribute_stream_and_caveated() {
    let record = NtfsParsedRecord::parse_mft_record(
        16,
        &mft_record(
            16,
            true,
            false,
            vec![
                file_name_attr(5, "listed-nonresident.bin", 0),
                nonresident_attribute_list_attr(),
            ],
        ),
        SECTOR_SIZE,
    )
    .unwrap();

    let stream = record
        .attribute_streams
        .iter()
        .find(|stream| stream.attribute_type == AttributeType::AttributeList)
        .unwrap();
    assert_eq!(stream.lowest_vcn, Some(0));
    assert_eq!(stream.highest_vcn, Some(0));
    assert_eq!(stream.logical_size, 64);
    assert_eq!(stream.data_runs.len(), 1);
    assert!(
        record
            .caveats
            .iter()
            .any(|c| c.code == "nonresident-attribute-list")
    );
}

#[test]
fn runlist_stream_reader_reads_fragmented_runs_by_logical_offset() {
    let mut source = FakeStreamSource::default()
        .with_bytes(40, b"abcd")
        .with_bytes(80, b"EFGH");
    let runs = vec![data_run(0, 1, Some(10)), data_run(1, 1, Some(20))];

    let bytes = NtfsStreamReader::new(4, SparseRunPolicy::Reject)
        .read_range(&mut source, &runs, 0, 8)
        .unwrap();

    assert_eq!(bytes, b"abcdEFGH");
}

#[test]
fn runlist_stream_reader_reads_sequential_chunks_across_fragmented_runs() {
    let mut source = FakeStreamSource::default()
        .with_bytes(40, b"abcd")
        .with_bytes(80, b"EFGH");
    let runs = vec![data_run(0, 1, Some(10)), data_run(1, 1, Some(20))];
    let mut chunks = Vec::new();

    NtfsStreamReader::new(4, SparseRunPolicy::Reject)
        .read_chunks(&mut source, &runs, 8, 4, |offset, bytes| {
            chunks.push((offset, bytes));
            true
        })
        .unwrap();

    assert_eq!(chunks, vec![(0, b"abcd".to_vec()), (4, b"EFGH".to_vec())]);
}

#[test]
fn runlist_stream_reader_handles_sparse_policy_and_gaps() {
    let mut source = FakeStreamSource::default().with_bytes(40, b"abcd");
    let runs = vec![data_run(0, 1, Some(10)), data_run(1, 1, None)];

    let bytes = NtfsStreamReader::new(4, SparseRunPolicy::ZeroFill)
        .read_range(&mut source, &runs, 0, 8)
        .unwrap();
    assert_eq!(bytes, b"abcd\0\0\0\0");

    let err = NtfsStreamReader::new(4, SparseRunPolicy::Reject)
        .read_range(&mut source, &runs, 0, 8)
        .unwrap_err();
    assert!(matches!(
        err,
        NtfsStreamReadError::SparseRun { starting_vcn: 1 }
    ));

    let err = NtfsStreamReader::new(4, SparseRunPolicy::Reject)
        .read_range(&mut source, &[data_run(1, 1, Some(20))], 0, 4)
        .unwrap_err();
    assert!(matches!(
        err,
        NtfsStreamReadError::VcnGap {
            expected_vcn: 0,
            actual_vcn: 1
        }
    ));
}

#[test]
fn runlist_stream_reader_rejects_short_source_reads() {
    let mut source = FakeStreamSource::default().with_bytes(40, b"ab");

    let err = NtfsStreamReader::new(4, SparseRunPolicy::Reject)
        .read_range(&mut source, &[data_run(0, 1, Some(10))], 0, 4)
        .unwrap_err();

    assert!(matches!(
        err,
        NtfsStreamReadError::ShortRead {
            expected: 4,
            actual: 2
        }
    ));
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
fn parent_sequence_mismatch_skips_child_edge() {
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
                    file_name_attr_with_parent_reference(file_reference(5, 99), "stale.bin", 0, 1),
                    nonresident_data_attr(10),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let summary = index.aggregate_subtree(5);

    assert_eq!(summary.bytes, 0);
    assert!(
        summary
            .caveats
            .iter()
            .any(|c| c.code == "parent-sequence-mismatch")
    );
}

#[test]
fn hardlink_path_candidates_are_preserved_and_counted_once() {
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
            9,
            mft_record(
                9,
                true,
                true,
                vec![file_name_attr(9, "other", FILE_ATTRIBUTE_DIRECTORY)],
            ),
        ),
        (
            10,
            mft_record(
                10,
                true,
                false,
                vec![
                    file_name_attr(5, "a.bin", 0),
                    file_name_attr(9, "b.bin", 0),
                    nonresident_data_attr(11),
                ],
            ),
        ),
    ]
    .into_iter()
    .map(|(id, raw)| NtfsParsedRecord::parse_mft_record(id, &raw, SECTOR_SIZE).unwrap());

    let index = MftIndex::from_parsed_records(records);
    let entry = index.get(10).unwrap();

    assert_eq!(entry.path_candidates.len(), 2);
    assert_eq!(index.aggregate_subtree(5).bytes, 11);
    assert_eq!(index.aggregate_subtree(9).bytes, 11);
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
            .any(|c| c.code == "hardlink-path-candidates")
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
    file_name_attr_with_parent_reference(
        file_reference(parent_id, parent_id as u16),
        name,
        file_attributes,
        namespace,
    )
}

fn file_name_attr_with_parent_reference(
    parent_reference: u64,
    name: &str,
    file_attributes: u32,
    namespace: u8,
) -> Vec<u8> {
    resident_attr(
        ATTR_FILE_NAME,
        &file_name_value(parent_reference, name, file_attributes, namespace),
    )
}

fn file_name_value(
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
    value
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

fn index_root_attr(
    child_reference: u64,
    parent_reference: u64,
    name: &str,
    file_attributes: u32,
) -> Vec<u8> {
    let file_name = file_name_value(parent_reference, name, file_attributes, 1);
    let entry_len = align8(16 + file_name.len());
    let end_entry_len = 16;
    let entries_offset = 16;
    let entries_start = 16 + entries_offset;
    let entries_size = entry_len + end_entry_len;
    let mut value = vec![0_u8; entries_start + entries_size];

    put_u32(&mut value, 0, ATTR_FILE_NAME);
    put_u32(&mut value, 16, entries_offset as u32);
    put_u32(&mut value, 20, (entries_offset + entries_size) as u32);
    put_u32(&mut value, 24, (entries_offset + entries_size) as u32);

    put_u64(&mut value, entries_start, child_reference);
    put_u16(&mut value, entries_start + 8, entry_len as u16);
    put_u16(&mut value, entries_start + 10, file_name.len() as u16);
    value[entries_start + 16..entries_start + 16 + file_name.len()].copy_from_slice(&file_name);

    let end_offset = entries_start + entry_len;
    put_u16(&mut value, end_offset + 8, end_entry_len as u16);
    put_u16(&mut value, end_offset + 12, 0x0002);

    resident_named_attr(ATTR_INDEX_ROOT, "$I30", &value)
}

fn empty_index_root_attr_with_child_vcn(index_record_size: u32, child_vcn: u64) -> Vec<u8> {
    empty_index_root_attr_with_child_vcn_option(index_record_size, Some(child_vcn))
}

fn empty_index_root_attr_with_child_vcn_option(
    index_record_size: u32,
    child_vcn: Option<u64>,
) -> Vec<u8> {
    let entries_offset = 16;
    let end_entry_len = if child_vcn.is_some() { 24 } else { 16 };
    let entries_size = end_entry_len;
    let mut value = vec![0_u8; 16 + entries_offset + entries_size];

    put_u32(&mut value, 0, ATTR_FILE_NAME);
    put_u32(&mut value, 8, index_record_size);
    put_u32(&mut value, 16, entries_offset as u32);
    put_u32(&mut value, 20, (entries_offset + entries_size) as u32);
    put_u32(&mut value, 24, (entries_offset + entries_size) as u32);

    let end_offset = 16 + entries_offset;
    put_u16(&mut value, end_offset + 8, end_entry_len as u16);
    put_u16(
        &mut value,
        end_offset + 12,
        if child_vcn.is_some() {
            0x0001 | 0x0002
        } else {
            0x0002
        },
    );
    if let Some(child_vcn) = child_vcn {
        put_u64(&mut value, end_offset + end_entry_len - 8, child_vcn);
    }

    resident_named_attr(ATTR_INDEX_ROOT, "$I30", &value)
}

fn index_allocation_record(vcn: u64, entries: Vec<Vec<u8>>) -> Vec<u8> {
    let mut raw_entries = Vec::new();
    for entry in entries {
        raw_entries.extend_from_slice(&entry);
    }
    index_allocation_record_with_raw_entries(vcn, raw_entries)
}

fn index_allocation_record_with_raw_entries(vcn: u64, entries: Vec<u8>) -> Vec<u8> {
    let mut record = vec![0_u8; RECORD_SIZE];
    let usa_offset = 0x28;
    let index_header_offset = 0x18;
    let entries_offset = 0x20;
    let entries_start = index_header_offset + entries_offset;
    let index_size = entries_offset + entries.len();

    record[0..4].copy_from_slice(b"INDX");
    put_u16(&mut record, 4, usa_offset as u16);
    put_u16(&mut record, 6, 3);
    put_u64(&mut record, 16, vcn);
    put_u32(&mut record, index_header_offset, entries_offset as u32);
    put_u32(&mut record, index_header_offset + 4, index_size as u32);
    put_u32(&mut record, index_header_offset + 8, index_size as u32);
    record[entries_start..entries_start + entries.len()].copy_from_slice(&entries);
    apply_test_fixup_at(&mut record, usa_offset);
    record
}

fn index_allocation_entry(
    child_reference: u64,
    parent_reference: u64,
    name: &str,
    file_attributes: u32,
) -> Vec<u8> {
    index_allocation_entry_with_child_vcn(
        child_reference,
        parent_reference,
        name,
        file_attributes,
        None,
    )
}

fn index_allocation_entry_with_child_vcn(
    child_reference: u64,
    parent_reference: u64,
    name: &str,
    file_attributes: u32,
    child_vcn: Option<u64>,
) -> Vec<u8> {
    let file_name = file_name_value(parent_reference, name, file_attributes, 1);
    let entry_len = align8(16 + file_name.len() + child_vcn.map_or(0, |_| 8));
    let mut entry = vec![0_u8; entry_len];
    put_u64(&mut entry, 0, child_reference);
    put_u16(&mut entry, 8, entry_len as u16);
    put_u16(&mut entry, 10, file_name.len() as u16);
    if let Some(child_vcn) = child_vcn {
        put_u16(&mut entry, 12, 0x0001);
        put_u64(&mut entry, entry_len - 8, child_vcn);
    }
    entry[16..16 + file_name.len()].copy_from_slice(&file_name);
    entry
}

fn index_allocation_last_entry(with_subnode: bool) -> Vec<u8> {
    if with_subnode {
        return index_allocation_last_entry_with_child_vcn(16);
    }

    let mut entry = vec![0_u8; 16];
    put_u16(&mut entry, 8, 16);
    put_u16(&mut entry, 12, 0x0002);
    entry
}

fn index_allocation_last_entry_with_child_vcn(child_vcn: u64) -> Vec<u8> {
    let entry_len = 24;
    let mut entry = vec![0_u8; entry_len];
    put_u16(&mut entry, 8, entry_len as u16);
    put_u16(&mut entry, 12, 0x0001 | 0x0002);
    put_u64(&mut entry, entry_len - 8, child_vcn);
    entry
}

fn index_allocation_attr() -> Vec<u8> {
    nonresident_named_attr(ATTR_INDEX_ALLOCATION, "$I30", 0, 0, &[0x00])
}

fn nonresident_attribute_list_attr() -> Vec<u8> {
    nonresident_named_attr(ATTR_ATTRIBUTE_LIST, "", 64, 0, &[0x11, 0x01, 0x20, 0x00])
}

fn data_run(starting_vcn: u64, cluster_count: u64, lcn: Option<u64>) -> NtfsDataRun {
    NtfsDataRun {
        starting_vcn,
        cluster_count,
        lcn,
    }
}

#[derive(Default)]
struct FakeStreamSource {
    bytes: BTreeMap<u64, u8>,
}

impl FakeStreamSource {
    fn with_bytes(mut self, offset: u64, bytes: &[u8]) -> Self {
        for (index, byte) in bytes.iter().copied().enumerate() {
            self.bytes.insert(offset + index as u64, byte);
        }
        self
    }
}

impl NtfsStreamSource for FakeStreamSource {
    type Error = &'static str;

    fn read_bytes_at(
        &mut self,
        volume_offset: u64,
        len: usize,
    ) -> std::result::Result<Vec<u8>, Self::Error> {
        let mut bytes = Vec::new();
        for index in 0..len {
            let Some(byte) = self.bytes.get(&(volume_offset + index as u64)) else {
                break;
            };
            bytes.push(*byte);
        }
        Ok(bytes)
    }
}

fn nonresident_named_attr(
    attribute_type: u32,
    name: &str,
    data_size: u64,
    lowest_vcn: u64,
    runlist: &[u8],
) -> Vec<u8> {
    let header_len = 64;
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let name_len = name_utf16.len() * 2;
    let runlist_offset = align8(header_len + name_len);
    let total_len = align8(runlist_offset + runlist.len());
    let mut attr = vec![0_u8; total_len];
    put_u32(&mut attr, 0, attribute_type);
    put_u32(&mut attr, 4, total_len as u32);
    attr[8] = 1;
    attr[9] = name_utf16.len() as u8;
    put_u16(&mut attr, 10, header_len as u16);
    put_u64(&mut attr, 16, lowest_vcn);
    put_u64(&mut attr, 24, 0);
    put_u16(&mut attr, 32, runlist_offset as u16);
    put_u64(&mut attr, 40, data_size);
    put_u64(&mut attr, 48, data_size);
    put_u64(&mut attr, 56, data_size);
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut attr, header_len + (index * 2), *character);
    }
    attr[runlist_offset..runlist_offset + runlist.len()].copy_from_slice(runlist);
    attr
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
    apply_test_fixup_at(record, USA_OFFSET);
}

fn apply_test_fixup_at(record: &mut [u8], usa_offset: usize) {
    let usn = 0xAAAA_u16;
    let first_tail = u16::from_le_bytes([record[SECTOR_SIZE - 2], record[SECTOR_SIZE - 1]]);
    let second_tail = u16::from_le_bytes([record[RECORD_SIZE - 2], record[RECORD_SIZE - 1]]);
    put_u16(record, usa_offset, usn);
    put_u16(record, usa_offset + 2, first_tail);
    put_u16(record, usa_offset + 4, second_tail);
    put_u16(record, SECTOR_SIZE - 2, usn);
    put_u16(record, RECORD_SIZE - 2, usn);
}

fn mark_test_fixup_as_already_applied(record: &mut [u8]) {
    let first_replacement = u16::from_le_bytes([record[USA_OFFSET + 2], record[USA_OFFSET + 3]]);
    let second_replacement = u16::from_le_bytes([record[USA_OFFSET + 4], record[USA_OFFSET + 5]]);
    put_u16(record, SECTOR_SIZE - 2, first_replacement);
    put_u16(record, RECORD_SIZE - 2, second_replacement);
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
