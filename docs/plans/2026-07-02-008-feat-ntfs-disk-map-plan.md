---
artifact_contract: "ce-unified-plan/v1"
artifact_readiness: "implementation-ready"
execution: "code"
product_contract_source: "ce-plan-bootstrap"
origin: "conversation"
created: "2026-07-02"
last_updated: "2026-07-02"
title: "NTFS Disk Map Inspect - Plan"
---

# NTFS Disk Map Inspect - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Add an explicit read-only disk-map inspect command that can use full-volume NTFS/MFT inventory for WizTree-class space insight without making ordinary cleanup estimates pay the full-volume cost. |
| User value | Users can quickly find the largest directories and files on an NTFS volume, compare logical and allocated size, and inspect caveats before deciding what cleanup workflow to run. |
| Safety stance | Report-only. Raw NTFS metadata is never deletion authority, never bypasses cleanup safety gates, and never writes history or cleanup plans. |
| Primary surfaces | `crates/rebecca-core`, `crates/rebecca`, CLI API v1 docs, performance/dogfood docs. |
| Stop conditions | Stop if the command can delete, mutates user data, hides NTFS caveats, silently scans a different volume than requested, reports partial full-volume inventory as exact, or copies GPL/LGPL/mixed-license code from references. |

## Product Contract

### Summary

Rebecca now has a fast targeted NTFS/MFT path for ordinary cleanup estimates. The next highest-value slice is the complementary explicit full-volume path: a read-only disk map for users who intentionally ask "what is using space on this volume?"

This should feel like a CLI-native WizTree report, not a UI clone. The command should return stable machine output, bounded human output, provenance, caveats, and safe fallback behavior.

### Problem Frame

The existing `inspect space` command answers "how large is this root and its top direct entries?" It is intentionally conservative and can use targeted traversal. It does not expose a whole-volume ranked tree or allocated-size insight.

Full-volume MFT indexing is too expensive for every cleanup target, but it is exactly the right tool for explicit disk-map inventory. The previous full-index code already exists as a diagnostic fallback; this plan promotes it into a named report-only product surface with tighter contracts, budgets, and tests.

### Requirements

- R1. Add a read-only CLI command, tentatively `rebecca inspect map`, that reports disk usage rankings for a requested root or volume.
- R2. The command must default to safe bounded output: top-N entries, optional max depth, and no unbounded human rendering.
- R3. When explicitly selected with `--scan-backend windows-ntfs-mft-experimental`, the command should use an explicit full-volume MFT inventory source rather than targeted traversal.
- R4. On unsupported, unelevated, non-NTFS, or timed-out hosts, the command must fall back to the existing safe directory scanner or return a clear diagnostic, depending on the selected backend policy.
- R5. Output must distinguish `logical_bytes`, optional `allocated_bytes`, file count, directory count, entry kind, path, depth, backend, backend source, confidence, fallback reason, and caveats.
- R6. Machine output must use the existing `rebecca.cli.v1` success/error envelope pattern with a new `payload_kind`, schemas, and examples.
- R7. The core report must preserve NTFS caveats for attribute-list, sequence mismatch, hardlinks, reparse points, invalid records, directory-index fallback, and budget/timeouts.
- R8. Full-volume inventory must validate target volume identity before attributing MFT records to a path.
- R9. The command must never write cleanup history, scan-cache records, or deletion plans.
- R10. The implementation must reuse existing parser/index structures where appropriate instead of introducing a second NTFS model.
- R11. Elevated dogfood must prove the command on a small root such as `docs/plans` and, when practical, on a drive root with bounded top output.
- R12. Docs must clearly say this is a read-only inspection surface and not cleanup authorization.

### Key Flows

- F1. Inspect a directory map on NTFS
  - Trigger: User runs `rebecca inspect map --root docs/plans --top 20`.
  - Behavior: Rebecca resolves the volume, builds an explicit full-volume MFT inventory if supported, ranks entries under the requested root, and returns logical/allocated totals plus top entries.
  - Outcome: The user gets fast disk-map insight with `estimate_backend_source` or equivalent source provenance.

- F2. Inspect a drive root
  - Trigger: User runs `rebecca inspect map --root <volume-root> --top 50 --max-depth 3`.
  - Behavior: Rebecca reports top ranked directories/files under the drive with bounded rendering.
  - Outcome: The command can answer whole-volume "what is big?" questions without invoking cleanup planning.

- F3. Unsupported or unelevated host
  - Trigger: User runs the command without volume privileges, on non-NTFS, or on an unsupported path.
  - Behavior: Rebecca returns a clear fallback reason and either uses the safe scanner for the requested root or reports a skipped root based on backend policy.
  - Outcome: No partial MFT data is mislabeled as exact.

