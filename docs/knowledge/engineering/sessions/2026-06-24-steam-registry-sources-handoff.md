---
type: "Session Handoff"
title: "Steam registry source table consolidation"
description: "Session summary for consolidating Steam discovery into an ordered registry source table."
tags: ["engineering-memory", "steam", "windows", "registry", "refactor"]
timestamp: 2026-06-24T05:17:21+08:00
status: "active"
source_session: "019ef4a0-c767-7942-af98-f2b0494e4c5f"
git_branch: "feat/windows-cleanup-mvp"
git_commit: "07fafa9"
verified_by: "cargo fmt --all; cargo nextest run -p rebecca-windows; cargo nextest run -p rebecca-core --test discovery; cargo nextest run -p rebecca-cli --test info; cargo nextest run --workspace"
---

# Summary

Steam discovery was consolidated into a single ordered registry source table in `rebecca-windows`.
The ordered sources now make the fallback precedence explicit and keep the registry lookup seam narrow:

1. `HKCU\Software\Valve\Steam\SteamPath`
2. `HKCU\Software\Valve\Steam\SteamExe`
3. `HKLM\SOFTWARE\Valve\Steam\InstallPath`
4. `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\InstallPath`
5. `HKCR\steam\Shell\Open\Command`

The refactor also pins the source ordering in tests, so future changes to Steam discovery precedence are deliberate.

# Verified State

- `cargo fmt --all` passed.
- `cargo nextest run -p rebecca-windows` passed.
- `cargo nextest run -p rebecca-core --test discovery` passed.
- `cargo nextest run -p rebecca-cli --test info` passed.
- `cargo nextest run --workspace` passed with 122 tests.
- The refactor commit is `07fafa9` with subject `refactor(windows): consolidate steam registry sources`.

# Open Threads

- Steam discovery remains intentionally best-effort and conservative.
- Remaining follow-up work, if any, should stay on narrow discovery or cleanup edges rather than re-expanding the fallback surface.

# Next Action

Continue with the next narrow Steam follow-up slice, or shift back to rule/catalog work if no further discovery edge needs tightening.

# Citations

- [Current state](../current-state.md)
- [Update log](../log.md)
- [Steam discovery implementation](../../../crates/rebecca-windows/src/steam.rs)
