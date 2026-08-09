# LiminalDB reconnect and replay evidence

Status: bounded transport reliability for the GardenLiminal `LiminalStore` adapter.

## Problem fixed

The previous adapter could lose an impulse silently in two cases:

- a write on an established WebSocket failed and the payload was printed to stdout but not retained for replay;
- a reconnect succeeded but the immediate replay `send()` result was ignored.

That behavior was incompatible with evidence-oriented lifecycle reporting.

## Outbox invariant

Every serialized impulse enters a bounded in-memory FIFO **before** transport is attempted.

```text
Store call
  ↓
serialize {cmd:"impulse", data:...}
  ↓
append FIFO (max 1024)
  ↓
ensure WebSocket connection
  ↓
flush from FIFO front
  ├─ send success → pop front → next
  └─ send failure → keep front → drop socket → retry on later Store call
```

When LiminalDB is unavailable, payloads remain queued. A later Store operation reconnects and replays the backlog in order before the newly queued payload completes its flush.

If the bounded outbox is full, the Store call returns an error instead of dropping evidence.

## Evidence fixture

The regression fixture uses a real local WebSocket handshake:

1. reserve a loopback port and leave it offline;
2. create `LiminalStore` against that URL;
3. submit event `seq=1` and verify it remains pending;
4. start a WebSocket listener on the same address;
5. submit event `seq=2`;
6. verify the listener receives two valid LiminalDB `cmd=impulse` frames in order `[1, 2]`;
7. verify the pending queue returns to zero.

A separate capacity test fills the bounded offline queue and requires the next Store write to fail closed.

## Claim boundary

A successful tungstenite `send()` means the WebSocket frame was accepted by the local transport stack. It is **not** a durable LiminalDB commit acknowledgement. The current raw `impulse` protocol does not return a per-impulse durable ACK, so GardenLiminal does not claim one.

The stronger durable-delivery boundary would require an acknowledgement/idempotency protocol shared by GardenLiminal and LiminalDB. This document intentionally does not imply that protocol already exists.
