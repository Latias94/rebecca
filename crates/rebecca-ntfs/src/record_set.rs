use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adapter::{
    NtfsAttributeStream, NtfsDirectoryEntry, NtfsFileReference, NtfsParsedRecord,
    merge_attribute_stream,
};
use crate::attrs::AttributeType;
use crate::dir_index::parse_i30_index_allocation_record;
use crate::record::ParseCaveat;
use crate::stream::{NtfsStreamGeometry, NtfsStreamReader, NtfsStreamSource, SparseRunPolicy};

const INDEX_ALLOCATION_FLAG_COMPRESSED: u16 = 0x0001;
const INDEX_ALLOCATION_FLAG_ENCRYPTED: u16 = 0x4000;
const INDEX_ALLOCATION_FLAG_SPARSE: u16 = 0x8000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsRecordSet {
    pub records: Vec<NtfsParsedRecord>,
    pub caveats: Vec<ParseCaveat>,
}

impl NtfsRecordSet {
    pub fn resolve_attribute_lists(records: Vec<NtfsParsedRecord>) -> Self {
        let record_positions = records
            .iter()
            .enumerate()
            .map(|(index, record)| (record.reference, index))
            .collect::<BTreeMap<_, _>>();
        let record_ids = records
            .iter()
            .enumerate()
            .map(|(index, record)| (record.reference.record_id, index))
            .collect::<BTreeMap<_, _>>();
        let extension_references = records
            .iter()
            .filter_map(|record| record.base_reference.map(|_| record.reference))
            .collect::<BTreeSet<_>>();

        let mut resolved = Vec::new();
        let mut caveats = Vec::new();

        for record in &records {
            if record.base_reference.is_some() {
                continue;
            }

            let mut record = record.clone();
            let mut unresolved_extension = false;
            let entries = record.attribute_list_entries.clone();
            for entry in entries {
                if entry.file_reference == record.reference
                    || entry.attribute_type == AttributeType::AttributeList
                {
                    continue;
                }

                let Some(extension) = find_extension_record(
                    &records,
                    &record_positions,
                    &record_ids,
                    entry.file_reference,
                ) else {
                    unresolved_extension = true;
                    record.caveats.push(ParseCaveat::new(
                        "attribute-list-extension-record-missing",
                        format!(
                            "attribute list references missing extension record {}",
                            entry.file_reference.record_id
                        ),
                    ));
                    continue;
                };

                if extension.base_reference != Some(record.reference) {
                    unresolved_extension = true;
                    record.caveats.push(ParseCaveat::new(
                        "attribute-list-extension-base-mismatch",
                        format!(
                            "extension record {} does not point back to base record {}",
                            extension.reference.record_id, record.reference.record_id
                        ),
                    ));
                    continue;
                }

                match entry.attribute_type {
                    AttributeType::Data | AttributeType::IndexAllocation => {
                        let mut matched = false;
                        for stream in extension.attribute_streams.iter().filter(|stream| {
                            stream.attribute_type == entry.attribute_type
                                && stream.attribute_id == entry.attribute_id
                                && stream.name == entry.name
                                && stream.lowest_vcn == Some(entry.lowest_vcn)
                        }) {
                            merge_attribute_stream(&mut record.attribute_streams, stream.clone());
                            matched = true;
                        }
                        if !matched {
                            unresolved_extension = true;
                            record.caveats.push(ParseCaveat::new(
                                "attribute-list-extension-attribute-missing",
                                format!(
                                    "extension record {} does not contain attribute id {}",
                                    extension.reference.record_id, entry.attribute_id
                                ),
                            ));
                        }
                    }
                    other => {
                        unresolved_extension = true;
                        record.caveats.push(ParseCaveat::new(
                            "attribute-list-extension-attribute-unsupported",
                            format!(
                                "attribute-list expansion does not yet merge {other:?} attributes"
                            ),
                        ));
                    }
                }
            }
            if !unresolved_extension {
                record
                    .caveats
                    .retain(|caveat| caveat.code != "attribute-list-extension-records-unexpanded");
            }
            resolved.push(record);
        }

        for extension_reference in extension_references {
            caveats.push(ParseCaveat::new(
                "attribute-list-extension-record-skipped",
                format!(
                    "extension record {} was skipped as a standalone index entry",
                    extension_reference.record_id
                ),
            ));
        }

        Self {
            records: resolved,
            caveats,
        }
    }

