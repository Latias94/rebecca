---
type: "Session Handoff"
title: "Steam test deepening"
description: "Session Handoff for extracting shared Steam test data into repo-level helpers."
timestamp: 2026-06-24T02:20:14Z
tags: ["steam", "tests", "refactor", "session"]
source_session: "019ef3ad-ccc1-7051-873a-9835b8b8f6ac"
---

# Summary

把 Steam 相关的 CLI 和 core 回归测试里的重复表格抽成仓库级共享 helper，减少 `scan` / `clean` / planner 三处各写一份相同案例的重复。

# Verified State

- `cargo fmt --all` 已通过。
- `cargo clippy --workspace --all-targets -- -D warnings` 已通过。
- `cargo nextest run --workspace` 已通过，132 个测试全部通过。
- 相关定点测试 `cargo nextest run -p rebecca-core --test planner -p rebecca-cli --test scan -p rebecca-cli --test cli_scan -p rebecca-cli --test cli_clean` 已通过。

# Open Threads

继续沿着 Steam cleanup 的剩余边界做后续切片；如果再加新规则，优先复用这次抽出的 Steam 测试数据层。

# Next Action

在下一个会话里继续 Steam 的发现或规则边界硬化，沿用共享的 Steam 测试 helper，避免再散落重复测试表。

# Citations

- [Shared Steam test helper](../../../../tests/common/steam.rs)
- [CLI common tests](../../../crates/rebecca-cli/tests/common/mod.rs)
- [Planner tests](../../../crates/rebecca-core/tests/planner.rs)
- [CLI clean tests](../../../crates/rebecca-cli/tests/cli_clean.rs)
- [CLI scan tests](../../../crates/rebecca-cli/tests/cli_scan.rs)
