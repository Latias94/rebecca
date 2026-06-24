---
type: "Session Handoff"
title: "Steam install stats cache"
description: "Session Handoff for adding the Steam install stats cache rule."
timestamp: 2026-06-24T02:34:14Z
tags: ["steam", "rules", "tests", "docs", "session"]
source_session: "019ef3ad-ccc1-7051-873a-9835b8b8f6ac"
---

# Summary

Added a conservative `windows.steam-install-stats-cache` rule for Steam's install-root `appcache\\stats`, and kept the shared Steam test data layer aligned so the new rule shows up in planner, scan, CLI, README, and rule-authoring coverage.

# Verified State

- `cargo fmt --all` has passed.
- `cargo clippy --workspace --all-targets -- -D warnings` has passed.
- `cargo nextest run --workspace` has passed with 132 tests.
- Targeted `cargo nextest run -p rebecca-rules -p rebecca-core --test planner -p rebecca-cli --test scan -p rebecca-cli --test cli_scan -p rebecca-cli --test cli_clean` has passed.

# Open Threads

Steam cleanup hardening can continue with remaining conservative boundaries if another defensible install-root cache point appears.

# Next Action

Continue the Steam cleanup expansion only if the next candidate is equally conservative and backed by local evidence; otherwise keep tightening adjacent contracts and docs.

# Citations

- [New stats rule](../../../../crates/rebecca-rules/rules/windows/steam-install-stats-cache.toml)
- [Shared Steam test data](../../../../tests/common/steam.rs)
- [README](../../../../README.md)
- [Rule authoring guide](../../../../docs/rule-authoring.md)
