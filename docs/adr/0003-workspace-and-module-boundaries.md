---
title: "Cargo Workspace and Module Boundaries"
status: "proposed"
created: "2026-06-23"
last_updated: "2026-06-29"
---

# Context

The cleaner will need a CLI entrypoint, a reusable core, Windows-specific APIs, and a rule catalog. Keeping all of that in a single crate would be simple at first, but it would quickly mix CLI concerns, platform code, and policy logic.

```mermaid
flowchart LR
  CLI[rebecca bin] --> FACADE[rebecca lib]
  FACADE --> CORE[rebecca-core]
  FACADE --> RULES[rebecca-rules]
  FACADE --> WIN[rebecca-windows]
  WIN --> CORE
  RULES --> CORE
```

# Decision

Use a small Cargo workspace with a few explicit crates:

- `rebecca` for the user-facing package, with a curated Rust facade plus the CLI binary.
- `rebecca-core` for scanning plans, safety checks, deletion orchestration, and history.
- `rebecca-windows` for Windows adapters and OS-specific APIs.
- `rebecca-rules` for built-in cleanup rules and category metadata.

The `rebecca` library surface may re-export stable product surfaces from the focused crates, while the `rebecca` binary remains the application adapter. The core crate must not depend on Windows-only APIs directly.

# Alternatives Considered

## Option A: Single crate with modules

**Pros**: Lowest initial setup cost.  
**Cons**: Boundaries blur quickly, test isolation is worse, platform code leaks into core.  
**Decision**: Rejected.

## Option B: Many small crates for every feature

**Pros**: Strong separation.  
**Cons**: Too much packaging overhead, harder navigation, unnecessary indirection.  
**Decision**: Rejected.

## Option C: Small workspace with a few stable crates

**Pros**: Clear boundaries, testable, easy to extend later.  
**Cons**: Slightly more setup than a monolith.  
**Decision**: Chosen.

## Option D: Publish only internal crates without a facade

**Pros**: Lowest packaging overhead after the workspace split.
**Cons**: External callers and the CLI would bind directly to implementation crate names, making future internal reshaping more expensive.
**Decision**: Rejected.

# Consequences

- CLI, core, and platform code can evolve independently.
- Rust callers get one product-level crate name, while focused implementation crates remain available for advanced users.
- The CLI validates the facade shape because it consumes Rebecca through `rebecca`.
- Shared policy stays in `rebecca-core`.
- Windows-specific code can be compiled only on Windows.
- Future Linux support can be added as a new adapter crate without reshaping the whole repo.

# Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Boundary clarity | `rebecca-core` compiles without Windows-only imports | CI compilation on target matrix |
| Testability | Core logic can be unit-tested without OS access | Unit tests for rules and planning |
| Extensibility | New platform adapter can be added without rewriting CLI | Architecture review |

# Risks & Mitigations

| Risk | Severity | Likelihood | Mitigation |
|------|----------|------------|------------|
| Too many crates too early | Medium | Medium | Keep the workspace small and stable |
| Circular dependencies | High | Low | Enforce one-way dependency direction |
| Premature abstraction | Medium | Medium | Keep `rebecca` curated and avoid turning it into a broad `pub use *` dumping ground |

# Status

Proposed.
