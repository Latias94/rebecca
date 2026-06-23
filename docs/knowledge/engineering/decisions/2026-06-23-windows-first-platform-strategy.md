---
type: "Decision"
title: "Windows-first product strategy"
description: "Platform decision for the Rust CLI cleaner."
tags: ["decision", "windows", "strategy", "cleaner"]
timestamp: 2026-06-23T09:20:00Z
status: "proposed"
---

# Decision

The product will be Windows-first. Linux support may exist later through shared core abstractions, but it is not the primary launch target.

# Context

The requested tool is a Rust + clap CLI for cleaning Windows system junk and software caches. The strongest market fit is Windows, where cache locations, uninstall leftovers, recycle-bin behavior, and NTFS-specific optimizations matter most.

GPL-licensed reference projects are useful for behavior and boundary ideas, but their code and rule data should not be copied directly unless the licensing strategy changes.

# Alternatives

## Option A: Windows-only

**Pros**: Clear scope, faster delivery, simplest safety model.  
**Cons**: Narrower future reuse.  
**Decision**: Rejected because we want a reusable core.

## Option B: Cross-platform from day one

**Pros**: Broader audience, one codebase.  
**Cons**: Scope explosion, weaker Windows specialization, slower MVP.  
**Decision**: Rejected because it delays the Windows product we actually want.

## Option C: Windows-first with shared core abstractions

**Pros**: Best fit for current demand, leaves room for Linux later, keeps architecture reusable.  
**Cons**: Requires discipline to avoid platform-specific leakage.  
**Decision**: Chosen.

# Consequences

- Windows-specific rules and adapters can be first-class.
- Linux can be added later without rewriting the core model.
- Product language and docs should describe Windows as the launch platform.
- Any GPL reference must remain reference-only unless licensing is intentionally adopted.

# Citations

- [Mole README](../../../../repo-ref/Mole/README.md)
- [windows-cleaner-cli README](../../../../repo-ref/windows-cleaner-cli/README.md)
- [CrunchyCleaner README](../../../../repo-ref/CrunchyCleaner/README.md)
- [Core runtime ADR](../../../../docs/adr/0002-core-runtime-architecture.md)
