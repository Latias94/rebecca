use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adapter::{
    NtfsAttributeListEntry, NtfsAttributeStream, NtfsDirectoryEntry, NtfsFileReference,
    NtfsIndexEntry, NtfsParsedAttribute, NtfsParsedRecord, merge_attribute_stream,
};
use crate::attribute_list::parse_attribute_list;
use crate::attrs::AttributeType;
use crate::dir_index::{NtfsIndexAllocationRecord, parse_i30_index_allocation_record};
use crate::record::ParseCaveat;
use crate::stream::{
    NtfsStreamGeometry, NtfsStreamReadError, NtfsStreamReader, NtfsStreamSource, SparseRunPolicy,
};

const STREAM_FLAG_COMPRESSED: u16 = 0x0001;
const STREAM_FLAG_ENCRYPTED: u16 = 0x4000;
const STREAM_FLAG_SPARSE: u16 = 0x8000;
const MAX_ATTRIBUTE_LIST_STREAM_BYTES: u64 = 1024 * 1024;
const DATA_RUN_ALLOCATED_BY_CLUSTER_CAVEAT: &str = "mft-data-run-allocated-by-cluster";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfsRecordSet {
    pub records: Vec<NtfsParsedRecord>,
    pub caveats: Vec<ParseCaveat>,
}

impl NtfsRecordSet {
    pub fn resolve_attribute_lists(records: Vec<NtfsParsedRecord>) -> Self {
        resolve_attribute_lists_with_record_prepare(
            records,
            caveat_unresolved_attribute_list_streams,
        )
    }

    pub fn resolve_with_stream_source<S>(
        records: Vec<NtfsParsedRecord>,
        geometry: NtfsStreamGeometry,
        source: &mut S,
    ) -> Self
    where
        S: NtfsStreamSource,
    {
        let mut record_set = resolve_attribute_lists_with_record_prepare(records, |record| {
            expand_record_attribute_lists(record, geometry, source);
        });
        record_set.expand_index_allocations(geometry, source);
        record_set.apply_stream_physical_allocated_sizes(geometry);
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

    fn apply_stream_physical_allocated_sizes(&mut self, geometry: NtfsStreamGeometry) {
        for record in &mut self.records {
            apply_record_stream_physical_allocated_sizes(record, geometry);
        }
    }
}

fn resolve_attribute_lists_with_record_prepare<F>(
    records: Vec<NtfsParsedRecord>,
    mut prepare_record: F,
) -> NtfsRecordSet
where
    F: FnMut(&mut NtfsParsedRecord),
{
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
        prepare_record(&mut record);
        resolve_attribute_list_extensions(&mut record, |reference| {
            find_extension_record(&records, &record_positions, &record_ids, reference).cloned()
        });
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

    NtfsRecordSet {
        records: resolved,
        caveats,
    }
}

pub fn resolve_record_with_stream_source<S, F>(
    mut record: NtfsParsedRecord,
    geometry: NtfsStreamGeometry,
    source: &mut S,
    resolve_extension: F,
) -> std::result::Result<NtfsParsedRecord, F::Error>
where
    S: NtfsStreamSource,
    F: NtfsRecordResolver,
{
    expand_record_attribute_lists(&mut record, geometry, source);
    resolve_attribute_list_extensions_result(&mut record, resolve_extension)?;
    expand_record_index_allocations(&mut record, geometry, source);
    apply_record_stream_physical_allocated_sizes(&mut record, geometry);
    Ok(record)
}

pub trait NtfsRecordResolver {
    type Error;

