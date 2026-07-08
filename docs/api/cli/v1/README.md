# Rebecca CLI API v1

Rebecca treats machine-mode CLI output as a versioned API for GUI wrappers and
automation. Human text remains the default. Machine consumers should always
request `--format json` for final results or `--format ndjson` for long-running
cleanup or inspect workflows.

API v1 is the only CLI machine contract. Cleanup execution, purge execution,
history, config, cache, doctor, catalog, and read-only inspect commands all
emit `api_version = "rebecca.cli.v1"`. `rebecca inspect artifacts` is the
canonical command for the `inspect-artifacts` payload.

## Channel Rules

- `human`: writes readable text to stdout and errors to stderr.
- `json`: writes one success envelope to stdout. Fatal errors write one error
  envelope to stderr and exit non-zero.
- `ndjson`: writes one compact JSON event per stdout line. Terminal success is
  a `completed` event. Terminal failures are `error` events.
- Human progress text is never mixed into machine stdout.
- `--no-progress` disables human stderr progress only. It does not suppress
  NDJSON machine progress events.
- `rebecca tui` is intentionally outside the JSON/NDJSON API. It owns the
  terminal screen in human mode and should not be used by wrappers. Its map,
  type distribution, extension distribution, scoped refresh, and cleanup
  workbench views are human UI surfaces; use `inspect map`, `clean --dry-run`,
  and workflow JSON/NDJSON payloads for automation.

## Path Encoding

JSON and NDJSON path fields use `/` as the separator on every platform. This
keeps machine output stable across Windows, Linux, and macOS while human output
continues to use the host platform's native display style. The rule applies to
fields named `path`, `root`, `roots`, and fields ending in suffixes such as
`_path`, `_paths`, `_dir`, `_file`, `_root`, and `_roots`. App inventory
fields such as `install_locations` follow the same path encoding rule.

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
`platform-unavailable`. When `--format json` or `--format ndjson` is already
discoverable in argv, command-line parse failures use `invalid-arguments` in
the same error envelope/event. Invalid `--format` values are intentionally left
to clap's native error output so humans can see the accepted values.

NDJSON events use `event.schema.json`. Consumers should read stdout line by
line and parse each line independently.

Cleanup workflow NDJSON defaults to target-level progress: `started`,
`target-scanning`, `target-finished`, scan-cache events, and terminal
`completed` or `error` events. File-level `file-measured` events are omitted by
default to avoid turning large scans into one JSON line per file. Ordinary
cleanup scans can opt into file-level scan details with
`--progress-detail file` when a debugger or GUI explicitly needs them.

Inspect workflow NDJSON uses the same event envelope but a different progress
payload. `inspect space --format ndjson` and `inspect map --format ndjson`
start with `started`, emit bounded `inspect-progress` events while roots,
entries, caches, fallbacks, traversal counters, and backend stages are observed,
then emit the final report event(s), and finish with `completed`.
`inspect-progress` events use `payload_kind = "inspect-progress"` and
`data.progress_kind` values such as `root-started`, `root-finished`,
`entry-measured`, `traversal-progress`, `backend-fallback`,
`backend-stage-started`, `backend-stage-finished`, `backend-metric`,
`cache-event`, and `finalizing`. Default inspect progress is bounded at target,
root, backend, cache, sampled-counter, sampled backend-stage, and sampled
backend-metric granularity. The final `completed` payload remains the
authoritative full report. Add `--progress-detail file` only when the caller
needs per-file scan events and unsampled backend stage/metric events.

`inspect map --format ndjson` emits bounded report events before the terminal
`completed` event: after scan progress, one `map-entry` event per `top_entries`
item with `payload_kind = "inspect-map-entry"`, then one `map-group` event per
requested `groups` item with `payload_kind = "inspect-map-group"`. The final
`completed` event still carries the full `inspect-map` report, so consumers can
either stream ranked rows as they arrive or keep reading the last completed
payload as the authoritative whole-report snapshot.

