---
type: "Session Handoff"
title: "Steam install appinfo cache"
description: "Session Handoff for adding the Steam install appinfo cache rule and file-target Steam fixtures."
timestamp: 2026-06-24T02:48:10Z
tags: ["steam", "rules", "tests", "docs", "session"]
source_session: "019ef3ad-ccc1-7051-873a-9835b8b8f6ac"
---

# Summary

Added a conservative `windows.steam-install-appinfo-cache` rule for Steam's install-root `appcache\\appinfo.vdf`, and deepened the shared Steam test helper so it can write both directory fixtures and file fixtures from the same table-driven cases.

# Verified State

- `cargo fmt --all` has passed.
- `cargo clippy --workspace --all-targets -- -D warnings` has passed.
- `cargo nextest run --workspace` has passed with 132 tests.
- Targeted `cargo nextest run -p rebecca-rules -p rebecca-core --test planner -p rebecca-cli --test scan -p rebecca-cli --test cli_scan -p rebecca-cli --test cli_clean` has passed.

# Open Threads

Steam cleanup hardening can continue if another conservative install-root cache file or directory appears; otherwise this slice is a reasonable place to stop.

# Next Action

Continue the Steam cleanup expansion only if the next candidate is equally conservative and backed by local evidence; otherwise keep tightening adjacent contracts and docs.

# Citations

- [New appinfo rule](../../../../crates/rebecca-rules/rules/windows/steam-install-appinfo-cache.toml)
- [Shared Steam test data](../../../../tests/common/steam.rs)
- [README](../../../../README.md)
- [Rule authoring guide](../../../../docs/rule-authoring.md)
