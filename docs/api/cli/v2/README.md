# Rebecca CLI API v2

API v2 is the cleanup-intelligence contract for read-only discovery surfaces.
Existing cleanup execution, purge execution, history, config, cache, and doctor
payloads remain on `rebecca.cli.v1` until they need a deliberate schema
migration.

Machine consumers should request `--format json` for one final success envelope
or `--format ndjson` when they want a completed event. Human text remains the
default and is not a stable parsing interface.

## Envelopes And Events

JSON success responses use `envelope.schema.json` with
`api_version = "rebecca.cli.v2"`.

NDJSON success responses use `event.schema.json`. The terminal event has
`event_kind = "completed"`, a v2 `payload_kind`, and the same `data` shape as
the JSON success envelope. Fatal CLI errors currently use Rebecca's global
structured error contract rather than a v2-specific error schema.

## Payload Kinds

The `payload_kind` field identifies the shape under `data`:

- `catalog`
- `inspect-space`
- `inspect-artifacts`
- `inspect-lint`

Schemas live next to this README and use JSON Schema Draft 2020-12.

## Catalog

`rebecca catalog --format json` emits a v2 success envelope with
`payload_kind = "catalog"`. The `data` array contains typed catalog items:

- `cleanup-rule`
- `project-artifact`
- `warning`
- `safety-category`
- `action-kind`

Filters are additive and mirror the CLI flags:

```powershell
rebecca catalog --format json --kind cleanup-rule --category browser
rebecca catalog --format json --kind project-artifact --artifact node-modules
rebecca catalog --format json --kind warning --warning active-process
rebecca catalog --format json --kind safety-category --category credentials
```

Catalog consumers should prefer this command over older one-purpose listing
commands when they need a complete view of cleanup rules, project artifacts,
warning gates, safety categories, and supported action kinds.

## Inspect Space

`rebecca inspect space --format json --root <PATH>` emits
`payload_kind = "inspect-space"`. It is a read-only directory inventory report
with root totals, top entries, byte-estimate provenance, and diagnostics.

`inspect space` never deletes files and does not append cleanup history. It is
intended for dashboards and wrappers that need space insight without selecting a
cleanup policy.

## Inspect Artifacts

`rebecca inspect artifacts --format json --root <PATH>` emits
`payload_kind = "inspect-artifacts"`. It is the canonical read-only project
artifact insight command. It shares purge discovery, selectors, depth, age,
exclude, scan-cache, warning-gate, reclaim-limit, and diagnostic behavior, but
does not accept `--yes`, prompt, execute cleanup, or write history.

`rebecca purge inspect` is retained as a legacy compatibility alias for the
same read-only report. New automation should use `inspect artifacts`.

## Inspect Lint

`rebecca inspect lint --format json --root <PATH>` emits
`payload_kind = "inspect-lint"`. The report identifies duplicate groups, large
files, empty files, and empty directories through the shared inventory layer.

The command is intentionally report-only. It does not delete duplicates, choose
remediation actions, mutate hardlinks, shred files, or write cleanup history.
`--reference <PATH>` marks roots that should be kept when estimating duplicate
reclaim potential, and protected paths are also treated as keep candidates.
