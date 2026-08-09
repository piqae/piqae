# Document render performance evidence

The capability-free load probe uses synthetic data, performs no network I/O and
cannot contact a printer:

```console
cargo run --release -p piqae-document-renderer --example render_load
```

It emits `piqae.render-load-evidence/v1` JSON with throughput and
p50/p95/p99/max latency. Archive it with runner model, OS, CPU allocation, Rust
version and commit SHA. Compare results only on a pinned runner class.

```console
PIQAE_RENDER_LOAD_ITERATIONS=5000 PIQAE_RENDER_LOAD_CONCURRENCY=8 \
PIQAE_RENDER_LOAD_WARMUP=100 PIQAE_RENDER_LOAD_MAX_P95_MS=25 \
cargo run --release -p piqae-document-renderer --example render_load
```

The optional p95 gate exits non-zero on regression. No universal millisecond
threshold is checked in because shared CI timing is not a production SLO.

## Production soak requirement

Before enabling hosted rendering, stage a 60-minute sustained run and 10-minute
burst through registration, PostgreSQL claiming, object storage and polling.
Capture registration-to-completion percentiles, errors/retries, queue depth and
oldest age, claim/storage latency, worker CPU/RSS, worker restart, key rotation,
and cleanup restart. Mix receipts, 100-line invoices, QR-heavy and maximum-bound
fixtures. These measurements are not physical-print latency.

## Artifact reuse lifecycle

Migration 0034 implements zero-copy render-to-print ownership. A deterministic
completed-upload alias points at the immutable render object; inserting the job
atomically creates a tenant-fenced artifact reference and extends render
retention through job expiry. A terminal job releases its reference. Cleanup
uses a lease and excludes live, unexpired references. If cleanup already leased
the render, the database rejects a racing job insertion rather than admitting a
job whose bytes could disappear.

This removes the former object-store read, Base64 expansion and duplicate write.
Production soak evidence must still include job replay, worker restart,
cleanup/print races, late job expiry and cross-tenant identifier probes.
