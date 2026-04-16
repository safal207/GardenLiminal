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

## First local benchmark report

### Environment

- Date: `2026-04-16`
- Commit: `38798527bde7cb80a890108dd24d4258e7bc170f`
- OS: `Ubuntu 24.04.2 LTS`
- Kernel: `6.6.87.1-microsoft-standard-WSL2`
- CPU: `AMD Ryzen 7 5700U with Radeon Graphics`
- RAM: `16 GB`
- Rust toolchain: `rustc 1.93.0 (254b59607 2026-01-19)`
- Cargo: `cargo 1.93.0 (083ac5135 2025-12-15)`

### Benchmark setup

- Benchmark target: `benches/manifest_and_events.rs`
- Command:

```bash
RUSTFLAGS="-Awarnings" cargo +1.93.0 bench --offline --bench manifest_and_events -- --sample-size 10
```

### Manifest parse results

- `gl_manifest_parse/1`: `41.911 us .. 45.449 us`
- `gl_manifest_parse/4`: `106.08 us .. 119.08 us`
- `gl_manifest_parse/8`: `194.76 us .. 226.06 us`
- `gl_manifest_parse/16`: `388.08 us .. 420.97 us`

### Event serialization results

- `gl_event_serialization/1`: `2.1342 us .. 2.3041 us`
- `gl_event_serialization/16`: `37.077 us .. 41.428 us`
- `gl_event_serialization/64`: `155.76 us .. 178.77 us`
- `gl_event_serialization/256`: `715.77 us .. 789.99 us`

### What these numbers mean

- manifest ingestion stays sub-millisecond even at `16` container definitions
- audit event serialization remains comfortably below `1 ms` for a `256`-event batch
- the measured control-plane cost is small enough that deeper runtime validation
  can focus on Linux isolation paths rather than YAML or JSON overhead

### Notes

- this report measures control-plane paths only, not namespace or seccomp startup
  latency
- the benchmark was run in `WSL2`, which is suitable for Linux compilation and
  control-plane measurement but is not a substitute for full rootful runtime validation
- `RUSTFLAGS="-Awarnings"` was used as a temporary workaround for a `rustc`
  warning-rendering ICE on the current source tree; benchmark code still compiled
  and ran successfully

## Recommended next step

Add a Linux-only benchmark runner for:

- container start latency
- event delivery overhead with `--store mem` vs `--store liminal`
- rootless vs rootful startup cost
