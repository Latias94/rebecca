# Rebecca NTFS Fuzz Targets

This directory is optional developer tooling for parser hardening. It is not a
workspace member, so normal `cargo check`, `cargo nextest`, and release builds
do not require fuzz tooling.

Useful checks:

```powershell
cargo check --manifest-path crates/rebecca-ntfs/fuzz/Cargo.toml --bins
cargo fuzz run mft_record --manifest-path crates/rebecca-ntfs/fuzz/Cargo.toml
```

Targets intentionally accept arbitrary bytes and discard parse results. The
contract is memory safety, bounded parser failure, and no panic on malformed
records, attribute lists, `$I30` indexes, or runlists.

Do not store live-volume corpus files here unless they are scrubbed of private
path names, deleted-entry slack, and machine identifiers. GPL/LGPL reference
projects are behavior references only; do not copy code or binary fixtures from
them into this tree.
