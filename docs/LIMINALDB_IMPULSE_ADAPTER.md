# Garden lifecycle → LiminalDB impulse adapter

Status: explicit application-level compatibility adapter.

## Why this exists

A successful WebSocket write is not enough to prove LiminalDB accepted a lifecycle record. LiminalDB's raw WebSocket command is `{"cmd":"impulse","data":...}`, but the application-level impulse parser requires `data.pattern` and maps optional `kind`, `strength`, `ttl_ms`, and `tags` into `liminal_core::Impulse`.

GardenLiminal previously placed its lifecycle object directly in `data`. That was transport-valid JSON but application-invalid for LiminalDB.

## Adapter shape

GardenLiminal now encodes each complete lifecycle envelope as:

```json
{
  "cmd": "impulse",
  "data": {
    "kind": "write",
    "pattern": "garden.lifecycle.v1:<full lifecycle JSON>",
    "strength": 0.85,
    "ttl_ms": 86400000,
    "tags": ["garden", "lifecycle", "event"]
  }
}
```

The last tag is derived from the Garden lifecycle record type (`seed_upsert`, `run_created`, `event`, or `run_status`).

## Why JSON is embedded in `pattern`

The current LiminalDB `Impulse` structure has `kind`, `pattern`, `strength`, `ttl_ms`, and `tags`, but no arbitrary metadata object. Embedding the full lifecycle JSON after a versioned prefix preserves the complete record without pretending an unsupported metadata field exists.

A future shared protocol can replace this compatibility encoding with a typed metadata field or a dedicated lifecycle command. Until then, the version prefix makes the representation explicit and reversible.

## Evidence contract

Unit tests require that:

- `data.pattern` exists;
- `kind` is `write`;
- `strength`, TTL, and Garden/lifecycle tags are explicit;
- stripping `garden.lifecycle.v1:` and decoding the suffix returns the exact original lifecycle record;
- reconnect/replay preserves the same application-valid frames and order.

The final Liminal Stack E2E additionally starts a real LiminalDB process and requires its application parser to accept Garden lifecycle impulses without `impulse requires pattern` or `ws command failed` errors.

## Claim boundary

This adapter proves protocol compatibility and transport replay. It still does not provide a per-impulse durable commit acknowledgement because the current LiminalDB WebSocket impulse path does not expose one.
