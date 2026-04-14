# Liminal Stack Demo

This document describes a minimal end-to-end demo for the current Liminal
Stack:

```text
DAO_lim -> GardenLiminal -> LiminalDB
```

The aim is to give reviewers one short scenario they can run without having to
reverse-engineer the relationship between the three repositories.

## Repositories

- DAO_lim: https://github.com/safal207/DAO_lim
- LiminalBD: https://github.com/safal207/LiminalBD
- GardenLiminal: https://github.com/safal207/GardenLiminal

## 1. Start LiminalDB

From the `LiminalBD` repository:

```bash
cargo build --release -p liminal-cli
./target/release/liminal-cli --store ./data --ws-port 8787
```

Useful reviewer commands:

```text
:status
:mirror top 10
```

Expected outcome:

- a WebSocket endpoint is available on `ws://127.0.0.1:8787`
- the CLI exposes live system status
- Mirror Timeline can show recent stored events

## 2. Run GardenLiminal with LiminalDB-backed event storage

From this repository:

```bash
cargo build --release
LIMINAL_URL=ws://127.0.0.1:8787 \
  sudo -E ./target/release/gl run -f examples/seed-busybox.yaml --store liminal
```

Optional helper script:

```bash
./examples/demo-liminaldb.sh
```

Expected outcome:

- the example seed runs successfully
- lifecycle events are emitted by the runtime
- those events are sent to the running LiminalDB instance

## 3. Inspect the resulting audit trail

Back in the `LiminalBD` CLI:

```text
:mirror top 20
```

Optional direct query path with `websocat`:

```bash
echo '{"cmd":"mirror.timeline","top":20}' | websocat -n1 ws://127.0.0.1:8787
```

Expected outcome:

- recent GardenLiminal events appear in the timeline
- the reviewer can see persistent runtime evidence, not just stdout logs

## 4. Start DAO_lim

From the `DAO_lim` repository:

```bash
cargo build --release
./target/release/dao --config configs/dao.toml
```

Expected outcome:

- DAO starts with admin API on `127.0.0.1:9103`
- metrics become available on `0.0.0.0:9102`

## 5. Inspect DAO routing behavior

From another terminal in `DAO_lim`:

```bash
./target/release/daoctl health
./target/release/daoctl upstreams
./target/release/daoctl explain \
  --host api.example.com \
  --path /v1/chat \
  --intent realtime
```

Expected outcome:

- health and upstream state are inspectable
- the routing decision is explainable through `daoctl explain`

## What this demo proves

This short demo demonstrates:

- `LiminalDB` as the replayable evidence layer
- `GardenLiminal` as the runtime emitting structured lifecycle events
- `DAO_lim` as the inspectable routing layer

It does not yet claim a single fully integrated production path for live user
traffic through all three components.

## Recommended next improvement

The next stronger version of the stack demo would route a real request through
DAO_lim into a GardenLiminal-managed service and then persist the resulting
runtime trail in LiminalDB as one continuous trace.
