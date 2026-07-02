# Rebecca NTFS Test Fixtures

This directory is reserved for deterministic NTFS parser fixtures.

Current coverage is generated in `tests/mft_parser.rs` and `benches/mft_parser.rs` instead of storing opaque binary blobs. The generated fixtures cover:

- resident and nonresident `$DATA` streams
- resident `$INDEX_ROOT:$I30` directory entries
- nonresident `$INDEX_ALLOCATION:$I30` INDX records
- fragmented and sparse runlists
- direct `$ATTRIBUTE_LIST` extension streams
- invalid fixup, VCN mismatch, short read, and missing extension cases

Recorded live-volume fixtures may be added later only when they contain no private path names, no deleted-entry slack, and clear provenance notes. GPL, LGPL, and forensic reference projects remain behavior references only; do not copy their implementation code or generated binary fixtures into this tree.
