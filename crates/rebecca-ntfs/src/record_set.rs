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

    let mut logical_offset = 0_u64;
    while logical_offset < stream.logical_size {
        let remaining = stream.logical_size - logical_offset;
        if remaining < index_record_size_u64 {
            record.caveats.push(invalid_index_allocation_caveat(
                record.reference.record_id,
                "index allocation stream has a partial trailing INDX record",
            ));
            return;
        }

        let raw_record =
            match reader.read_range(source, &stream.data_runs, logical_offset, index_record_size) {
                Ok(raw_record) => raw_record,
                Err(err) => {
                    record.caveats.push(invalid_index_allocation_caveat(
                        record.reference.record_id,
                        format!("stream read failed: {err}"),
                    ));
                    return;
                }
            };
        let expected_vcn = logical_offset / geometry.bytes_per_cluster;
        match parse_i30_index_allocation_record(
            &raw_record,
            geometry.bytes_per_sector,
            expected_vcn,
        ) {
            Ok(entries) => append_unique_directory_entries(&mut record.directory_entries, entries),
            Err(err) => {
                record.caveats.push(invalid_index_allocation_caveat(
                    record.reference.record_id,
                    format!("INDX record at logical offset {logical_offset} is invalid: {err}"),
                ));
                return;
            }
        }

        logical_offset = match logical_offset.checked_add(index_record_size_u64) {
            Some(next) => next,
            None => {
                record.caveats.push(invalid_index_allocation_caveat(
                    record.reference.record_id,
                    "logical offset overflowed while reading index allocation",
                ));
                return;
            }
        };
    }
}

fn append_unique_directory_entries(
    existing: &mut Vec<NtfsDirectoryEntry>,
    incoming: Vec<NtfsDirectoryEntry>,
) {
    for entry in incoming {
        if !existing.contains(&entry) {
            existing.push(entry);
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
