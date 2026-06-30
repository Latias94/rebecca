# Rebecca CLI API v2

API v2 starts with the unified catalog surface. Existing cleanup, purge,
history, config, and doctor payloads remain on `rebecca.cli.v1` until they need
a deliberate schema migration.

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
```

Schemas live next to this README and use JSON Schema Draft 2020-12.
