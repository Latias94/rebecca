# Rebecca CLI API v1

Rebecca treats machine-mode CLI output as a versioned API for GUI wrappers and
automation. Human text remains the default. Machine consumers should always
request `--format json` for final results or `--format ndjson` for long-running
cleanup workflows.

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

## Payload Kinds

The `payload_kind` field identifies the shape under `data`:

- `rule-catalog`
- `cleanup-plan`
- `app-leftovers-cleanup-plan`
- `project-artifact-cleanup-plan`
- `project-artifact-catalog`
- `cache-purge-report`
- `history-list`
- `config-paths`
- `permissions-diagnostic`

Payload data is intentionally nested under `data` so Rebecca can evolve
metadata, event transport, and error handling without turning internal core
models into the top-level API.

Project artifact cleanup targets include a `project_artifact` object when they
were discovered by `rebecca purge`. The object explains why the target was
eligible:

- `matched_context`: stable kebab-case rule context such as `node-project`,
  `target-project`, or `cachedir-tag`;
- `project_root`: directory whose project context was accepted;
- `project_anchor`: file or marker directory that justified the match, such as
  `package.json`, `Cargo.toml`, or `CACHEDIR.TAG`.

Rebecca does not emit confidence scores for purge targets. Consumers should use
the explicit `rule_id`, `status`, `reason_code`, and `project_artifact`
explanation fields.

## Examples

```powershell
rebecca scan --format json
rebecca clean --format json --category system
rebecca clean --format ndjson --scan-cache --category system
rebecca purge --format json --root . --min-age-days 0
rebecca doctor permissions --format json
```

Representative fixtures live in `examples/` and schemas live next to this
README. The schemas use JSON Schema Draft 2020-12.