    fn resolve(
        &mut self,
        reference: NtfsFileReference,
    ) -> std::result::Result<Option<NtfsParsedRecord>, Self::Error>;
}

impl<F, E> NtfsRecordResolver for F
where
    F: FnMut(NtfsFileReference) -> std::result::Result<Option<NtfsParsedRecord>, E>,
{
    type Error = E;

    fn resolve(
        &mut self,
        reference: NtfsFileReference,
    ) -> std::result::Result<Option<NtfsParsedRecord>, Self::Error> {
        self(reference)
    }
}

fn resolve_attribute_list_extensions_result<R>(
    record: &mut NtfsParsedRecord,
    mut resolve: R,
) -> std::result::Result<(), R::Error>
where
    R: NtfsRecordResolver,
{
    let entries = record.attribute_list_entries.clone();
    for entry in entries {
        if should_skip_attribute_list_entry(record.reference, &entry) {
            continue;
        }

        let Some(extension) = resolve.resolve(entry.file_reference)? else {
            record.caveats.push(ParseCaveat::new(
                "attribute-list-extension-record-missing",
                format!(
                    "attribute list references missing extension record {}",
                    entry.file_reference.record_id
                ),
            ));
            continue;
        };

        merge_attribute_list_extension(record, &entry, &extension);
    }

    Ok(())
}

fn resolve_attribute_list_extensions<F>(record: &mut NtfsParsedRecord, mut resolve: F)
where
    F: FnMut(NtfsFileReference) -> Option<NtfsParsedRecord>,
{
    let entries = record.attribute_list_entries.clone();
    for entry in entries {
        if should_skip_attribute_list_entry(record.reference, &entry) {
            continue;
        }

        let Some(extension) = resolve(entry.file_reference) else {
            record.caveats.push(ParseCaveat::new(
                "attribute-list-extension-record-missing",
                format!(
                    "attribute list references missing extension record {}",
                    entry.file_reference.record_id
                ),
            ));
            continue;
        };

        merge_attribute_list_extension(record, &entry, &extension);
    }
}

fn should_skip_attribute_list_entry(
    base_reference: NtfsFileReference,
    entry: &NtfsAttributeListEntry,
) -> bool {
    entry.file_reference == base_reference || entry.attribute_type == AttributeType::AttributeList
}

fn merge_attribute_list_extension(
    record: &mut NtfsParsedRecord,
    entry: &NtfsAttributeListEntry,
    extension: &NtfsParsedRecord,
) {
    if extension.base_reference != Some(record.reference) {
        record.caveats.push(ParseCaveat::new(
            "attribute-list-extension-base-mismatch",
            format!(
                "extension record {} does not point back to base record {}",
                extension.reference.record_id, record.reference.record_id
            ),
        ));
        return;
    }
    if extension_sequence_mismatches(entry.file_reference, extension.reference) {
        record.caveats.push(ParseCaveat::new(
            "attribute-list-extension-sequence-mismatch",
            format!(
                "attribute list references extension record {} sequence {:?}, but current sequence is {:?}",
                entry.file_reference.record_id,
                entry.file_reference.sequence_number,
                extension.reference.sequence_number
            ),
        ));
        return;
    }

    match entry.attribute_type {
        AttributeType::Data | AttributeType::IndexAllocation => {
            let mut matched = false;
            for stream in extension.attribute_streams.iter().filter(|stream| {
                stream.attribute_type == entry.attribute_type
                    && stream.attribute_id == entry.attribute_id
                    && stream.name == entry.name
                    && stream.lowest_vcn.unwrap_or(0) == entry.lowest_vcn
            }) {
                merge_attribute_stream(&mut record.attribute_streams, stream.clone());
                matched = true;
            }
            if matched {
                append_extension_attribute_metadata(record, extension, entry);
            }
            if !matched {
                push_missing_extension_attribute_caveat(record, entry, extension);
            }
        }
        AttributeType::StandardInformation => {
            if !append_extension_attribute_metadata(record, extension, entry) {
                push_missing_extension_attribute_caveat(record, entry, extension);
                return;
            }
            record.is_directory |= extension.is_directory;
            record.is_reparse_point |= extension.is_reparse_point;
        }
        AttributeType::FileName => {
            if !append_extension_attribute_metadata(record, extension, entry) {
                push_missing_extension_attribute_caveat(record, entry, extension);
                return;
            }
            let mut merged_name = false;
            for name in extension
                .names
                .iter()
                .filter(|name| file_name_matches_entry(name, entry))
            {
                append_unique(&mut record.names, name.clone());
                merged_name = true;
            }
            if !merged_name {
                push_missing_extension_attribute_caveat(record, entry, extension);
            } else {
                record
                    .caveats
                    .retain(|caveat| caveat.code != "pathless-record");
            }
        }
        AttributeType::IndexRoot => {
            let mut matched = false;
            let mut seen_entries = directory_entry_set(&record.directory_entries);
            for index in extension.directory_indexes.iter().filter(|index| {
                index.attribute_id == entry.attribute_id
                    && Some(index.name.as_str()) == entry.name.as_deref()
            }) {
                append_unique(&mut record.directory_indexes, index.clone());
                for directory_entry in &index.root_entries {
                    if let Some(directory_entry) = &directory_entry.directory_entry {
                        append_unique_directory_entry(
                            &mut record.directory_entries,
                            &mut seen_entries,
                            directory_entry.clone(),
                        );
                    }
                }
                matched = true;
            }
            if matched {
                append_extension_attribute_metadata(record, extension, entry);
            } else {
                push_missing_extension_attribute_caveat(record, entry, extension);
            }
        }
        other => {
            record.caveats.push(ParseCaveat::new(
                "attribute-list-extension-attribute-unsupported",
                format!("attribute-list expansion does not yet merge {other:?} attributes"),
            ));
        }
    }
}

fn append_extension_attribute_metadata(
    record: &mut NtfsParsedRecord,
    extension: &NtfsParsedRecord,
    entry: &NtfsAttributeListEntry,
) -> bool {
    let mut matched = false;
    for attribute in extension
        .attributes
        .iter()
        .filter(|attribute| attribute_matches_entry(attribute, entry))
    {
        append_unique(&mut record.attributes, attribute.clone());
        matched = true;
    }
    matched
}

fn attribute_matches_entry(
    attribute: &NtfsParsedAttribute,
    entry: &NtfsAttributeListEntry,
) -> bool {
    attribute.attribute_type == entry.attribute_type
        && attribute.attribute_id == entry.attribute_id
        && attribute.name == entry.name
        && attribute.lowest_vcn.unwrap_or(0) == entry.lowest_vcn
}

fn file_name_matches_entry(name: &crate::NtfsFileName, entry: &NtfsAttributeListEntry) -> bool {
    name.attribute_id == Some(entry.attribute_id)
        && name.attribute_name == entry.name
        && name.lowest_vcn.unwrap_or(0) == entry.lowest_vcn
}

fn extension_sequence_mismatches(expected: NtfsFileReference, actual: NtfsFileReference) -> bool {
    matches!(
        (expected.sequence_number, actual.sequence_number),
        (Some(expected), Some(actual)) if expected != 0 && actual != 0 && expected != actual
    )
}

fn append_unique<T: PartialEq>(items: &mut Vec<T>, incoming: T) {
    if !items.contains(&incoming) {
        items.push(incoming);
    }
}

fn caveat_unresolved_attribute_list_streams(record: &mut NtfsParsedRecord) {
    if record
        .attribute_streams
        .iter()
        .any(|stream| stream.attribute_type == AttributeType::AttributeList && stream.non_resident)
    {
        record.caveats.push(ParseCaveat::new(
            "nonresident-attribute-list-unresolved",
            format!(
                "record {} has nonresident $ATTRIBUTE_LIST but no stream source was provided",
                record.reference.record_id
            ),
        ));
    }
}

fn apply_record_stream_physical_allocated_sizes(
    record: &mut NtfsParsedRecord,
    geometry: NtfsStreamGeometry,
) {
    let mut caveats = Vec::new();
    for stream in &mut record.attribute_streams {
        let original_allocated_size = stream.allocated_size;
        let Some(allocated_size) = stream_physical_allocated_size(stream, geometry) else {
            continue;
        };
        stream.allocated_size = Some(allocated_size);
        if stream.attribute_type == AttributeType::Data
            && stream.name.is_none()
            && original_allocated_size != Some(allocated_size)
        {
            caveats.push(data_run_allocated_by_cluster_caveat(
                record.reference.record_id,
                original_allocated_size,
                allocated_size,
            ));
        }
    }
    record.caveats.extend(caveats);
}

fn stream_physical_allocated_size(
    stream: &NtfsAttributeStream,
    geometry: NtfsStreamGeometry,
) -> Option<u64> {
    if !stream.non_resident || stream.data_runs.is_empty() || geometry.bytes_per_cluster == 0 {
        return None;
    }
    if !data_runs_cover_stream(stream, geometry.bytes_per_cluster) {
        return None;
    }

    let allocated_clusters = stream
        .data_runs
        .iter()
        .filter(|run| run.lcn.is_some())
        .try_fold(0_u64, |sum, run| sum.checked_add(run.cluster_count))?;
    allocated_clusters.checked_mul(geometry.bytes_per_cluster)
}

fn data_runs_cover_stream(stream: &NtfsAttributeStream, bytes_per_cluster: u64) -> bool {
    let logical_clusters = stream.logical_size.div_ceil(bytes_per_cluster);
    let mut expected_vcn = 0_u64;
    for run in &stream.data_runs {
        if run.starting_vcn != expected_vcn {
            return false;
        }
        let Some(next_vcn) = expected_vcn.checked_add(run.cluster_count) else {
            return false;
        };
        expected_vcn = next_vcn;
    }

    expected_vcn >= logical_clusters
}

fn data_run_allocated_by_cluster_caveat(
    record_id: u64,
    header_allocated_size: Option<u64>,
    data_run_allocated_size: u64,
) -> ParseCaveat {
    let header_allocated = header_allocated_size
        .map(|bytes| bytes.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    ParseCaveat::new(
        DATA_RUN_ALLOCATED_BY_CLUSTER_CAVEAT,
        format!(
            "record {record_id} unnamed $DATA allocated_bytes uses {data_run_allocated_size} bytes from covering NTFS data-run clusters instead of the attribute header value {header_allocated}; this is physical cluster evidence and may differ from Windows file allocation APIs"
        ),
    )
}

fn expand_record_attribute_lists<S>(
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
            stream.attribute_type == AttributeType::AttributeList && stream.non_resident
        })
        .cloned()
        .collect::<Vec<_>>();
    for stream in streams {
        expand_attribute_list_stream(record, geometry, source, &stream);
    }
}

