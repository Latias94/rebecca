# Security Policy

Rebecca is a local Windows cleanup tool. Its highest-risk behavior is
unintended local data loss from cleanup targets, path expansion, execution
boundaries, history persistence, or release/install trust.

For the current cleanup safety model, see
[Rebecca Cleanup Safety Audit](docs/security-audit.md).

## Reporting A Vulnerability

Please report suspected security issues privately to the project maintainer.
Do not open a public issue for an unpatched vulnerability.

If the repository's private security advisory channel is enabled, use that
channel. Otherwise, contact the maintainer through the private channel listed
on the project hosting profile.

Include as much of the following as possible:

- Rebecca version or commit
- Windows version
- exact command or workflow involved
- reproduction steps or proof of concept
- whether the issue involves cleanup boundaries, path validation, reparse
  points, protected data categories, history/audit data, or release/install
  integrity

## Supported Versions

Security fixes are prioritized for:

- the current `main` branch
- the latest published release, when releases exist

Older releases may not receive security fixes. Users running cleanup commands
should stay current.

## What We Consider A Security Issue

Security-relevant issues include:

- path validation bypasses
- deletion outside intended cleanup boundaries
- unsafe handling of symlinks, junctions, reparse points, or traversal
- sensitive data removal that bypasses documented protections
- cleanup history or cache persistence of secrets, file contents, credentials,
  tokens, browser databases, or arbitrary child-file listings
- release, installation, update, or checksum integrity issues
- logic defects that can cause unintended destructive behavior

## What Usually Does Not Qualify

The following are usually normal bugs or feature requests unless they create a
plausible security impact:

- cleanup misses that leave recoverable cache data behind
- false negatives where Rebecca refuses to clean a path
- requests for broader or more aggressive cleanup behavior
- cosmetic CLI output issues
- compatibility issues without a destructive-operation or data-exposure risk

When unsure, report privately first.

## Security-Focused Areas

Rebecca pays particular attention to:

- destructive command boundaries
- protected roots and protected data categories
- rule catalog target-shape validation
- dry-run and execution parity
- execution-time target revalidation
- symlink, junction, reparse-point, and traversal handling
- history and scan-cache privacy boundaries
- release and install trust signals

## Release Integrity

Official release artifacts should come from the repository's GitHub Releases
workflow and be accompanied by `SHA256SUMS` and GitHub build-provenance
attestations. Verification guidance lives in
[docs/release.md](docs/release.md).

The PowerShell installer verifies `SHA256SUMS` before extraction and supports
`-RequireAttestation` for fail-closed GitHub CLI provenance verification.

Report release-integrity problems privately when:

- a published asset has no matching checksum entry;
- a checksum does not match the downloaded asset;
- `gh attestation verify` fails for a published release asset;
- provenance indicates an unexpected repository or self-hosted runner;
- an installer or package-manager manifest points at an unverified artifact.
