# Rebecca NTFS Test Fixtures

This directory is reserved for deterministic NTFS parser fixtures.

Current coverage is generated in `tests/mft_parser.rs` and `benches/mft_parser.rs` instead of storing opaque binary blobs. The generated fixtures cover:

- resident and nonresident `$DATA` streams
- resident `$INDEX_ROOT:$I30` directory entries
- nonresident `$INDEX_ALLOCATION:$I30` INDX records
- fragmented and sparse runlists
- direct `$ATTRIBUTE_LIST` extension streams
- resolved `$STANDARD_INFORMATION`, `$FILE_NAME`, `$INDEX_ROOT:$I30`, `$DATA`, and `$INDEX_ALLOCATION:$I30` extension attributes
- invalid record used-size bounds, invalid attribute name/value ranges, invalid fixup, VCN mismatch, short read, and missing extension cases
- `$MFTMirr` recovery semantics where generated mirror records can recover corrupt or truncated primary `$MFT` records with explicit caveats

Optional fuzz targets live outside the normal workspace at `crates/rebecca-ntfs/fuzz` and cover MFT records, resident attribute lists, `$I30` index records, and runlists. They are developer tooling only; normal CI and release verification use deterministic tests under `tests/mft_parser.rs`.

Recorded live-volume or raw-image fixtures may be added later only when they contain no private path names, no deleted-entry slack, no credentials, no user file contents, and clear provenance notes. Mirror recovery fixtures should remain generated wherever possible; `$MFTMirr` data is read-only evidence and never deletion authority. Prefer checked-in generators over opaque binary blobs. GPL, LGPL, and forensic reference projects remain behavior references only; do not copy their implementation code or generated binary fixtures into this tree.
