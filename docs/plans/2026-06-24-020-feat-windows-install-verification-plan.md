---
title: "feat: Add Windows install-time verification"
type: "feat"
date: "2026-06-24"
---

# feat: Add Windows install-time verification

## Summary

Rebecca now has release artifacts, checksums, and build-provenance
attestations. This plan adds the Windows install/update entry point that
consumes that contract: download a tagged release ZIP, verify `SHA256SUMS`,
optionally require GitHub attestation verification, and install `rebecca.exe`
into a user-controlled directory.

The install script should be conservative and transparent. It must never bypass
checksum verification, and it should make attestation policy explicit instead
of silently treating checksum-only installation as equivalent to provenance.

---

## Problem Frame

Release integrity is only useful if users and maintainers have a reliable way
to consume it. The current `docs/release.md` tells users how to verify a ZIP
manually, but it does not provide a repeatable install/update path.

Mole's installer verifies release checksums and can enforce attestation when the
GitHub CLI is available. Rebecca should adopt the same trust posture in a
Windows PowerShell shape while keeping the first installer local, auditable, and
free of package-manager assumptions.

---

## Requirements

**Install Contract**

- R1. The installer must support a pinned release tag or the latest GitHub
  release.
- R2. The installer must download the Windows x86_64 ZIP and `SHA256SUMS` from
  the same release.
- R3. The default install directory must be user-writable and overridable.
- R4. Re-running the installer against the same directory must replace the
  installed binary atomically enough for normal user-level updates.

**Verification Contract**

- R5. Checksum verification is mandatory and must run before extraction or
  install.
- R6. Attestation verification must be supported through GitHub CLI when
  available.
- R7. `-RequireAttestation` must fail closed if GitHub CLI is missing,
  unauthenticated, or verification fails.
- R8. The installer must not report checksum-only installation as provenance
  verified.

**Safety And UX**

- R9. The installer must fail before modifying the install directory when
  required inputs cannot be resolved or verified.
- R10. The installer must avoid deleting arbitrary paths; cleanup must be
  limited to installer-owned temp/staging directories and files under the
  target install directory.
- R11. Documentation must explain install, update, PATH, checksum-only, and
  attestation-required modes.

---

## Key Technical Decisions

- KTD1. Use a repo-local PowerShell script (`scripts/install.ps1`) instead of a
  native installer. This keeps the first install path reviewable and aligned
  with Rebecca's Windows-first tooling.
- KTD2. Default to `%LOCALAPPDATA%\Programs\Rebecca` as the install directory.
  It is user-writable and avoids administrator prompts.
- KTD3. Use direct GitHub release asset URLs for downloads and GitHub CLI only
  for optional attestation verification. This keeps checksum-only installs
  possible on machines without `gh`.
- KTD4. Require an explicit repository slug through `-Repository`, environment,
  or git-remote inference. The script must not ship with a fake `OWNER/REPO`
  default.
- KTD5. Preserve a simple update story: running the installer again with a newer
  tag replaces `rebecca.exe` and writes install metadata.

---

## Scope Boundaries

### In Scope

- `scripts/install.ps1` for Windows release install/update.
- Mandatory checksum verification and optional required attestation.
- README and release-guide documentation for install usage.
- Security audit updates for install-time verification.
- Local smoke tests using the already generated release-smoke ZIP.

### Deferred To Follow-Up Work

- MSI/MSIX installer packaging.
- PATH registry mutation automation.
- Scoop, winget, Chocolatey, or other package-manager manifests.
- Auto-update command inside `rebecca.exe`.
- Multi-architecture installer selection.

### Outside This Product's Identity

- Installing unverified artifacts.
- Copying Mole's Bash installer implementation.
- Editing system-wide PATH or privileged directories by default.

---

## Implementation Units

### U1. Add Windows Install Script

- **Goal:** Create a PowerShell install/update script that downloads, verifies,
  extracts, and installs the current Windows release artifact.
- **Requirements:** R1, R2, R3, R4, R5, R9, R10
- **Files:** `scripts/install.ps1`
- **Related files:** `scripts/release/build-release.ps1`,
  `.github/workflows/release.yml`
- **Approach:** Resolve repository and release tag, download ZIP and
  `SHA256SUMS` into a temp directory, verify the checksum, extract into a temp
  staging directory, then copy `rebecca.exe` and release metadata into the
  install directory.
- **Test scenarios:**
  - A local release-smoke ZIP installs successfully when the checksum matches.
  - A checksum mismatch fails before the install directory is modified.
  - Missing ZIP, missing checksum entry, or unsupported artifact names fail
    clearly.
  - `-InstallDir` can target a temp directory for smoke tests.

### U2. Add Attestation Policy Controls

- **Goal:** Support GitHub CLI provenance verification without making `gh`
  mandatory for checksum-only installs.
- **Requirements:** R6, R7, R8
- **Files:** `scripts/install.ps1`, `docs/release.md`, `SECURITY.md`
- **Approach:** Run `gh attestation verify <asset> --repo <owner/repo>
  --deny-self-hosted-runners` when `gh` is available or
  `-RequireAttestation` is set. Treat a failed attestation as fatal when
  required; otherwise surface that only checksum verification was performed.
- **Test scenarios:**
  - `-RequireAttestation` fails when `gh` is not available.
  - Optional attestation failures do not bypass checksum verification and do not
    claim provenance success.
  - Required attestation runs after checksum verification but before install.

### U3. Document Install And Update Usage

- **Goal:** Make the install path clear enough for early users and future
  package-manager work.
- **Requirements:** R8, R11
- **Files:** `README.md`, `docs/release.md`, `docs/security-audit.md`
- **Approach:** Document pinned-version install, latest-release install,
  update-by-rerun, default install directory, PATH guidance, and attestation
  policy.
- **Test scenarios:**
  - Commands use real script parameters.
  - Documentation distinguishes checksum-only from attestation-required mode.
  - Security docs keep installer problems in the release/install trust surface.

### U4. Record Continuity

- **Goal:** Leave durable state for the next distribution slice.
- **Requirements:** R11
- **Files:** `docs/knowledge/engineering/current-state.md`,
  `docs/knowledge/engineering/log.md`
- **Approach:** Record implemented script behavior, verification results, and
  remaining follow-up work after the smoke tests pass.
- **Test scenarios:** Test expectation: none -- this is memory maintenance, but
  the claims must cite the committed script and release docs.

---

## Sources / Research

- `docs/plans/2026-06-24-019-feat-release-integrity-and-distribution-plan.md`
- `docs/release.md`
- `.github/workflows/release.yml`
- `repo-ref/mole/install.sh`
- `repo-ref/mole/tests/install_checksum.bats`
- `repo-ref/mole/SECURITY_AUDIT.md`
- `SECURITY.md`
