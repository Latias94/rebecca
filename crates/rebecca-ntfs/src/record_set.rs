use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adapter::{NtfsFileReference, NtfsParsedRecord, merge_data_stream};
use crate::attrs::AttributeType;
use crate::record::ParseCaveat;

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
                    AttributeType::Data => {
                        let mut matched = false;
                        for stream in extension.data_streams.iter().filter(|stream| {
                            stream.attribute_id == entry.attribute_id
                                && stream.name == entry.name
                                && stream.lowest_vcn == Some(entry.lowest_vcn)
                        }) {
                            merge_data_stream(&mut record.data_streams, stream.clone());
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