- F4. Ambiguous NTFS metadata
  - Trigger: Attribute-list, hardlink, sequence, reparse, or directory-index caveats appear.
  - Behavior: Rebecca includes bounded caveats and keeps byte totals conservative.
  - Outcome: Machine consumers can explain uncertainty instead of treating raw metadata as final truth.

### Acceptance Examples

- AE1. Given `docs/plans`, when `inspect map --format json --root docs/plans --top 3` runs, then totals match `inspect space` for the same root and output source identifies portable recursive inventory by default.
- AE1b. Given an elevated NTFS host, when `inspect map --format json --scan-backend windows-ntfs-mft-experimental --root docs/plans --top 3` runs, then Rebecca either identifies an explicit full-volume NTFS map source or falls back with a budget/timeout reason without returning a partial exact map.
- AE2. Given a non-NTFS path, when `inspect map` runs, then it falls back or reports unsupported with no NTFS backend source.
- AE3. Given a directory with a sparse or allocated-size fixture, when the map report renders JSON, then logical and allocated byte fields are distinct where data is available.
- AE4. Given a root with more entries than `--top`, when human output renders, then only bounded entries are shown while totals still reflect the scanned inventory.
- AE5. Given a timeout during full-volume inventory, when the command runs, then no partial exact map is returned.
- AE6. Given `--format ndjson`, when the command runs, then lifecycle/completion events use the v1 event envelope and the final payload matches the JSON schema.

### Scope Boundaries

- In scope:
  - A read-only CLI/API v1 disk-map report.
  - NTFS full-volume inventory as an explicit inspect source.
  - Portable/native fallback for requested roots.
  - Logical and allocated bytes where available.
  - Bounded human, JSON, and NDJSON output.

- Out of scope:
  - Graphical treemap UI.
  - Persistent cross-run raw MFT index cache.
  - Automatic cleanup suggestions from disk-map entries.
  - Deletion, quarantine, or recycle-bin actions.
  - Non-Windows filesystem-specific raw metadata readers.

## Planning Contract

### Key Technical Decisions

- KTD1. Add `inspect map`, not another mode flag on `inspect space`.
  - Rationale: `inspect space` is root-focused and safe for routine estimates. Disk map is an explicit whole-volume-shaped operation with different cost, output, and caveat semantics.

- KTD2. Reuse the existing NTFS parser/index model.
  - Rationale: `rebecca-ntfs::MftIndex`, streams, file references, hardlink candidates, and caveats already encode the correctness work. A second model would diverge.

- KTD3. Keep full-volume MFT inventory explicit.
  - Rationale: The targeted traversal refactor made ordinary cleanup estimates fast. This plan must not regress that by routing normal scans back through full-volume indexing.

- KTD3b. Default `inspect map` to portable recursive inventory until full-volume MFT map construction is fast enough on large live volumes.
  - Rationale: Elevated dogfood on E: showed full-volume MFT construction exceeding both the 20 second internal budget and a 180 second no-budget script timeout. A read-only disk map should be useful by default and make current full-index costs explicit through `--scan-backend windows-ntfs-mft-experimental`.

- KTD4. Treat partial full-volume inventory as fallback/diagnostic, not exact.
  - Rationale: Disk-map users need trustworthy totals. Bounded caveats are acceptable for metadata ambiguity; partial traversal caused by budget or timeout is not an exact map.

- KTD5. Expose allocated bytes as optional.
  - Rationale: NTFS streams can provide allocated size, but portable fallback may not. The API should be additive and honest about availability.

- KTD6. Do not use restricted-license source code.
  - Rationale: GPL/LGPL/mixed projects remain behavior references only. Implementation stays first-party.

### High-Level Design

```text
CLI inspect map
  -> rebecca::inspect map command adapter
  -> rebecca-core::disk_map request
  -> backend selector
      -> explicit NTFS full-volume inventory on supported NTFS
      -> windows-native or portable fallback for requested root
  -> ranked bounded report
  -> human / JSON / NDJSON v1 renderers
```

The core should define a report model separate from cleanup plans:

- `DiskMapRequest`: roots, top limit, max depth, backend policy, cancellation.
- `DiskMapReport`: roots, totals, entries, diagnostics.
- `DiskMapEntry`: path, root, kind, depth, logical bytes, allocated bytes, files, directories, backend provenance, caveats.
- `DiskMapDiagnostic`: unsupported, fallback, timeout, metadata caveat summaries.

The NTFS path should adapt the existing full-index builder into an explicit function that returns enough ranked entries for the requested root. It should avoid materializing unbounded rendered rows; ranking can use bounded heaps where possible.