fn expand_attribute_list_stream<S>(
    record: &mut NtfsParsedRecord,
    geometry: NtfsStreamGeometry,
    source: &mut S,
    stream: &NtfsAttributeStream,
) where
    S: NtfsStreamSource,
{
    if (stream.flags & (STREAM_FLAG_COMPRESSED | STREAM_FLAG_ENCRYPTED)) != 0 {
        record
            .caveats
            .push(unsupported_attribute_list_stream_caveat(
                record.reference.record_id,
                "compressed or encrypted attribute-list streams are unsupported",
            ));
        return;
    }
    if (stream.flags & STREAM_FLAG_SPARSE) != 0 {
        record
            .caveats
            .push(unsupported_attribute_list_stream_caveat(
                record.reference.record_id,
                "sparse attribute-list streams are unsupported",
            ));
        return;
    }
    if stream.logical_size == 0 {
        return;
    }
    if stream.logical_size > MAX_ATTRIBUTE_LIST_STREAM_BYTES {
        record
            .caveats
            .push(unsupported_attribute_list_stream_caveat(
                record.reference.record_id,
                format!(
                    "attribute-list stream size {} exceeds cap {}",
                    stream.logical_size, MAX_ATTRIBUTE_LIST_STREAM_BYTES
                ),
            ));
        return;
    }

    let Ok(len) = usize::try_from(stream.logical_size) else {
        record
            .caveats
            .push(unsupported_attribute_list_stream_caveat(
                record.reference.record_id,
                "attribute-list stream size does not fit in memory",
            ));
        return;
    };
    let reader = NtfsStreamReader::new(geometry.bytes_per_cluster, SparseRunPolicy::Reject);
    let bytes = match reader.read_range(source, &stream.data_runs, 0, len) {
        Ok(bytes) => bytes,
        Err(err) => {
            record.caveats.push(invalid_attribute_list_stream_caveat(
                record.reference.record_id,
                format!("stream read failed: {err}"),
            ));
            return;
        }
    };
    let entries = match parse_attribute_list(&bytes) {
        Ok(entries) => entries,
        Err(err) => {
            record.caveats.push(invalid_attribute_list_stream_caveat(
                record.reference.record_id,
                format!("attribute list could not be parsed: {err}"),
            ));
            return;
        }
    };
    append_attribute_list_entries(record, entries);
}