`inspect map --table csv|tsv` is a command-specific raw table export, not a
JSON/NDJSON API envelope. It cannot be combined with `--format json` or
`--format ndjson`. The table has one header and flat `total`, `root`, `entry`,
and `group` rows using the same bounded `top_entries` and requested `groups` as
the report payload; empty cells mean the column is not applicable to that row
type or the metric is unknown. Repeated `--table-row total|root|entry|group`
flags can limit the export to selected row kinds; omitting them preserves the
full table. When `--cleanup-advice` or `--advice-status` is enabled, the table
appends cleanup columns for entry rows: status, relation, source, rule id,
category, safety level, required flags, required warnings, protection kind,
matched path, reason, a PowerShell-quoted dry-run command hint, and optional
`cleanup_app_*` context columns for app-leftover advice.

## Payload Kinds

The `payload_kind` field identifies the shape under `data`:

- `rule-catalog`
- `capabilities`
- `cli-schema`
- `cleanup-plan`
- `cleanup-receipt`
- `saved-cleanup-plan`
- `app-leftovers-cleanup-plan`
- `project-artifact-cleanup-plan`
- `catalog`
- `catalog-validation`
- `cache-inventory`
- `cache-doctor`
- `cache-prune-report`
- `inspect-space`
- `inspect-map`
- `inspect-map-entry`
- `inspect-map-group`
- `inspect-progress`
- `inspect-artifacts`
- `inspect-lint`
- `cache-purge-report`
- `trash-report`
- `history-list`
- `config-paths`
- `config-view`
- `config-validation`
- `rule-validation`
- `rule-import`
- `rule-import-list`
- `rule-import-mutation`
- `skill-management`
- `permissions-diagnostic`
- `active-process-diagnostic`

Payload data is intentionally nested under `data` so Rebecca can evolve
metadata, event transport, and error handling without turning internal core
models into the top-level API.

Cleanup and inspect targets include estimate provenance so consumers can explain
byte total trust without changing `estimated_bytes` arithmetic. `estimate_source`
remains the stable source field:

- `fresh-scan`: bytes came from a live filesystem scan during this command;
- `scan-cache`: bytes came from an enabled scan-cache hit whose root metadata,
  backend, confidence, and metric semantics were compatible with the current
  request;
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
- `estimate_caveats`: structured caveats with `code` and `message`;
- `estimate_backend_evidence`: optional structured evidence with `timings_ms`,
  `counters`, and `cache_events`. Consumers should prefer this object over
  parsing human caveat text when comparing scan/cache behavior.

Scan-cache miss evidence may include reasons such as `missing`, `stale`,
`expired`, `metadata-unavailable`, `incompatible-backend`, or
`incompatible-metric-semantics`. Incompatible records are retained rather than
pruned because they may still be valid for a future request using the original
backend or byte metric.

The `windows-ntfs-mft-experimental` backend is read-only, opt-in, and only live
in binaries compiled with the `ntfs` Cargo feature. When live NTFS metadata is
available, `estimate_backend_source` distinguishes the normal targeted
per-record FSCTL traversal source from explicit full-index diagnostic sources.
When the feature is disabled, live metadata is unavailable, or metadata is
ambiguous, Rebecca reports fallback provenance instead of treating raw metadata
as cleanup authority. Parser caveats may include sequence mismatches, hardlink
path candidates, resident or nonresident attribute-list handling, resident or
nonresident `$I30` directory-index fallback, unreadable or unsupported stream
expansion, or bounded parse-error summaries. Valid nonresident
`$ATTRIBUTE_LIST` and `$INDEX_ALLOCATION:$I30` metadata can supplement record
streams and subtree edges, but these fields are still explainability data; they
do not authorize deletion or change cleanup byte semantics.

`inspect-map` can also emit disk inventory caveats such as `compressed-file`,
`sparse-file`, `hardlink-file`, and `reparse-skipped`. Hardlink caveats mean
path-ranked logical and allocated bytes include each path. When stable file-id
metadata is available, `unique_logical_bytes` and `unique_allocated_bytes`
deduplicate those paths by backend identity, such as Unix `st_dev`/`st_ino` or
Windows `(volume serial, file index)`; otherwise the unique fields remain `null`
rather than mixing accounting modes.

