# GardenLiminal Security Status

This document is a reviewer-facing snapshot of what is implemented today,
what is partial, and what remains roadmap work in `GardenLiminal`.

It is intended to keep grant and README claims aligned with the actual codebase.

## Status summary

| Area | Status | Notes |
|---|---|---|
| Linux namespaces | Implemented | User, pid, uts, ipc, mnt, and net namespace code is present |
| `pivot_root` support | Partial | Implemented in isolation code, but pod startup still falls back to `chroot` |
| cgroups v2 | Implemented | CPU, memory, and pids limits are applied in code |
| `no_new_privs` | Implemented | Applied before seccomp in isolation flow |
| seccomp filter compilation | Implemented | Named profiles compile and apply through `seccompiler` |
| seccomp policy coverage | Partial | README roadmap still lists full seccomp implementation as pending |
| capability dropping | Partial | Current code logs intent and warns that full dropping is not yet implemented |
| DNS isolation | Partial | DNS registry and status helpers exist; full UDP DNS server is still an MVP stub |
| OCI import path | Partial | OCI support exists, but some tests and code paths are still marked MVP |
| LiminalDB event delivery | Implemented | Runtime emits lifecycle events and supports WebSocket delivery with fallback |
| LiminalDB authenticated transport | Planned | API key handshake is still roadmap work |
| CNI integration | Planned | Not implemented yet |
| External security audit | Planned | Not yet completed |

## What is implemented today

### Isolation primitives

- namespace setup is present under `src/isolate/ns.rs`
- mount isolation includes `pivot_root` support in `src/isolate/mount.rs`
- cgroups v2 code exists for CPU, memory, and pids limits
- `no_new_privs` is set before seccomp application

### Audit and observability

- lifecycle events are structured and serializable in `src/events.rs`
- metrics export exists via the Prometheus registry in `src/metrics.rs`
- LiminalDB delivery is implemented in `src/store/liminal.rs`

### Policy model

- security policy objects ("Pacts") exist in `src/store/pacts.rs`
- named seccomp profiles are compiled and applied in `src/isolate/seccomp.rs`

## Partial or caveated areas

These are the main areas where reviewers should treat the project as
in-progress rather than fully hardened.

### Capability dropping

`src/isolate/caps.rs` currently documents itself as MVP code and warns that
full capability dropping is not yet implemented. The present behavior is not
equivalent to a fully enforced capability minimization layer.

### DNS server

`src/isolate/dns.rs` contains the registry and status surface, but the actual
UDP DNS server remains an MVP stub. Current service discovery is better
described as DNS groundwork plus `/etc/hosts`-style fallback behavior.

### `pivot_root` adoption

`pivot_root` support is implemented in the isolation module, but pod startup
still contains a `chroot` fallback path. The codebase should therefore be
described as supporting `pivot_root` work, not yet using it uniformly across
all runtime paths.

### Test maturity

Several integration tests under `tests/` are still placeholders. This means the
current repository demonstrates architecture and direction more strongly than it
demonstrates complete verification coverage.

## Recommended wording for reviewers

Use wording like:

- "implements core Linux isolation primitives in Rust"
- "includes structured lifecycle audit events"
- "contains partial hardening work for capability dropping, DNS, and broader runtime verification"

Avoid wording like:

- "fully hardened runtime"
- "complete DNS isolation"
- "complete capability dropping"
- "externally audited"

## Recommended next deliverables

For grant readiness, the most valuable next security artefacts are:

1. implement real capability dropping and document exact semantics
2. replace the DNS MVP stub with a real UDP resolver path
3. complete a Linux-only isolation test matrix for rootless and rootful modes
4. publish an external security review scope and disclosure process