fn append_attribute_list_entries(
    record: &mut NtfsParsedRecord,
    entries: Vec<NtfsAttributeListEntry>,
) {
    if entries
        .iter()
        .any(|entry| entry.attribute_type == AttributeType::AttributeList)
    {
        record.caveats.push(ParseCaveat::new(
            "recursive-attribute-list-unsupported",
            "attribute list points at another attribute list; recursive expansion is refused",
        ));
    }

    for entry in entries {
        append_unique(&mut record.attribute_list_entries, entry);
    }
}

fn unsupported_attribute_list_stream_caveat(
    record_id: u64,
    reason: impl Into<String>,
) -> ParseCaveat {
    ParseCaveat::new(
        "unsupported-attribute-list-stream",
        format!(
            "record {record_id} nonresident $ATTRIBUTE_LIST was not expanded: {}",
            reason.into()
        ),
    )
}

fn invalid_attribute_list_stream_caveat(record_id: u64, reason: impl Into<String>) -> ParseCaveat {
    ParseCaveat::new(
        "invalid-attribute-list-stream",
        format!(
            "record {record_id} nonresident $ATTRIBUTE_LIST could not be parsed: {}",
            reason.into()
        ),
    )
}

fn push_missing_extension_attribute_caveat(
    record: &mut NtfsParsedRecord,
    entry: &NtfsAttributeListEntry,
    extension: &NtfsParsedRecord,
) {
    record.caveats.push(ParseCaveat::new(
        "attribute-list-extension-attribute-missing",
        format!(
            "extension record {} does not contain attribute id {}",
            extension.reference.record_id, entry.attribute_id
        ),
    ));
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
    let root_entries = directory_index.root_entries.clone();
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
            &root_entries,
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
    root_entries: &[NtfsIndexEntry],
    source: &mut S,
) where
    S: NtfsStreamSource,
{
    if (stream.flags & (STREAM_FLAG_COMPRESSED | STREAM_FLAG_ENCRYPTED)) != 0 {
        record.caveats.push(ParseCaveat::new(
            "unsupported-index-allocation",
            format!(
                "record {} has compressed or encrypted $INDEX_ALLOCATION:$I30",
                record.reference.record_id
            ),
        ));
        return;
    }
    if (stream.flags & STREAM_FLAG_SPARSE) != 0 {
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
    let mut traversal = IndexAllocationTraversal {
        record_id,
        reader,
        geometry,
        index_record_size,
        stream,
        source,
        entries: &mut entries,
        seen_entries: &mut seen_entries,
        visited_vcns: BTreeSet::new(),
        caveats: Vec::new(),
    };
    traversal.traverse_entries(root_entries);
    let caveats = traversal.caveats;
    record.directory_entries = entries;
    record.caveats.extend(caveats);
}

struct IndexAllocationTraversal<'a, S>
where
    S: NtfsStreamSource,
{
    record_id: u64,
    reader: &'a NtfsStreamReader,
    geometry: NtfsStreamGeometry,
    index_record_size: usize,
    stream: &'a NtfsAttributeStream,
    source: &'a mut S,
    entries: &'a mut Vec<NtfsDirectoryEntry>,
    seen_entries: &'a mut BTreeSet<NtfsDirectoryEntryKey>,
    visited_vcns: BTreeSet<u64>,
    caveats: Vec<ParseCaveat>,
}

impl<S> IndexAllocationTraversal<'_, S>
where
    S: NtfsStreamSource,
{
    fn traverse_entries(&mut self, node_entries: &[NtfsIndexEntry]) {
        for entry in node_entries {
            if let Some(child_vcn) = entry.child_vcn {
                self.traverse_child(child_vcn);
            }
            if let Some(directory_entry) = &entry.directory_entry {
                append_unique_directory_entry(
                    self.entries,
                    self.seen_entries,
                    directory_entry.clone(),
                );
            }
        }
    }

    fn traverse_child(&mut self, child_vcn: u64) {
        if !self.visited_vcns.insert(child_vcn) {
            self.caveats.push(invalid_index_allocation_caveat(
                self.record_id,
                format!("child VCN {child_vcn} was already visited while traversing $I30"),
            ));
            return;
        }

        match read_index_allocation_record(
            self.reader,
            self.source,
            self.stream,
            self.geometry,
            self.index_record_size,
            child_vcn,
        ) {
            Ok(record) => self.traverse_entries(&record.entries),
            Err(err) => self.caveats.push(invalid_index_allocation_caveat(
                self.record_id,
                format!("child VCN {child_vcn} could not be read: {err}"),
            )),
        }
    }
}

fn read_index_allocation_record<S>(
    reader: &NtfsStreamReader,
    source: &mut S,
    stream: &NtfsAttributeStream,
    geometry: NtfsStreamGeometry,
    index_record_size: usize,
    child_vcn: u64,
) -> Result<NtfsIndexAllocationRecord, IndexAllocationReadError>
where
    S: NtfsStreamSource,
{
    let logical_offset = index_allocation_record_offset(child_vcn, geometry, index_record_size)?;
    let index_record_size_u64 =
        u64::try_from(index_record_size).map_err(|_| IndexAllocationReadError::OffsetOverflow)?;
    let logical_end = logical_offset
        .checked_add(index_record_size_u64)
        .ok_or(IndexAllocationReadError::OffsetOverflow)?;
    if logical_end > stream.logical_size {
        return Err(IndexAllocationReadError::VcnOutOfRange {
            child_vcn,
            logical_offset,
            logical_size: stream.logical_size,
        });
    }
    let raw_record = reader
        .read_range(source, &stream.data_runs, logical_offset, index_record_size)
        .map_err(IndexAllocationReadError::Stream)?;

    parse_i30_index_allocation_record(&raw_record, geometry.bytes_per_sector, child_vcn)
        .map_err(|err| IndexAllocationReadError::InvalidRecord(err.to_string()))
}

fn index_allocation_record_offset(
    child_vcn: u64,
    geometry: NtfsStreamGeometry,
    index_record_size: usize,
) -> Result<u64, IndexAllocationReadError> {
    if geometry.bytes_per_cluster == 0 {
        return Err(IndexAllocationReadError::InvalidGeometry(
            "cluster size is zero",
        ));
    }
    if index_record_size == 0 {
        return Err(IndexAllocationReadError::InvalidGeometry(
            "index record size is zero",
        ));
    }

    let index_record_size_u64 =
        u64::try_from(index_record_size).map_err(|_| IndexAllocationReadError::OffsetOverflow)?;
    if index_record_size_u64 < geometry.bytes_per_cluster && child_vcn != 0 {
        return Err(IndexAllocationReadError::UnsupportedGeometry(
            "nonzero child VCN with index record size smaller than cluster size",
        ));
    }

    child_vcn
        .checked_mul(geometry.bytes_per_cluster)
        .ok_or(IndexAllocationReadError::OffsetOverflow)
}

#[derive(Debug)]
enum IndexAllocationReadError {
    InvalidGeometry(&'static str),
    UnsupportedGeometry(&'static str),
    OffsetOverflow,
    VcnOutOfRange {
        child_vcn: u64,
        logical_offset: u64,
        logical_size: u64,
    },
    Stream(NtfsStreamReadError),
    InvalidRecord(String),
}

impl std::fmt::Display for IndexAllocationReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGeometry(reason) => write!(formatter, "invalid geometry: {reason}"),
            Self::UnsupportedGeometry(reason) => {
                write!(formatter, "unsupported geometry: {reason}")
            }
            Self::OffsetOverflow => write!(formatter, "index allocation offset overflowed"),
            Self::VcnOutOfRange {
                child_vcn,
                logical_offset,
                logical_size,
            } => write!(
                formatter,
                "child VCN {child_vcn} maps to logical offset {logical_offset}, beyond stream size {logical_size}"
            ),
            Self::Stream(err) => write!(formatter, "stream read failed: {err}"),
            Self::InvalidRecord(err) => write!(formatter, "INDX record is invalid: {err}"),
        }
    }
}

fn append_unique_directory_entry(
    existing: &mut Vec<NtfsDirectoryEntry>,
    seen: &mut BTreeSet<NtfsDirectoryEntryKey>,
    incoming: NtfsDirectoryEntry,
) {
    if seen.insert(NtfsDirectoryEntryKey::from(&incoming)) {
        existing.push(incoming);
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