Cleanup plans include `summary.warning_matrix` and warning-bearing targets carry
`warnings`. A target with `reason_code: "warning-gate-required"` was excluded
until the user selects the named gate with `--allow-warning <warning>`.

`active-process-diagnostic` is emitted by `rebecca doctor active-processes`.
It reports whether process inspection is available and lists running processes
that match cleanup rules carrying the `active-process` warning. Windows uses the
native process adapter; Linux reads `/proc/<pid>/comm` with `/proc/<pid>/exe` as
a fallback when those files are readable.

`permissions-diagnostic` is emitted by `rebecca doctor permissions`. It reports
the current platform, whether cleanup execution is supported on that platform,
the detected privilege level, and a short suggested action. Linux privilege
labels are derived from the effective UID in `/proc/self/status`; standard-user
Linux cleanup should stay preview-first and use elevated permissions only for
reviewed permission-sensitive system cache rules. macOS cleanup should stay
current-user and preview-first; the macOS payload adds a read-only
`macos_privacy` preflight with Mail, Messages, and Safari probe paths, each
reported as `readable`, `permission-denied`, `missing`, or `unknown`. Treat a
`likely-blocked` macOS privacy status as evidence to grant Full Disk Access to
the terminal only for reviewed user-owned cache paths, and do not treat `sudo`
as a TCC or Full Disk Access workaround.

`catalog` is emitted by `rebecca catalog`. The payload is a typed array of
cleanup rules, project artifact policies, warning gates, safety categories, and
supported action kinds. Cleanup rule entries include their generated `platform`
field, and `catalog --kind cleanup-rule --platform linux|windows|macos|unknown`
filters those entries before rendering the API envelope. `catalog-validation` is emitted by
`rebecca catalog validate`.

`capabilities` is emitted by `rebecca capabilities`. GUI wrappers should call it
before choosing workflows instead of hard-coding feature assumptions. It reports
the CLI API version, package version, current platform, compile-time features
such as `rules`, `windows`, and `ntfs`, available schema documents, recommended
startup commands, command payload kinds, NDJSON support, mutating commands,
platform availability, preflight commands, required execution flags, macOS
privacy relevance, and safety-model facts such as preview-by-default cleanup
and recoverable deletion.

A GUI startup sequence should stay read-only until the user confirms a cleanup:
call `capabilities --format json`, export the schemas it needs with
`schema export --document ...`, run `doctor permissions --format json`, then run
`config validate --format json` and any `rules validate --format json` checks
for user-selected manifests. Preview cleanup with `clean --dry-run --format json`
or NDJSON progress, and execute only by replaying the reviewed command with
`--yes` and the same rule, warning, safety, exclude, and scan-cache choices.

`cli-schema` is emitted by `rebecca schema export --document <name>`. It returns
one embedded JSON Schema document from this directory inside a normal API
envelope. Exportable documents are `envelope`, `event`, `error`, `payloads`,
`config`, and `cleaner-manifest-v1`. Use them when a GUI wants to validate
Rebecca payloads, config files, or external rule manifests at runtime.

`rule-validation` is emitted by `rebecca rules validate`. This command validates
external Cleaner Manifest v1 TOML files or directories before import. A success
payload means the manifests parse, rule ids and target specs are unique, warning
gates are known, protected target shapes are blocked, browser/state boundaries
are respected, target shapes have a positive cleanup basis, and dangerous safety
levels are rejected. Directory inputs are bounded by `--max-depth` and
`--max-files`, and Rebecca does not traverse symbolic links while discovering
manifests. The payload includes a `discovery` object so wrappers can show those
limits. Validation does not enable or import those rules; the payload includes
`enabled = false` to make that boundary explicit.

`rule-import`, `rule-import-list`, and `rule-import-mutation` are emitted by
`rebecca rules import`, `rules list`, `rules enable`, `rules disable`, and
`rules remove`. Import validates and copies a manifest into Rebecca-owned state,
records source display path, stored manifest path, content hash, import time,
rule ids, platforms, and `enabled = false`. Enable revalidates the stored
manifest before planning can consume it; corrupted or now-invalid stored
manifests fail closed instead of silently participating in cleanup planning.