### System-Wide Impact

| Area | Impact |
|---|---|
| `crates/rebecca-core/src/scan/windows_ntfs_mft.rs` | Extract or expose explicit full-volume inventory builder while keeping cleanup estimate default targeted-first. |
| `crates/rebecca-core/src/disk_map.rs` | New report-only core domain for map requests, entries, diagnostics, and ranking. |
| `crates/rebecca/src/cli.rs` | Add `inspect map` arguments. |
| `crates/rebecca/src/inspect.rs` and renderers | Add command adapter and human/machine output. |
| `docs/api/cli/v1` | Add schema/example docs for `inspect-map`. |
| `docs/configuration.md`, `docs/performance/perf-matrix.md`, `docs/release.md` | Document backend behavior and dogfood. |

### Sources And Research

- `docs/plans/2026-07-02-007-refactor-ntfs-targeted-traversal-plan.md` established the split: targeted traversal for ordinary estimates, full-volume indexing for explicit disk-map/deep-inspect surfaces.
- `repo-ref/DiscUtils`, `repo-ref/go-ntfs`, `repo-ref/gomft`, `repo-ref/python-ntfs`, `repo-ref/libfsntfs`, `repo-ref/ntfs-3g`, and `repo-ref/sleuthkit` remain behavior/model references only. Do not copy restricted-license code.
- `docs/adr/0005-scan-engine-strategy.md` and `docs/adr/0009-ntfs-parser-dependency-strategy.md` define the scan/backend safety boundary.

### Risks And Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Full-volume inventory is slow on busy drives. | Poor UX or timeouts. | Use existing budget monitor, expose timeout diagnostics, keep output bounded, and dogfood on local NTFS. |
| Partial MFT map is mistaken for exact. | User makes wrong cleanup decisions. | Return fallback-capable errors on timeout/budget failure; preserve caveats and confidence. |
| The command duplicates `inspect space`. | API confusion. | Position `inspect map` as ranked inventory/deep inspect; leave routine root estimates in `inspect space`. |
| Allocated size availability differs by backend. | Schema inconsistency. | Make allocated bytes nullable/optional and document source support. |
| Full index builder remains tangled with cleanup scan backend. | Future regressions. | Extract explicit internal API with tests proving `inspect space` still uses targeted-first. |

### Dependencies And Sequencing

1. Extract explicit NTFS full-inventory API without changing existing scanner behavior.
2. Build core disk-map report model and ranking on top of that API.
3. Add CLI/API/renderers and fallback behavior.
4. Add docs, dogfood, and performance evidence.

## Implementation Units

### U1. Extract Explicit NTFS Full-Volume Inventory API

