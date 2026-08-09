# GardenLiminal isolation hardening audit — 2026-08-09

Status: defensive local-code audit. No external systems were tested.

Baseline reviewed: `df6f7fd4ece9a21b4b89866fef4170177aa3d137`

## Scope

Current single-process path:

```text
cgroups (parent)
  → namespace transition
  → fork
  → hostname
  → private mounts
  → pivot_root
  → optional UID/GID mapping
  → no_new_privs
  → capability policy
  → seccomp
  → cwd/env
  → exec
```

Files reviewed: `src/process.rs`, `src/isolate/{mod,ns,mount,idmap,caps,seccomp}.rs`.

The goal is evidence integrity and fail-closed behavior, not a production-security certification.

## Findings

### GL-ISO-001 — capability-drop evidence can be false

Severity: **P0 / evidence-integrity blocker**

`drop_capabilities()` currently returns success for a non-empty requested drop list without changing Linux capability state. The process path can then emit `CAPS_DROPPED` as though enforcement happened.

Immediate safe behavior:

- fail closed whenever a non-empty capability-drop policy is requested until real enforcement exists;
- emit `CAPS_DROPPED` only after an actual successful enforcement path.

Future acceptance:

- update effective/permitted/inheritable capability sets;
- handle bounding and ambient sets where applicable;
- verify kernel-visible post-state;
- negative-test that a requested dropped capability cannot be reacquired across `execve` under `no_new_privs`.

### GL-ISO-002 — unknown seccomp profile falls back instead of failing closed

Severity: **P0 / policy-integrity blocker**

An unknown profile name currently falls back to `default`. A typo or stale policy therefore changes the intended security contract silently.

Immediate safe behavior: reject unknown profile names; only `strict`, `minimal`, and `default` are accepted until a versioned policy registry exists.

### GL-ISO-003 — supervisor enters namespaces before fork

Severity: **P1 / architecture hardening**

`create_namespaces()` is called before `fork()`. `CLONE_NEWUSER`, `CLONE_NEWUTS`, `CLONE_NEWIPC`, `CLONE_NEWNS`, and optionally `CLONE_NEWNET` therefore affect the calling supervisor as well as the later child. `CLONE_NEWPID` has child-specific semantics, which is likely why the ordering exists.

Risk:

- supervisor is not a clean host-side observer;
- with network isolation enabled, audit/store connectivity may be affected by the supervisor entering the new network namespace;
- rootless mapping and lifecycle ownership are harder to reason about.

Do **not** fix this by simply moving `unshare(CLONE_NEWPID)` below the existing `fork()`: that changes PID namespace semantics.

Preferred milestone:

```text
host supervisor
    ↓
clone/child bootstrap with namespace flags
    ↓
container-init (PID 1 in target PID namespace)
    ↓
policy application
    ↓
exec workload
```

Alternative: an intermediate bootstrap child performs namespace setup and forks the final workload while the host supervisor remains outside.

Acceptance:

- host supervisor namespace IDs remain unchanged across a run;
- workload receives intended namespace IDs;
- PID 1/init semantics are explicit and tested;
- audit-store traffic is proven from the intended namespace side.

### GL-ISO-004 — rootless UID/GID mapping needs exact transition evidence

Severity: **P1 / validation gap**

Mapping is written after namespace creation and inside the child path. The code comment describes sampled UID/GID as the parent identity, but namespace transition has already occurred.

This audit does not claim the mapping is universally wrong. It requires a rootless integration fixture before treating it as verified.

Acceptance:

- record host UID/GID before namespace creation;
- record namespace UID/GID before/after map installation;
- assert expected mapping;
- test `setgroups`, `uid_map`, and `gid_map` failures separately;
- pin kernel/user-namespace prerequisites.

### GL-ISO-005 — pivot_root implementation exists but lacks canonical rootful evidence

Severity: **P1 / evidence gap**

`pivot_root_to()` is implemented: mount propagation is made private, the new root is bind-mounted, `.old_root` is created, `pivot_root` is called, cwd becomes `/`, the old root is detached, and the temporary directory is removed.

The README roadmap is therefore stale when it describes `pivot_root` as pending. What remains pending is rootful post-condition evidence.

Acceptance examples:

- old host root is not reachable through expected root/proc paths;
- mount propagation does not escape to the host;
- failure at each bootstrap step aborts workload execution;
- evidence pins kernel, distro and source SHA.

### GL-ISO-006 — seccomp BPF exists but needs profile calibration evidence

Severity: **P1 / evidence gap**

The repository contains real BPF filter construction and installation through `seccompiler`. README wording that full seccomp implementation is pending is stale.

Remaining work:

- test forbidden syscalls under each profile in disposable local fixtures;
- prove `no_new_privs` is set before filter installation;
- document intentional syscall families per profile;
- keep unsupported profile names fail closed.

## Current truth table

| Control | Code exists | Evidence maturity | Safe claim today |
| --- | --- | --- | --- |
| mount namespace privacy | yes | limited | implementation present; rootful validation pending |
| `pivot_root` | yes | no canonical rootful pack | implementation present; validation pending |
| `no_new_privs` | yes | no explicit post-condition artifact | implementation present; validation pending |
| capability dropping | **no effective enforcement** | none | policy surface only; do not claim dropped capabilities |
| seccomp BPF | yes | no canonical forbidden-syscall pack | implementation present; profile validation pending |
| rootless ID mapping | yes | insufficient transition evidence | experimental / validation pending |
| namespace lifecycle | yes | supervisor currently joins non-PID namespaces | architecture hardening pending |

## Hardening sequence

### Phase 0 — evidence integrity

1. Fail closed for requested capability dropping while enforcement is absent.
2. Do not emit `CAPS_DROPPED` unless a requested policy was actually enforced.
3. Reject unknown seccomp profile names.
4. Correct README/reviewer claims so docs match code.

### Phase 1 — capability enforcement

1. Choose one implementation strategy (`caps` crate or direct Linux capability syscalls).
2. Define supported capability names.
3. Apply bounding/effective/permitted/inheritable policy deliberately.
4. Add kernel-visible post-condition verification.
5. Add positive and negative local tests.

### Phase 2 — namespace lifecycle

1. Keep host supervisor outside workload namespaces.
2. Preserve correct PID namespace init semantics.
3. Make bootstrap failure atomic: no workload `exec` after partial isolation failure.
4. Capture namespace IDs as evidence.

### Phase 3 — rootful validation pack

Produce a dedicated Linux-only local workflow covering `pivot_root`, mount propagation, `no_new_privs`, capability post-state, forbidden seccomp syscalls, rootless mapping where supported, and exact environment/source metadata.

## Non-claims

This audit does not establish that GardenLiminal is equivalent to Docker, containerd, gVisor, Kata, or a production-certified sandbox. It does not establish absence of sandbox escapes.

The defensible Phase-0 claim is narrower:

> GardenLiminal has inspectable Linux-isolation primitives, and requested controls that are not actually enforceable are prevented from producing false success evidence.
