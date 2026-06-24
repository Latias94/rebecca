---
type: "Session Handoff"
title: "Steam install localization and package info caches"
description: "Session Handoff for adding the Steam install-root localization and package info cache rules and extending the shared Steam test fixture table."
timestamp: 2026-06-24T02:53:43Z
tags: ["steam", "rules", "tests", "docs", "session"]
source_session: "019ef3ad-ccc1-7051-873a-9835b8b8f6ac"
---

# Summary

Added conservative `windows.steam-install-localization-cache` and `windows.steam-install-packageinfo-cache` rules for Steam's install-root `appcache\\localization.vdf` and `appcache\\packageinfo.vdf` files, and extended the shared Steam test helper so the file-target table still drives the CLI and planner regressions.

# Verified State

- `cargo fmt --all` has passed.
- `cargo clippy --workspace --all-targets -- -D warnings` has passed.
- `cargo nextest run --workspace` has passed with 132 tests.
- Targeted `cargo nextest run -p rebecca-rules -p rebecca-core --test planner -p rebecca-cli --test scan -p rebecca-cli --test cli_scan -p rebecca-cli --test cli_clean` has passed.

# Open Threads

None. These are conservative appcache file rules with the same best-effort discovery shape as the existing Steam install-root cache entries.

# Next Action

The tree is green and ready to commit.

# Citations

- [New localization rule](../../../../crates/rebecca-rules/rules/windows/steam-install-localization-cache.toml)
- [New package info rule](../../../../crates/rebecca-rules/rules/windows/steam-install-packageinfo-cache.toml)
- [Shared Steam test data](../../../../tests/common/steam.rs)
- [README](../../../../README.md)
- [Rule authoring guide](../../../../docs/rule-authoring.md)
