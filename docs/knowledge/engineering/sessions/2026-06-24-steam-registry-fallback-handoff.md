---
type: "Session Handoff"
title: "Steam registry fallback hardening"
description: "Session summary for the Steam discovery registry fallback and verification pass."
tags: ["engineering-memory", "steam", "windows", "discovery", "verification"]
timestamp: 2026-06-24T04:57:04+08:00
status: "active"
source_session: "019ef4a0-c767-7942-af98-f2b0494e4c5f"
git_branch: "feat/windows-cleanup-mvp"
git_commit: "71e2988"
verified_by: "cargo fmt --all; cargo nextest run -p rebecca-windows --test recycle_bin; cargo nextest run -p rebecca-core --test discovery; cargo nextest run -p rebecca-cli --test info; cargo nextest run --workspace"
---

# Summary

本轮继续推进 Steam cleanup 扩展，重点把 Steam discovery 的 registry fallback 补齐并收紧。
`rebecca-windows` 现在会按顺序尝试：

1. `HKCU\Software\Valve\Steam` 的 `SteamPath`
2. `HKLM\SOFTWARE\Valve\Steam` 的 `InstallPath`
3. `HKLM\SOFTWARE\WOW6432Node\Valve\Steam` 的 `InstallPath`
4. `HKCR\steam\Shell\Open\Command` 的默认命令并解析可执行文件路径

同时把命令解析提成了纯函数，补了单元测试，避免后续再次在 registry API 上回退。

# Verified State

- `cargo fmt --all` 通过。
- `cargo nextest run -p rebecca-windows --test recycle_bin` 通过。
- `cargo nextest run -p rebecca-core --test discovery` 通过。
- `cargo nextest run -p rebecca-cli --test info` 通过。
- `cargo nextest run --workspace` 通过，117 个测试全绿。
- 变更已提交，commit 为 `71e2988`，提交信息是 `fix(windows): add steam registry fallback`。

# Open Threads

- 继续观察 Steam discovery 的真实边界，后续可考虑把 registry fallback 再拆成更细的可测试层。
- 当前 goal 仍然是长期持续推进，不需要在这里收口为完成。

# Next Action

继续沿着 Steam cleanup / discovery 方向做下一步无畏重构，优先挑选能继续提升发现准确性或减少平台差异脆弱性的切片。

# Citations

- [Current state](../current-state.md)
- [Update log](../log.md)
- [Windows application discovery](../../../crates/rebecca-windows/src/steam.rs)
