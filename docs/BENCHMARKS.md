# GardenLiminal Benchmarks

This repository now includes a reproducible microbenchmark in
`benches/manifest_and_events.rs`.

## What it measures

- Garden manifest parsing cost as container count grows
- lifecycle event serialization cost for audit output batches

These numbers do not claim kernel isolation throughput. They provide a stable
baseline for two user-visible control-plane paths that can regress silently:

- manifest ingestion
- audit event emission

## Run locally

```bash
cargo bench --bench manifest_and_events
```

Criterion reports are written to `target/criterion/`.

## Why this helps the grant story

- shows the runtime can be evaluated with repeatable measurements
- gives maintainers a low-friction regression signal before deeper system tests
- complements, rather than replaces, future rootful isolation benchmarks

## Recommended next step

Add a Linux-only benchmark runner for:

- container start latency
- event delivery overhead with `--store mem` vs `--store liminal`
- rootless vs rootful startup cost