`skill-management` is emitted by `rebecca skills install`, `skills path`, and
`skills remove`. The payload reports the selected agent preset, resolved
`skills_dir`, final `skill_dir`, operation status, whether the command was a
dry run, whether files changed, and whether the existing directory looked
Rebecca-managed. The default preset installs to `~/.agents/skills`; `--agent
codex` resolves `$CODEX_HOME/skills` or `~/.codex/skills`, and
`--destination <SKILLS_DIR>` is the explicit escape hatch for other agents.

`cache-inventory`, `cache-doctor`, and `cache-prune-report` are emitted by
`rebecca cache inspect`, `rebecca cache doctor`, and `rebecca cache prune`.
Inventory entries intentionally expose both `absolute_path` and `display_path`.
`absolute_path` is a local machine path and may include usernames or disk
layout; `display_path` is the value Rebecca uses for human-oriented diagnostics
and examples. `record_root` and NTFS cache identifiers are also local metadata.
Issue reports and dogfood artifacts should prefer display fields unless the
user explicitly needs full local evidence.

`inspect-space`, `inspect-map`, `inspect-artifacts`, and `inspect-lint` are
read-only cleanup intelligence reports. They inventory top-level space, ranked
disk-map usage, project artifact reclaim opportunities, or duplicate/large/empty
file findings without prompting, executing cleanup, writing history, or mutating
files. `inspect-map` uses path-ranked `logical_bytes` plus nullable
`allocated_bytes` instead of `estimated_bytes` because it is a disk inventory
surface rather than a cleanup estimate surface. It also exposes nullable
`unique_logical_bytes` and `unique_allocated_bytes` for backends that can
deduplicate stable file identities. Unix portable inventory fills allocation
from `st_blocks` and deduplicates hardlinks by `st_dev`/`st_ino`; Windows native
inventory fills file allocation bytes and file-id-deduplicated unique bytes when
the host API exposes them; NTFS/MFT inventory fills allocation from parsed stream
metadata and uses NTFS record identity for unique metrics when all counted files
have parser-backed evidence.
When callers pass one or more `--group-by type|extension|depth|age` flags,
`inspect-map` includes `groups`: bounded distribution summaries with
`kind`, stable `key`, human `label`, and the same `metrics` object used by roots
and entries. `--group-limit` bounds the combined group list across all requested
group kinds. Windows native and experimental NTFS/MFT disk-map inventory both
feed these groups from the same traversal that produces ranked entries; backend
fallback is reserved for ordinary backend unavailability.
`--sort logical|allocated|files|unique` changes the order of `top_entries`, and
`--group-sort logical|allocated|files|unique` changes the order of `groups`.
Unavailable allocated or unique metrics fall back to logical-byte ordering for
that rank value so portable reports remain deterministic and useful.
In NDJSON mode, those already-bounded `top_entries` and `groups` are also
emitted as ranked `map-entry` and `map-group` events before the final full
report.
For table-first tools, `--table csv|tsv` exports the same totals, roots, ranked
entries, and requested groups as a flat row set outside the JSON API envelope.
Repeated `--table-row` flags can narrow that row set when a caller only needs
entries, groups, or root summaries.
When callers pass `--cleanup-advice`, each ranked entry may include
`cleanup_advice`. Advice is read-only guidance derived from Rebecca's cleanup
rule catalog, project artifact policy, app-leftover discovery, and protection
policy; it is not deletion authority. Status values are `cleanable`, `maybe-cleanable`,
`contains-cleanable`, `protected`, and `unknown`. Rule-backed and
project-artifact-backed advice can include `rule_id`, `category`,
`safety_level`, `required_flags`, `required_warnings`, `matched_path`, `reason`,
and a structured `suggested_command`. App-leftover advice can also include an
`app_leftover` object with the installed app identity, app-data source, target
leaf, deletion style, and optional modification time. `--advice-status <status>` implies
`--cleanup-advice` and filters only the ranked entries, not root totals or
diagnostic summaries. CSV/TSV `cleanup_command` cells are PowerShell-quoted
human hints derived from the structured command, and app-leftover rows append
`cleanup_app_*` context columns; machine consumers should use JSON or NDJSON
`suggested_command` and `app_leftover` fields instead of reparsing table cells.

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

