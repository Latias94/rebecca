# Rebecca Performance Matrix

The performance matrix is the product-level baseline for scan, cache, and cleanup execution work. It is intentionally synthetic and deterministic so later refactors can compare the same shapes before using real-machine dogfood.

Run the compile check first:

```powershell
cargo check -p rebecca-core --benches
```

Run the matrix and collect a JSON report:

```powershell
pwsh -File scripts/perf/run-benchmark-matrix.ps1
```

The script runs `cargo bench -p rebecca-core --bench perf_matrix`, reads Criterion estimates from `target/criterion/perf_matrix`, combines them with scenario metadata from `target/perf/perf_matrix-scenarios.json`, and writes `target/perf/rebecca-core-perf_matrix-report.json`.

The report records scenario name, operation, backend, fixture shape, physical files and directories, expected bytes, progress-event count, target count, cache mode, delete mode, and Criterion mean/median timing estimates. The default scenarios cover:

- cold recursive scan over many small files
- recursive scan with file-level progress callbacks
- one large flat directory
- a deep directory tree
- parallel target measurement
- duplicate target candidates
- ordinary rule planning over many directory targets
- target-level rule-planning progress over many directory targets
- scan-cache miss plus store
- scan-cache hit
- serial cleanup deletion
- parallel cleanup deletion

Keep reports under `target/perf/`; they are local measurement artifacts and should not be committed unless a future release process explicitly asks for a checked-in baseline.
