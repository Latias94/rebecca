---
type: "Verification Evidence"
title: "Mole-Parity Completion Review"
description: "Requirement-by-requirement review of Rebecca's Windows-first cleanup safety governance against the Mole-like parity goal."
tags: ["mole-parity", "security", "cleanup", "verification"]
timestamp: 2026-06-24T14:37:58Z
status: "complete"
related_plan: "../../../plans/2026-06-24-018-refactor-mole-parity-safety-governance-plan.md"
git_branch: "feat/windows-cleanup-mvp"
---

# Summary

Rebecca has reached the intended Mole-like safety and maintainability posture
for its Windows-first cleanup scope. The comparison target is Mole's safety
posture, not macOS feature parity or Mole's implementation. The current code
and docs prove the core cleanup-system objective: bounded cleanup, shared
protection policy, protected categories, execution revalidation, catalog
governance, privacy-limited history/audit surfaces, CLI contract coverage, and
public security reporting guidance.

The remaining non-blocking work is distribution-layer maturity, such as release
artifact attestations and installer integrity. That work is useful, but it is
outside the cleanup-system objective and was already deferred by the
Mole-parity roadmap.

# Requirement Evidence

| Requirement | Status | Evidence |
|-------------|--------|----------|
| R1 central protection model | Proven | `crates/rebecca-core/src/protection.rs` defines `ProtectionPolicy`, never-delete roots, Rebecca-owned storage, protected categories, and allowlisted maintenance paths. `crates/rebecca-core/tests/safety_policy.rs` covers roots, storage, allowlists, and protected categories. |
| R2 protected sensitive data families | Proven | `ProtectedCategory` covers credentials, VPN/proxy state, AI/coding durable state, browser private data, cloud-synced data, container/runtime state, startup automation, and application durable data. The audit lists the same categories in `docs/security-audit.md`. |
| R3 execution-time revalidation | Proven | `execute_cleanup_plan_with_policy` in `crates/rebecca-core/src/executor.rs` reassesses executable targets before backend deletion. `crates/rebecca-core/tests/executor_contract.rs` proves protected category targets, Rebecca-owned storage, and missing targets do not reach the backend. |
| R4 built-in catalog governance | Proven | `crates/rebecca-rules/src/lib.rs` validates Windows platform, `windows.` ids, restore hints, owned provenance, project-owned license, duplicate target specs, and target shapes through `ProtectionPolicy::assess_catalog_target_shape`. `cargo nextest run -p rebecca-rules` covers the contract. |
| R5 guarded rule expansion | Proven | The Slack expansion batch added `windows.slack-cache` with safety/planner near-miss tests and scan/clean CLI contracts. Existing Steam, Electron, browser, Cargo, and JetBrains patterns show the same rule-family discipline. |
| R6 dry-run and additive JSON contract | Proven | `clean --dry-run` and real cleanup share planner construction. `crates/rebecca-core/tests/model_contract.rs`, `crates/rebecca-cli/tests/cli_clean.rs`, and `crates/rebecca-cli/tests/cli_history.rs` cover JSON shape, reason codes, restore hints, and legacy compatibility. |
| R7 human output and privacy-limited history | Proven | Human clean/history output surfaces issue matrices, target-scoped reasons, and restore hints. History persists request metadata, paths, byte counts, statuses, reason codes, issue matrices, and restore hints, but not file contents or child listings. |
| R8 living safety audit | Proven | `docs/security-audit.md` documents destructive-operation boundaries, protected categories, rule governance, execution controls, dry-run/history behavior, known limitations, and verification coverage. Root `SECURITY.md` adds reporting guidance. |
| R9 Mole-like Windows-first scope | Proven | `docs/plans/2026-06-24-018-refactor-mole-parity-safety-governance-plan.md` defines parity as comparable safety posture for Windows cleanup, not Mole feature parity. The current evidence satisfies that scoped definition. |

# Mole Audit Category Comparison

| Mole category | Rebecca status |
|---------------|----------------|
| Destructive path validation | Matched in Windows form through `ProtectionPolicy`, planner checks, execution revalidation, and tests. |
| Protected roots and protected categories | Matched in Windows form with filesystem roots, critical Windows paths, user profile roots, Rebecca-owned storage, and sensitive data categories. |
| Symlink, traversal, and reparse handling | Matched for cleanup scope through traversal rejection and existing reparse-like path blocking before deletion. |
| Privilege and sudo boundaries | Not applicable by design. Rebecca does not auto-elevate and currently returns a platform error for non-Windows execution. |
| Preview and audit logging | Matched through dry-run, stable JSON, history JSONL, issue matrices, and human history replay. |
| Sensitive data exclusions | Matched for current Windows cleanup scope through protected categories and allowlisted maintenance subpaths. |
| Release/security public signals | Partially matched. `SECURITY.md` and `docs/security-audit.md` now exist. Release artifact attestations and installer integrity remain future distribution-layer work. |

# Completion Decision

The Windows-first cleanup-system objective is complete. No remaining required
work is needed for Rebecca's current cleanup rule catalog, protected-path model,
execution boundaries, history/audit surfaces, or CLI contracts to be considered
Mole-like for the scoped Windows cleanup product.

Future work should be tracked as new bounded plans rather than continuing this
goal indefinitely:

- add release artifact attestations and installer integrity checks when Rebecca
  has a formal distribution pipeline;
- expand protected categories as new app families are added;
- continue adding cleanup rules only in guardrailed batches with unsafe
  near-miss tests, CLI contracts, and audit updates;
- create a separate cross-platform plan if Rebecca expands beyond Windows.

# Verification Commands

- `cargo fmt --all -- --check`
- `cargo nextest run -p rebecca-core --test safety_policy`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run -p rebecca-rules`
- `cargo nextest run -p rebecca-cli --test cli_scan`
- `cargo nextest run -p rebecca-cli --test scan`
- `cargo nextest run -p rebecca-cli --test cli_clean`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `git diff --check`
- `python C:\Users\Frankorz\.codex\skills\engineering-wiki-memory\scripts\wiki_memory.py validate --root docs\knowledge\engineering`

# Citations

- [Mole security policy](../../../../repo-ref/Mole/SECURITY.md)
- [Mole security audit](../../../../repo-ref/Mole/SECURITY_AUDIT.md)
- [Rebecca safety audit](../../../security-audit.md)
- [Mole-parity roadmap](../../../plans/2026-06-24-018-refactor-mole-parity-safety-governance-plan.md)
- [Rule authoring guide](../../../rule-authoring.md)
- [Configuration contract](../../../configuration.md)