`config-view` and `config-validation` are emitted by `rebecca config show` and
`rebecca config validate`. `config show` returns the loaded TOML config plus the
effective runtime paths and policies. `config validate` checks the same parse,
schema-version, path, scan-cache, purge-root, and runtime path resolution rules
without writing configuration. Use these commands instead of editing or parsing
Rebecca's config file format directly.

Purge targets carry the same estimate provenance fields as cleanup targets.
Consumers should use the explicit `rule_id`, `status`, `reason_code`,
`estimate_source`, backend/source/confidence/caveat fields, and `project_artifact`
explanation fields. Provenance explains where a byte estimate came from; it is
not a freshness guarantee and is never deletion authority.

## Saved Cleanup Plans

`clean --dry-run --save-plan <FILE>`, `purge --dry-run --save-plan <FILE>`, and
`apps clean --dry-run --save-plan <FILE>` write a `saved-cleanup-plan` document.
The saved document contains the original dry-run `plan` plus per-target metadata
fingerprints. `plan inspect <FILE>` returns the saved document without touching
the filesystem. `plan run <FILE>` revalidates the current host and target
fingerprints but still does not delete anything. `plan run <FILE> --yes` moves
still-valid targets to trash by default, and `--permanent` bypasses trash.

If a target path disappears, changes type, changes file length, changes
modification time, or becomes a symlink/reparse point, Rebecca blocks or skips
that target and reports a stable reason code such as
`execution-target-missing`, `saved-plan-target-changed`, or
`safety-policy-blocked`. Callers should treat saved plans as review artifacts,
not as reusable delete scripts.

## Cleanup Receipts

`clean --yes --receipt <FILE>`, `purge --yes --receipt <FILE>`,
`apps clean --yes --receipt <FILE>`, and
`plan run <FILE> --yes --receipt <FILE>` write a `cleanup-receipt` document
after execution. The receipt records the command, platform, workflow, delete
mode, destination, summary, execution report, target outcomes, and next steps.
For recoverable cleanup, `destination` is `windows-recycle-bin` on Windows and
`system-trash` elsewhere; space remains pending until the user empties trash.
For permanent cleanup, `destination` is `permanent-delete` and the receipt
records that the system trash was bypassed.

## Examples

```powershell
rebecca capabilities --format json
rebecca schema export --document payloads --format json
rebecca scan --format json
rebecca clean --format json --category system
rebecca clean --dry-run --save-plan cleanup-plan.json --category system
rebecca plan inspect --format json cleanup-plan.json
rebecca plan run --format json cleanup-plan.json --yes --receipt cleanup-receipt.json
rebecca clean --format json --yes --category system --receipt cleanup-receipt.json
rebecca clean --format ndjson --scan-cache --category system
rebecca clean --format ndjson --progress-detail file --rule windows.user-temp
rebecca doctor active-processes --format json
rebecca purge --format json --root . --min-age-days 0
rebecca catalog --format json --kind warning
rebecca catalog --format json --kind cleanup-rule --platform linux
rebecca cache inspect --format json --namespace scan-cache
rebecca cache doctor --format json
rebecca cache prune --format json --namespace scan-cache --stale-only
rebecca config show --format json
rebecca config validate --format json
rebecca rules validate --format json --dir ./rules
rebecca skills install --format json --dry-run
rebecca inspect space --format json --root . --diagnostic-limit 100
rebecca inspect map --format json --root . --top 20 --max-depth 3 --sort logical --diagnostic-limit 100
rebecca inspect map --format ndjson --root . --top 20 --group-by type --group-by extension
rebecca inspect map --format ndjson --progress-detail file --root . --top 20
rebecca inspect map --table csv --table-row entry --table-row group --root . --top 20 --group-by type --group-by extension
rebecca inspect artifacts --format json --root . --min-age-days 0
rebecca inspect lint --format json --root .
rebecca doctor permissions --format json
```

Representative fixtures live in `examples/` and schemas live next to this
README. The schemas use JSON Schema Draft 2020-12.
