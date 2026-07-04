# Rebecca NTFS Fuzz Targets

This directory is optional developer tooling for parser hardening. It is not a
workspace member, so normal `cargo check`, `cargo nextest`, and release builds
do not require fuzz tooling.

Useful checks:

```powershell
cargo check --manifest-path crates/rebecca-ntfs/fuzz/Cargo.toml --bins
pwsh -File scripts/fuzz/run-ntfs-fuzz-smoke.ps1 -SecondsPerTarget 5
cargo fuzz run mft_record --manifest-path crates/rebecca-ntfs/fuzz/Cargo.toml
```

Targets accept arbitrary bytes and also understand text seeds that start with
`hex:`. The committed corpus uses that text form so seed intent stays reviewable
while the target still receives raw bytes after decoding.

The target contract is memory safety, bounded parser failure, no panic on
malformed records, attribute lists, `$I30` indexes, or runlists, and stable
repeat parsing for successful inputs. Lightweight invariants are intentionally
limited to facts the parser already owns, such as sequential runlist VCNs and
stable DTO equality across repeated parses.

Seed layout:

- `corpus/mft_record/`
- `corpus/attribute_list/`
- `corpus/i30_index/`
- `corpus/runlist/`

Smoke workflow:

1. Run `pwsh -File scripts/fuzz/run-ntfs-fuzz-smoke.ps1 -SecondsPerTarget 5`.
2. The script always runs `cargo check --manifest-path crates/rebecca-ntfs/fuzz/Cargo.toml --bins`.
3. If `cargo-fuzz` is on `PATH`, each selected target runs for the bounded time.
4. If `cargo-fuzz` is missing, the fuzz-run phase is reported as skipped after
   compile succeeds.
5. Reports are written under `target/fuzz-smoke/<timestamp>/` as JSON and
   Markdown.

Long runs can target one parser at a time:

```powershell
cargo fuzz run attribute_list --manifest-path crates/rebecca-ntfs/fuzz/Cargo.toml
```

Crash promotion:

- Minimize the crash with `cargo fuzz tmin` or the shortest reproducing bytes.
- Scrub private paths, deleted-entry slack, volume serials, and machine-specific
  identifiers before saving anything.
- Prefer adding an owned deterministic fixture to `tests/mft_parser.rs` over
  committing opaque bytes.
- If a corpus seed is still useful, store it as a reviewable `hex:` text seed.

Do not store live-volume corpus files here unless they are scrubbed of private
path names, deleted-entry slack, and machine identifiers. GPL/LGPL reference
projects are behavior references only; do not copy code or binary fixtures from
them into this tree.
