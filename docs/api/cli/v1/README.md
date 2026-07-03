# Rebecca CLI API v1

Rebecca treats machine-mode CLI output as a versioned API for GUI wrappers and
automation. Human text remains the default. Machine consumers should always
request `--format json` for final results or `--format ndjson` for long-running
cleanup workflows.

API v1 is the only CLI machine contract. Cleanup execution, purge execution,
history, config, cache, doctor, catalog, and read-only inspect commands all
emit `api_version = "rebecca.cli.v1"`. `rebecca purge inspect` is retained as
a compatibility alias for the `inspect-artifacts` payload.

## Channel Rules

- `human`: writes readable text to stdout and errors to stderr.
- `json`: writes one success envelope to stdout. Fatal errors write one error
  envelope to stderr and exit non-zero.
- `ndjson`: writes one compact JSON event per stdout line. Terminal success is
  a `completed` event. Terminal failures are `error` events.
- Human progress text is never mixed into machine stdout.

## Envelopes

Success responses use `envelope.schema.json`:

```json
{
  "api_version": "rebecca.cli.v1",
  "kind": "success",
  "command": "history",
  "payload_kind": "history-list",
  "generated_at_unix_seconds": 1782660000,
  "data": []
}
```

Failures use `error.schema.json`. Error codes are stable kebab-case strings
such as `invalid-rule-id`, `invalid-category`, `config-parse-failed`, and
`platform-unavailable`.

NDJSON events use `event.schema.json`. Consumers should read stdout line by
line and parse each line independently.

Cleanup workflow NDJSON defaults to target-level progress: `started`,
`target-scanning`, `target-finished`, scan-cache events, and terminal
`completed` or `error` events. File-level `file-measured` events are omitted by
default to avoid turning large scans into one JSON line per file. Ordinary
cleanup scans can opt into file-level scan details with
`--progress-detail file` when a debugger or GUI explicitly needs them.

## Payload Kinds

The `payload_kind` field identifies the shape under `data`:

- `rule-catalog`
- `cleanup-plan`
- `app-leftovers-cleanup-plan`
- `project-artifact-cleanup-plan`
- `project-artifact-catalog`
- `catalog`
- `catalog-validation`
- `inspect-space`
- `inspect-map`
- `inspect-artifacts`
- `inspect-lint`
- `cache-purge-report`
- `history-list`
- `config-paths`
- `permissions-diagnostic`
- `active-process-diagnostic`

Payload data is intentionally nested under `data` so Rebecca can evolve
metadata, event transport, and error handling without turning internal core
models into the top-level API.

Cleanup and inspect targets include estimate provenance so consumers can explain
byte total trust without changing `estimated_bytes` arithmetic. `estimate_source`
remains the stable source field:

- `fresh-scan`: bytes came from a live filesystem scan during this command;
- `scan-cache`: bytes came from an enabled scan-cache hit;
- `not-measured`: the target was skipped or blocked before byte measurement;
- `unknown`: legacy or externally supplied plans that predate this field.

When known, targets also include:

- `estimate_backend`: scanner that produced the byte estimate, such as
  `portable-recursive`, `windows-native`, or
  `windows-ntfs-mft-experimental`;
- `estimate_backend_source`: optional implementation source within the selected
  scanner, such as `windows-ntfs-mft-experimental-targeted-fsctl`,
  `windows-ntfs-mft-experimental-sequential`, or
  `windows-ntfs-mft-experimental-fsctl-record`;
- `estimate_confidence`: estimate confidence, currently `exact`;
- `estimate_fallback_reason`: why Rebecca fell back from a requested backend;
- `estimate_caveats`: structured caveats with `code` and `message`.

