use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rebecca_ntfs::{
    MftIndex, MftRecordReader, NtfsDataRun, NtfsRecordSet, NtfsStreamGeometry, NtfsStreamReader,
    NtfsStreamSource, SparseRunPolicy,
};

const RECORD_SIZE: usize = 1024;
const SECTOR_SIZE: usize = 512;
const BYTES_PER_CLUSTER: u64 = 4096;
const USA_OFFSET: usize = 0x30;
const FIRST_ATTR_OFFSET: usize = 0x38;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_INDEX_ROOT: u32 = 0x90;
const ATTR_INDEX_ALLOCATION: u32 = 0xA0;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const INDEX_ALLOCATION_RECORD_SIZE: usize = 4096;

fn mft_parser(criterion: &mut Criterion) {
    let fixture = build_fixture(1024);
    let tree_fixture = build_tree_fixture(1024);
    let (index_allocation_fixture, index_allocation_source) =
        build_index_allocation_tree_fixture(1024);
    let fragmented_runs = fragmented_runs(1024);
    let fragmented_source = FragmentedStreamSource::from_runs(&fragmented_runs);
    let reader = MftRecordReader::default();
    let stream_reader = NtfsStreamReader::new(BYTES_PER_CLUSTER, SparseRunPolicy::Reject);

    criterion.bench_function("parse_generated_1024_mft_records", |bencher| {
        bencher.iter(|| {
            let batch = reader.parse_records(black_box(&fixture));
            assert_eq!(batch.records.len(), 1024);
            assert!(batch.errors.is_empty());
            black_box(batch);
        });
    });

    criterion.bench_function("parse_and_index_generated_1024_mft_files", |bencher| {
        bencher.iter(|| {
            let batch = reader.parse_records_from(5, black_box(&tree_fixture));
            assert_eq!(batch.records.len(), 1025);
            assert!(batch.errors.is_empty());
            let index = MftIndex::from_parsed_records(batch.records);
            let summary = index.aggregate_subtree(5);
            assert_eq!(summary.files, 1024);
            assert_eq!(summary.bytes, 1024 * 128);
            black_box(summary);
        });
    });

    criterion.bench_function(
        "parse_expand_and_index_generated_1024_mft_files_from_i30_allocation",
        |bencher| {
            bencher.iter(|| {
                let batch = reader.parse_records_from(5, black_box(&index_allocation_fixture));
                assert_eq!(batch.records.len(), 1025);
                assert!(batch.errors.is_empty());
                let mut source = index_allocation_source.clone();
                let record_set = NtfsRecordSet::resolve_with_stream_source(
                    batch.records,
                    NtfsStreamGeometry::new(BYTES_PER_CLUSTER, SECTOR_SIZE),
                    &mut source,
                );
                let index = MftIndex::from_record_set(record_set);
                let summary = index.aggregate_subtree(5);
                assert_eq!(summary.files, 1024);
                assert_eq!(summary.bytes, 1024 * 128);
                black_box(summary);
            });
        },
    );

    criterion.bench_function("read_fragmented_runlist_1024_clusters", |bencher| {
        bencher.iter(|| {
            let mut source = fragmented_source.clone();
            let bytes = stream_reader
                .read_range(
                    &mut source,
                    black_box(&fragmented_runs),
                    0,
                    1024 * BYTES_PER_CLUSTER as usize,
                )
                .expect("fragmented stream should read");
            assert_eq!(bytes.len(), 1024 * BYTES_PER_CLUSTER as usize);
            black_box(bytes);
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

fn build_tree_fixture(file_records: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((file_records + 1) * RECORD_SIZE);
    bytes.extend_from_slice(&mft_record_with_flags(
        5,
        0x0003,
        vec![file_name_attr_with_file_attributes(
            5,
            "root",
            FILE_ATTRIBUTE_DIRECTORY,
        )],
    ));
    for index in 0..file_records {
        let record_id = 6 + index as u64;
        bytes.extend_from_slice(&mft_record(
            record_id,
            vec![
                file_name_attr(5, &format!("file-{index:04}.bin")),
                nonresident_data_attr(128),
            ],
        ));
    }
    bytes
}

fn build_index_allocation_tree_fixture(file_records: usize) -> (Vec<u8>, InMemoryStreamSource) {
    let index_records = build_index_allocation_records(file_records);
    let cluster_count = index_records.len() / INDEX_ALLOCATION_RECORD_SIZE;
    let runlist = [0x21, cluster_count as u8, 0x80, 0x00, 0x00];
    let mut bytes = Vec::with_capacity((file_records + 1) * RECORD_SIZE);
    bytes.extend_from_slice(&mft_record_with_flags(
        5,
        0x0003,
        vec![
            file_name_attr_with_file_attributes(5, "root", FILE_ATTRIBUTE_DIRECTORY),
            empty_index_root_attr(INDEX_ALLOCATION_RECORD_SIZE as u32),
            nonresident_named_attr(
                ATTR_INDEX_ALLOCATION,
                "$I30",
                index_records.len() as u64,
                &runlist,
            ),
        ],
    ));
    for index in 0..file_records {
        let record_id = 6 + index as u64;
        bytes.extend_from_slice(&mft_record(
            record_id,
            vec![
                file_name_attr(99, &format!("file-{index:04}.bin")),
                nonresident_data_attr(128),
            ],
        ));
    }

    (
        bytes,
        InMemoryStreamSource::default().with_bytes(0x80 * BYTES_PER_CLUSTER, &index_records),
    )
}

fn build_index_allocation_records(file_records: usize) -> Vec<u8> {
    let mut records = Vec::new();
    let entries_per_record = 32;
    for (record_index, chunk) in (0..file_records)
        .collect::<Vec<_>>()
        .chunks(entries_per_record)
        .enumerate()
    {
        let mut entries = Vec::new();
        for file_index in chunk {
            let record_id = 6 + *file_index as u64;
            entries.push(index_allocation_entry(
                file_reference(record_id, record_id as u16),
                file_reference(5, 5),
                &format!("file-{file_index:04}.bin"),
                0,
            ));
        }
        entries.push(index_allocation_last_entry());
        records.extend_from_slice(&index_allocation_record(record_index as u64, entries));
    }
    records
}

fn mft_record(record_id: u64, attrs: Vec<Vec<u8>>) -> Vec<u8> {
    mft_record_with_flags(record_id, 0x0001, attrs)
}

fn mft_record_with_flags(record_id: u64, record_flags: u16, attrs: Vec<Vec<u8>>) -> Vec<u8> {
    let mut record = vec![0_u8; RECORD_SIZE];
    record[0..4].copy_from_slice(b"FILE");
    put_u16(&mut record, 4, USA_OFFSET as u16);
    put_u16(&mut record, 6, 3);
    put_u16(&mut record, 16, record_id as u16);
    put_u16(&mut record, 20, FIRST_ATTR_OFFSET as u16);
    put_u16(&mut record, 22, record_flags);
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
    file_name_attr_with_file_attributes(parent_id, name, 0)
}

fn file_name_attr_with_file_attributes(
    parent_id: u64,
    name: &str,
    file_attributes: u32,
) -> Vec<u8> {
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let mut value = vec![0_u8; 66 + (name_utf16.len() * 2)];
    put_u64(&mut value, 0, parent_id);
    put_u32(&mut value, 56, file_attributes);
    value[64] = name_utf16.len() as u8;
    value[65] = 1;
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut value, 66 + (index * 2), *character);
    }
    resident_attr(ATTR_FILE_NAME, &value)
}

fn empty_index_root_attr(index_record_size: u32) -> Vec<u8> {
    let entries_offset = 16;
    let end_entry_len = 16;
    let entries_size = end_entry_len;
    let mut value = vec![0_u8; 16 + entries_offset + entries_size];

    put_u32(&mut value, 0, ATTR_FILE_NAME);
    put_u32(&mut value, 8, index_record_size);
    put_u32(&mut value, 16, entries_offset as u32);
    put_u32(&mut value, 20, entries_size as u32);
    put_u32(&mut value, 24, entries_size as u32);

    let end_offset = 16 + entries_offset;
    put_u16(&mut value, end_offset + 8, end_entry_len as u16);
    put_u16(&mut value, end_offset + 12, 0x0002);

    resident_named_attr(ATTR_INDEX_ROOT, "$I30", &value)
}

fn index_allocation_record(vcn: u64, entries: Vec<Vec<u8>>) -> Vec<u8> {
    let mut raw_entries = Vec::new();
    for entry in entries {
        raw_entries.extend_from_slice(&entry);
    }

    let mut record = vec![0_u8; INDEX_ALLOCATION_RECORD_SIZE];
    let usa_offset = 0x28;
    let index_header_offset = 0x18;
    let entries_offset = 0x40;
    let entries_start = index_header_offset + entries_offset;
    let index_size = entries_offset + raw_entries.len();

    record[0..4].copy_from_slice(b"INDX");
    put_u16(&mut record, 4, usa_offset as u16);
    put_u16(
        &mut record,
        6,
        (INDEX_ALLOCATION_RECORD_SIZE / SECTOR_SIZE + 1) as u16,
    );
    put_u64(&mut record, 16, vcn);
    put_u32(&mut record, index_header_offset, entries_offset as u32);
    put_u32(&mut record, index_header_offset + 4, index_size as u32);
    put_u32(&mut record, index_header_offset + 8, index_size as u32);
    record[entries_start..entries_start + raw_entries.len()].copy_from_slice(&raw_entries);
    apply_test_fixup_at(&mut record, usa_offset, SECTOR_SIZE);
    record
}

fn index_allocation_entry(
    child_reference: u64,
    parent_reference: u64,
    name: &str,
    file_attributes: u32,
) -> Vec<u8> {
    let file_name = file_name_value(parent_reference, name, file_attributes);
    let entry_len = align8(16 + file_name.len() + 8);
    let mut entry = vec![0_u8; entry_len];
    put_u64(&mut entry, 0, child_reference);
    put_u16(&mut entry, 8, entry_len as u16);
    put_u16(&mut entry, 10, file_name.len() as u16);
    put_u16(&mut entry, 12, 0x0001);
    entry[16..16 + file_name.len()].copy_from_slice(&file_name);
    put_u64(&mut entry, entry_len - 8, 8);
    entry
}

fn index_allocation_last_entry() -> Vec<u8> {
    let mut entry = vec![0_u8; 16];
    put_u16(&mut entry, 8, 16);
    put_u16(&mut entry, 12, 0x0002);
    entry
}

fn file_name_value(parent_reference: u64, name: &str, file_attributes: u32) -> Vec<u8> {
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let mut value = vec![0_u8; 66 + (name_utf16.len() * 2)];
    put_u64(&mut value, 0, parent_reference);
    put_u32(&mut value, 56, file_attributes);
    value[64] = name_utf16.len() as u8;
    value[65] = 1;
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut value, 66 + (index * 2), *character);
    }
    value
}

fn file_reference(record_id: u64, sequence_number: u16) -> u64 {
    ((sequence_number as u64) << 48) | (record_id & 0x0000_FFFF_FFFF_FFFF)
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

fn resident_named_attr(attribute_type: u32, name: &str, value: &[u8]) -> Vec<u8> {
    let header_len = 24;
    let name_utf16 = name.encode_utf16().collect::<Vec<_>>();
    let name_len = name_utf16.len() * 2;
    let name_offset = header_len;
    let value_offset = align8(name_offset + name_len);
    let total_len = align8(value_offset + value.len());
    let mut attr = vec![0_u8; total_len];
    put_u32(&mut attr, 0, attribute_type);
    put_u32(&mut attr, 4, total_len as u32);
    attr[9] = name_utf16.len() as u8;
    put_u16(&mut attr, 10, name_offset as u16);
    put_u32(&mut attr, 16, value.len() as u32);
    put_u16(&mut attr, 20, value_offset as u16);
    for (index, character) in name_utf16.iter().enumerate() {
        put_u16(&mut attr, name_offset + (index * 2), *character);
    }
    attr[value_offset..value_offset + value.len()].copy_from_slice(value);
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

fn nonresident_named_attr(
    attribute_type: u32,
    name: &str,
    data_size: u64,
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
    put_u64(
        &mut attr,
        24,
        data_size.saturating_sub(1) / BYTES_PER_CLUSTER,
    );
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

fn apply_test_fixup_at(record: &mut [u8], usa_offset: usize, sector_size: usize) {
    let update_sequence = 0xBBAA_u16;
    let sector_count = record.len() / sector_size;
    put_u16(record, usa_offset, update_sequence);
    for sector_index in 0..sector_count {
        let tail = ((sector_index + 1) * sector_size) - 2;
        let original = u16::from_le_bytes([record[tail], record[tail + 1]]);
        put_u16(record, usa_offset + ((sector_index + 1) * 2), original);
        put_u16(record, tail, update_sequence);
    }
}

#[derive(Clone, Default)]
struct InMemoryStreamSource {
    bytes: Vec<u8>,
    base_offset: u64,
}

impl InMemoryStreamSource {
    fn with_bytes(mut self, base_offset: u64, bytes: &[u8]) -> Self {
        self.base_offset = base_offset;
        self.bytes = bytes.to_vec();
        self
    }
}

impl NtfsStreamSource for InMemoryStreamSource {
    type Error = &'static str;

    fn read_bytes_at(
        &mut self,
        volume_offset: u64,
        len: usize,
    ) -> std::result::Result<Vec<u8>, Self::Error> {
        let start = volume_offset
            .checked_sub(self.base_offset)
            .ok_or("offset before fixture")?;
        let start = usize::try_from(start).map_err(|_| "offset overflow")?;
        let end = start.checked_add(len).ok_or("length overflow")?;
        Ok(self.bytes.get(start..end).unwrap_or(&[]).to_vec())
    }
}

fn fragmented_runs(count: usize) -> Vec<NtfsDataRun> {
    (0..count)
        .map(|index| NtfsDataRun {
            starting_vcn: index as u64,
            cluster_count: 1,
            lcn: Some(0x200 + (index as u64 * 2)),
        })
        .collect()
}

#[derive(Clone, Default)]
struct FragmentedStreamSource {
    runs: Vec<NtfsDataRun>,
}

impl FragmentedStreamSource {
    fn from_runs(runs: &[NtfsDataRun]) -> Self {
        Self {
            runs: runs.to_vec(),
        }
    }
}

impl NtfsStreamSource for FragmentedStreamSource {
    type Error = &'static str;

    fn read_bytes_at(
        &mut self,
        volume_offset: u64,
        len: usize,
    ) -> std::result::Result<Vec<u8>, Self::Error> {
        let lcn = volume_offset / BYTES_PER_CLUSTER;
        let Some(run) = self.runs.iter().find(|run| run.lcn == Some(lcn)) else {
            return Err("unknown LCN");
        };
        let fill = (run.starting_vcn % 251) as u8;
        Ok(vec![fill; len])
    }
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