    pub fn resolve_with_stream_source<S>(
        records: Vec<NtfsParsedRecord>,
        geometry: NtfsStreamGeometry,
        source: &mut S,
    ) -> Self
    where
        S: NtfsStreamSource,
    {
        let mut record_set = Self::resolve_attribute_lists(records);
        record_set.expand_index_allocations(geometry, source);
        record_set
    }

    fn expand_index_allocations<S>(&mut self, geometry: NtfsStreamGeometry, source: &mut S)
    where
        S: NtfsStreamSource,
    {
        for record in &mut self.records {
            expand_record_index_allocations(record, geometry, source);
        }
    }
}

fn expand_record_index_allocations<S>(
    record: &mut NtfsParsedRecord,
    geometry: NtfsStreamGeometry,
    source: &mut S,
) where
    S: NtfsStreamSource,
{
    let streams = record
        .attribute_streams
        .iter()
        .filter(|stream| {
            stream.attribute_type == AttributeType::IndexAllocation
                && stream.name.as_deref() == Some("$I30")
        })
        .cloned()
        .collect::<Vec<_>>();
    if streams.is_empty() {
        return;
    }

    let Some(directory_index) = record
        .directory_indexes
        .iter()
        .find(|index| index.name == "$I30" && index.indexed_attribute == AttributeType::FileName)
    else {
        record.caveats.push(ParseCaveat::new(
            "index-allocation-root-missing",
            format!(
                "record {} has $INDEX_ALLOCATION:$I30 but no resident $INDEX_ROOT:$I30 metadata",
                record.reference.record_id
            ),
        ));
        return;
    };
    let Ok(index_record_size) = usize::try_from(directory_index.index_record_size) else {
        record.caveats.push(invalid_index_allocation_caveat(
            record.reference.record_id,
            "index record size does not fit in memory",
        ));
        return;
    };
    if index_record_size == 0 {
        record.caveats.push(invalid_index_allocation_caveat(
            record.reference.record_id,
            "index record size is zero",
        ));
        return;
    }
    if geometry.bytes_per_cluster == 0 || geometry.bytes_per_sector < 2 {
        record.caveats.push(invalid_index_allocation_caveat(
            record.reference.record_id,
            "stream geometry is invalid",
        ));
        return;
    }

    let reader = NtfsStreamReader::new(geometry.bytes_per_cluster, SparseRunPolicy::Reject);
    for stream in streams {
        expand_index_allocation_stream(
            record,
            &reader,
            geometry,
            index_record_size,
            &stream,
            source,
        );
    }
}