- **Goal:** Make the older full-volume MFT index builder callable only by explicit inspect-map code and diagnostic fallback.
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`, `crates/rebecca-core/tests/scan_engine.rs`
- **Approach:** Split the full-index build path from `WindowsNtfsMftScanBackend` into an internal read-only function or service that returns source label, `MftIndex`, timings, and caveats. Keep ordinary `measure_path_with_progress` targeted-first.
- **Patterns to follow:** `build_mft_index_from_records`, `WindowsNtfsMftIndexCache`, `NtfsMftBuildMonitor`, `with_bounded_mft_caveats`.
- **Test scenarios:** Existing cleanup/inspect estimates still use `windows-ntfs-mft-experimental-targeted-fsctl` when targeted succeeds; explicit full-inventory API reports `sequential` or `fsctl-record` source; timeout returns fallback-capable error.
- **Verification:** `cargo nextest run -p rebecca-core windows_ntfs_mft::tests`

### U2. Add Core Disk Map Report Model

- **Goal:** Add a report-only core module that ranks disk-map entries with bounded memory and honest provenance.
- **Files:** `crates/rebecca-core/src/disk_map.rs`, `crates/rebecca-core/src/lib.rs`, `crates/rebecca-core/tests/disk_map.rs`
- **Approach:** Define request/report/entry/diagnostic types. Implement ranking for entries under requested roots. Use logical bytes everywhere; add optional allocated bytes when the source supplies it.
- **Patterns to follow:** `crates/rebecca-core/src/inspect.rs`, `crates/rebecca-core/src/lint.rs`, `SpaceInsightTopEntries`.
- **Test scenarios:** top-N ranking is deterministic; max-depth limits rendered entries but not totals; fallback diagnostics are preserved; allocated bytes are optional and do not break portable fallback.
- **Verification:** `cargo nextest run -p rebecca-core --test disk_map`

### U3. Wire NTFS Disk Map Backend

- **Goal:** Use explicit full-volume MFT inventory for disk-map reports on supported NTFS volumes.
- **Files:** `crates/rebecca-core/src/disk_map.rs`, `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`, `crates/rebecca-core/tests/disk_map.rs`
- **Approach:** Resolve each requested root to a volume and file reference, build the full index once per command/volume, map records to canonical paths, aggregate ranked entries under the requested root, and preserve caveats.
- **Patterns to follow:** `MftIndex::aggregate_subtree`, `MftIndex` path candidate logic, `SpaceInsightMeasurement`.
- **Test scenarios:** NTFS fixture map totals match subtree aggregation; hardlink records count once; sequence mismatch surfaces caveat; timeout/budget failure falls back or reports skipped according to policy.
- **Verification:** `cargo nextest run -p rebecca-core --test disk_map windows_ntfs_mft::tests`

### U4. Add CLI And v1 Machine Output

- **Goal:** Expose `rebecca inspect map` in human, JSON, and NDJSON modes.
- **Files:** `crates/rebecca/src/cli.rs`, `crates/rebecca/src/main.rs`, `crates/rebecca/src/inspect.rs`, `crates/rebecca/src/render/inspect.rs`, `crates/rebecca/tests/cli_inspect.rs`
- **Approach:** Add `InspectMapArgs` with `--root`, `--top`, `--max-depth`, `--scan-backend`, and existing global format handling. Use `CliApiContract::v1("inspect map", "inspect-map")`.
- **Patterns to follow:** `inspect space` command wiring and tests.
- **Test scenarios:** help lists map; JSON envelope uses `inspect-map`; NDJSON emits completion event; invalid roots produce diagnostics not cleanup prompts; `--top 0` preserves totals without entries.
- **Verification:** `cargo nextest run -p rebecca --test cli_inspect`

### U5. Publish API Schema, Docs, And Examples

- **Goal:** Make the new command consumable by wrappers and future agents.
- **Files:** `docs/api/cli/v1/README.md`, `docs/api/cli/v1/payloads.schema.json`, `docs/api/cli/v1/examples/success-inspect-map.json`, `README.md`, `docs/configuration.md`, `CHANGELOG.md`
- **Approach:** Add schema definitions, one JSON example, provenance field descriptions, and read-only safety notes. Update changelog Unreleased.
- **Patterns to follow:** Existing `inspect-space`, `inspect-artifacts`, and `inspect-lint` docs.
- **Test scenarios:** schema parses; examples validate; documented payload kind appears in API contract tests.
- **Verification:** `cargo nextest run -p rebecca --test cli_api`

### U6. Add Performance And Dogfood Evidence

- **Goal:** Prove the map command works locally and stays bounded.
- **Files:** `scripts/ntfs/run-live-mft-dogfood.ps1`, `docs/performance/perf-matrix.md`, `docs/release.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Extend the NTFS dogfood script with `inspect-map` mode or a separate scenario. Record backend source, totals, fallback reasons, and duration. Keep dry-run/read-only guarantees.
- **Patterns to follow:** Existing NTFS dogfood script and report schema.
- **Test scenarios:** self-test accepts inspect-map reports; elevated local run records source and matching totals; unsupported hosts report fallback.
- **Verification:** `pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -SelfTest`, elevated local dogfood when available.

## Verification Contract

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo nextest run -p rebecca-core --test disk_map`
- `cargo nextest run -p rebecca-core windows_ntfs_mft::tests`
- `cargo nextest run -p rebecca --test cli_inspect --test cli_api`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo check -p rebecca-core --benches`
- `pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -SelfTest`
- Elevated dogfood: `REBECCA_NTFS_MFT_INDEX_TIMINGS=1 pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -Root docs/plans -Mode inspect-map -Top 3 -TimeoutSeconds 60`
- `git diff --check`

## Definition Of Done

| ID | Done Condition |
|---|---|
| DoD1 | `inspect map` exists as a read-only CLI command with human, JSON, and NDJSON output. |
| DoD2 | NTFS full-volume inventory is explicit to disk-map/deep-inspect behavior through `--scan-backend windows-ntfs-mft-experimental`, and ordinary cleanup estimates remain targeted-first. |
| DoD3 | Output includes logical bytes, optional allocated bytes, counts, ranking, source provenance, confidence, fallback reason, and caveats. |
| DoD4 | Unsupported or partial NTFS inventory never reports exact partial maps. |
| DoD5 | API v1 schema/examples/docs validate. |
| DoD6 | Local elevated dogfood evidence exists or an explicit environment limitation is documented. |
| DoD7 | All Verification Contract gates pass or any unavailable host-only gate is documented with reason. |
