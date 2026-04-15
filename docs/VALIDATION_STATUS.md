# GardenLiminal Validation Status

This document complements `docs/SECURITY_STATUS.md` by focusing on what is
currently validated, what is only partially exercised, and what still needs
stronger evidence.

## Status summary

| Area | Status | Notes |
|---|---|---|
| Unit and integration tests | Implemented | Repository contains tests for process, restart, OCI, garden, and iteration flows |
| Linux isolation ordering | Partial | `no_new_privs -> caps -> seccomp` flow exists, but not all runtime paths are equally covered |
| Named volume lifecycle tests | Implemented | Test now uses temp storage instead of system path, making CI reproducible |
| Seccomp profile compilation | Implemented | Named profiles compile and apply through `seccompiler` |
| Capability dropping validation | Partial | Code path exists, but effective dropping is still MVP-level |
| `pivot_root` validation | Partial | Isolation module supports it, but pod path still uses `chroot` fallback |
| DNS/service discovery validation | Partial | Registry and status checks exist; full UDP DNS path is still not validated as production-ready |
| Rootless/rootful matrix | Planned | No published Linux matrix covering both modes across supported kernels |
| External security review | Planned | Not yet performed |

## What is validated today

- process lifecycle and restart behavior through `tests/fork_exec.rs` and `tests/restart.rs`
- OCI import and run paths through `tests/oci.rs`
- multi-container garden flows through `tests/garden_e2e.rs`
- iteration 4 features including volumes, secrets, metrics, and network helpers through `tests/iteration4.rs`
- benchmark target compilation and microbenchmark coverage for manifest/event overhead

## Where evidence is still weaker

### Capability dropping

The runtime invokes capability dropping in the isolation flow, but
`src/isolate/caps.rs` still documents the implementation as MVP and warns that
full dropping is not yet implemented.

### `pivot_root` adoption

`src/isolate/mount.rs` implements `pivot_root`, but `src/pod.rs` still
contains a `chroot` path for pod startup. Reviewers should therefore treat
`pivot_root` as implemented groundwork rather than uniform runtime behavior.

### DNS isolation

The codebase contains service registry and status helpers, but this is not yet
the same thing as publishing a complete, production-ready DNS isolation story.

## Recommended wording

Use wording like:

- "contains reproducible Linux-focused runtime tests"
- "validates lifecycle, OCI, garden, and event paths"
- "includes partial hardening work still being expanded"

Avoid wording like:

- "fully verified hardened runtime"
- "uniform `pivot_root` across all execution paths"
- "complete capability dropping validation"

## Recommended next validation milestones

1. publish a Linux-only rootless/rootful test matrix
2. add focused tests for effective capability dropping semantics
3. add an explicit `pivot_root` vs `chroot` validation note per runtime path
4. define external audit scope and expected review artefacts