fn expand_index_allocation_stream<S>(
    record: &mut NtfsParsedRecord,
    reader: &NtfsStreamReader,
    geometry: NtfsStreamGeometry,
    index_record_size: usize,
    stream: &NtfsAttributeStream,
    source: &mut S,
) where
    S: NtfsStreamSource,
{
    if (stream.flags & (INDEX_ALLOCATION_FLAG_COMPRESSED | INDEX_ALLOCATION_FLAG_ENCRYPTED)) != 0 {
        record.caveats.push(ParseCaveat::new(
            "unsupported-index-allocation",
            format!(
                "record {} has compressed or encrypted $INDEX_ALLOCATION:$I30",
                record.reference.record_id
            ),
        ));
        return;
    }
    if (stream.flags & INDEX_ALLOCATION_FLAG_SPARSE) != 0 {
        record.caveats.push(ParseCaveat::new(
            "unsupported-index-allocation",
            format!(
                "record {} has sparse $INDEX_ALLOCATION:$I30",
                record.reference.record_id
            ),
        ));
        return;
    }
    if stream.logical_size == 0 {
        return;
    }

    let Ok(index_record_size_u64) = u64::try_from(index_record_size) else {
        record.caveats.push(invalid_index_allocation_caveat(
            record.reference.record_id,
            "index record size does not fit in u64",
        ));
        return;
    };

    if !stream.logical_size.is_multiple_of(index_record_size_u64) {
        record.caveats.push(invalid_index_allocation_caveat(
            record.reference.record_id,
            "index allocation stream has a partial trailing INDX record",
        ));
        return;
    }

    let record_id = record.reference.record_id;
    let mut entries = std::mem::take(&mut record.directory_entries);
    let mut seen_entries = directory_entry_set(&entries);
    let mut parse_error = None;
    let read_result = reader.read_chunks(
        source,
        &stream.data_runs,
        stream.logical_size,
        index_record_size,
        |logical_offset, raw_record| {
            let expected_vcn = logical_offset / geometry.bytes_per_cluster;
            match parse_i30_index_allocation_record(
                &raw_record,
                geometry.bytes_per_sector,
                expected_vcn,
            ) {
                Ok(parsed_record) => {
                    let parsed_entries = parsed_record.directory_entries().collect();
                    append_unique_directory_entries(&mut entries, &mut seen_entries, parsed_entries)
                }
                Err(err) => {
                    parse_error = Some(format!(
                        "INDX record at logical offset {logical_offset} is invalid: {err}"
                    ));
                    return false;
                }
            }
            true
        },
    );
    record.directory_entries = entries;

    if let Some(reason) = parse_error {
        record
            .caveats
            .push(invalid_index_allocation_caveat(record_id, reason));
        return;
    }
    if let Err(err) = read_result {
        record.caveats.push(invalid_index_allocation_caveat(
            record_id,
            format!("stream read failed: {err}"),
        ));
    }
}

fn append_unique_directory_entries(
    existing: &mut Vec<NtfsDirectoryEntry>,
    seen: &mut BTreeSet<NtfsDirectoryEntryKey>,
    incoming: Vec<NtfsDirectoryEntry>,
) {
    for entry in incoming {
        if seen.insert(NtfsDirectoryEntryKey::from(&entry)) {
            existing.push(entry);
        }
    }
}

fn directory_entry_set(entries: &[NtfsDirectoryEntry]) -> BTreeSet<NtfsDirectoryEntryKey> {
    entries.iter().map(NtfsDirectoryEntryKey::from).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NtfsDirectoryEntryKey {
    child: NtfsFileReference,
    parent: NtfsFileReference,
    namespace: crate::record::FileNameNamespace,
    name: String,
    file_attributes: u32,
}

impl From<&NtfsDirectoryEntry> for NtfsDirectoryEntryKey {
    fn from(entry: &NtfsDirectoryEntry) -> Self {
        Self {
            child: entry.child,
            parent: entry.parent,
            namespace: entry.namespace,
            name: entry.name.clone(),
            file_attributes: entry.file_attributes,
        }
    }
}

fn invalid_index_allocation_caveat(record_id: u64, reason: impl Into<String>) -> ParseCaveat {
    ParseCaveat::new(
        "invalid-index-allocation",
        format!(
            "record {record_id} $INDEX_ALLOCATION:$I30 could not be parsed: {}",
            reason.into()
        ),
    )
}

fn find_extension_record<'a>(
    records: &'a [NtfsParsedRecord],
    record_positions: &BTreeMap<NtfsFileReference, usize>,
    record_ids: &BTreeMap<u64, usize>,
    reference: NtfsFileReference,
) -> Option<&'a NtfsParsedRecord> {
    record_positions
        .get(&reference)
        .or_else(|| record_ids.get(&reference.record_id))
        .and_then(|index| records.get(*index))
}