The `windows-ntfs-mft-experimental` backend is read-only and opt-in. When live
NTFS metadata is available, `estimate_backend_source` distinguishes the
normal targeted per-record FSCTL traversal source from explicit full-index
diagnostic sources. When live metadata is unavailable or ambiguous, Rebecca
reports fallback provenance instead of treating raw metadata as cleanup
authority. Parser caveats may
include sequence mismatches, hardlink path candidates, direct attribute-list
extension handling, resident or nonresident `$I30` directory-index fallback,
unreadable or unsupported index-allocation streams, or bounded parse-error
summaries. Valid nonresident `$INDEX_ALLOCATION:$I30` metadata can supplement
subtree edges, but these fields are still explainability data; they do not
authorize deletion or change v1 byte semantics.

Cleanup plans include `summary.warning_matrix` and warning-bearing targets carry
`warnings`. A target with `reason_code: "warning-gate-required"` was excluded
until the user selects the named gate with `--allow-warning <warning>`.

`active-process-diagnostic` is emitted by `rebecca doctor active-processes`.
It reports whether process inspection is available and lists running processes
that match cleanup rules carrying the `active-process` warning.

`catalog` is emitted by `rebecca catalog`. The payload is a typed array of
cleanup rules, project artifact policies, warning gates, safety categories, and
supported action kinds. `catalog-validation` is emitted by
`rebecca catalog validate`.

`inspect-space`, `inspect-map`, `inspect-artifacts`, and `inspect-lint` are
read-only cleanup intelligence reports. They inventory top-level space, ranked
disk-map usage, project artifact reclaim opportunities, or duplicate/large/empty
file findings without prompting, executing cleanup, writing history, or mutating
files. `inspect-map` uses `logical_bytes` plus nullable `allocated_bytes`
instead of `estimated_bytes` because it is a disk inventory surface rather than
a cleanup estimate surface. Portable inventory leaves allocation unknown;
Windows native inventory fills file allocation bytes when the host API exposes
them, and NTFS/MFT inventory can fill allocation from parsed stream metadata.

Project artifact cleanup targets include a `project_artifact` object when they
were discovered by `rebecca purge`. The object explains why the target was
eligible:

- `matched_context`: stable kebab-case rule context such as `node-project`,
  `target-project`, or `cachedir-tag`;
- `project_root`: directory whose project context was accepted;
- `project_anchor`: file or marker directory that justified the match, such as
  `package.json`, `Cargo.toml`, or `CACHEDIR.TAG`.

Project artifact cleanup plans may also include `discovery_diagnostics`.
Diagnostics are plan-level observations with `kind`, `path`, and `detail`; they
make partial discovery visible without adding fake cleanup targets or changing
target counts.

`inspect-space` and `inspect-map` reports include `diagnostic_summary` with
complete diagnostic counts. The `diagnostics` array is a bounded raw sample list
controlled by `--diagnostic-limit`; use the summary fields for authoritative
totals and truncation detection.

Purge targets carry the same estimate provenance fields as cleanup targets.
Consumers should use the explicit `rule_id`, `status`, `reason_code`,
`estimate_source`, backend/source/confidence/caveat fields, and `project_artifact`
explanation fields. Provenance explains where a byte estimate came from; it is
not a freshness guarantee and is never deletion authority.

## Examples

```powershell
rebecca scan --format json
rebecca clean --format json --category system
rebecca clean --format ndjson --scan-cache --category system
rebecca clean --format ndjson --progress-detail file --rule windows.user-temp
rebecca doctor active-processes --format json
rebecca purge --format json --root . --min-age-days 0
rebecca catalog --format json --kind warning
rebecca inspect space --format json --root . --diagnostic-limit 100
rebecca inspect map --format json --root . --top 20 --max-depth 3 --diagnostic-limit 100
rebecca inspect artifacts --format json --root . --min-age-days 0
rebecca purge inspect --format json --root . --min-age-days 0
rebecca inspect lint --format json --root .
rebecca doctor permissions --format json
```

Representative fixtures live in `examples/` and schemas live next to this
README. The schemas use JSON Schema Draft 2020-12.
