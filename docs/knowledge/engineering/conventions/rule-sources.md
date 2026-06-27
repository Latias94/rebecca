---
type: "Repo Convention"
title: "Rule Sources"
description: "Reference sources and license boundaries for Rebecca built-in cleanup rule authoring."
tags: ["engineering-memory", "rule-authoring", "provenance"]
status: "active"
---

# Rule Sources

This directory records upstream projects and datasets that can inform Rebecca
rule authoring. It is a reference index, not a copy of external rule data.

## Usage Rules

- Record the upstream project name, repository or file path, license, and
  revision reference for every external source you use.
- Put the same source summary in the rule's `provenance.notes`.
- GPL sources may inform behavior and safety boundaries, but their rule
  definitions and code must not be copied into Rebecca.
- If reuse terms are unclear, treat the source as behavior-only reference and
  rewrite the rule from scratch.

## Current Sources

| Source | License | Repository / file path | Revision | Notes |
|--------|---------|------------------------|----------|-------|
| Mole | GPL-3.0-or-later | `repo-ref/Mole` | `6be20eaa1eacc78d0355b4ed4744bb7a08447704` | Behavior and safety benchmark only. Reference paths and UX, not rule text. |
| BleachBit | GPL-3.0-or-later | `repo-ref/bleachbit/cleaners/*.xml` | `1517daf22201e4f8b05fbffcc4f89c992ba06375` | Cleaner behavior reference only. Use `cleaners/*.xml` and docs for path ideas, not copied rule data. |
| Winapp2.ini | CC-BY-SA-4.0 / reference-only | `MoscaDotTo/Winapp2/Non-CCleaner/Winapp2.ini` | 2026-06-27 review | Use as a discovery index for Windows cleanup candidates only; rewrite every rule from scratch. Useful domestic-app entries include Tencent WeChat, Tencent WeChat Work, and Kingsoft Office. |
| windows-cleaner-cli | MIT | `repo-ref/windows-cleaner-cli` | `ee03ebd94ee1bc6de32fc226ecef488c7bbfd7c5` | Useful for Windows maintenance cache categories such as temp files, Prefetch, update downloads, and browser/system cache comparisons. |
| null-e | WTFPL-2.0 | `repo-ref/null-e` | `079a038f71159dab07c4d2bd8bd700cb5647972d` | Useful batch reference for developer-cache families such as npm, pip, cargo, uv, Poetry, Docker, Android, IDE, ML/AI caches, and Electron app candidates including Postman, Notion, and Figma. Behavior reference only. |
| Bulk Crap Uninstaller | Apache-2.0 | `repo-ref/Bulk-Crap-Uninstaller/Licence.txt` | `f39663316ad5d593c4d160b0445841ce7eb6a35f` | Useful for uninstall and leftovers modeling; not a rule source. |
| Hugging Face Hub | Apache-2.0 | `huggingface/huggingface_hub/src/huggingface_hub/constants.py` and `package_reference/environment_variables` | `1e41293da4a0b1e5ea1afab85d3701843aa4b3bc` | Verified cache-root behavior for HF_HOME, HF_HUB_CACHE, HF_ASSETS_CACHE, and HF_XET_CACHE. |
| Hugging Face Datasets | Apache-2.0 | `huggingface/datasets/src/datasets/config.py` | `b713dcdffa92ada37c569e6f1419ce94fc170b0c` | Verified HF_DATASETS_CACHE and the dataset cache layout under HF_HOME. |
| PyTorch | BSD-style | `pytorch/pytorch/torch/hub.py` | `0a8f331c4de50a57643fb72b692dbc6a41b12297` | Verified Torch Hub cache root behavior and the default checkpoints subdirectory via get_dir and load_state_dict_from_url. |
| Android Studio / Android tools | Android docs license / JetBrains reference-only | Android tool environment variable docs and JetBrains IDE cache directory docs | 2026-06-26 docs review | Verified the `.android` user-home cache boundary and Android Studio cache directory shape; SDK packages, AVDs, keys, licenses, and IDE settings are durable state. |

## BleachBit Windows Coverage Notes

BleachBit's Windows cleaners are the highest-signal external reference for
future rule batches. The reusable families are browser caches, Electron-like
apps, Office-style application caches, and a smaller set of Windows utility
cleaners. Linux-only cleaners are generally not relevant to Rebecca's current
scope.

Candidates worth batch review:

- Developer cache families such as Cargo, npm, pnpm, Yarn, pip, uv, Poetry, Conda, Gradle, Maven, and IDE caches
- Chromium-family browsers and derivatives
- Firefox-family profile caches
- Electron-based apps such as Discord, Slack, Teams-like apps, and editor
  shells
- Office and document-app caches when they are stable and user-scoped
- Windows utilities such as Explorer, Media Player, Defender, and similar
  regenerated caches
- Windows maintenance caches such as temp files, Prefetch, and update
  download directories
- Uninstall leftovers and app inventory modeling

## Domestic Windows Desktop App Notes

The first domestic desktop-app cache batch used Winapp2 as a discovery index,
Mole as a behavior benchmark where it has comparable macOS app-cache coverage,
Bulk Crap Uninstaller as an uninstall-leftover boundary reference, and local
Windows AppData layout inspection for cache-leaf confirmation. Rebecca keeps
the resulting rules project-owned and intentionally narrow:

- Tencent apps: WeChat `radium`/mini-program cache leaves, WXWork cache crash
  artifacts, QQ `Cache`, Tencent Meeting / WeMeet dynamic-resource caches,
  QQ Music cache leaves, and Tencent Video / QQLive cache leaves.
- Collaboration apps: Feishu `LarkShell` cache and shader-cache leaves, plus
  DingTalk `Cache` and `resource_cache`.
- Document and sync apps: WPS HTTP/file-cache leaves and Baidu Netdisk's
  Local AppData `cache` leaf only.

Do not turn these into app-root, account-root, document-root, sync-root, or
session-storage cleanup rules.

## Rule Family Trace Template

Use this shape in `provenance.notes`:

```text
Derived from <upstream project> (<repo or file path>, <license>, <revision>).
Rewritten for Rebecca; behavior-only reference.
```
