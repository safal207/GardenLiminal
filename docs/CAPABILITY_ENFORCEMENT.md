# Linux capability enforcement contract

Status: implemented in the GardenLiminal single-process Seed path and validated by a privileged Linux CI fixture.

## Security objective

A manifest such as:

```yaml
security:
  drop_caps:
    - NET_ADMIN
    - SYS_ADMIN
```

must never be treated as advisory metadata. If GardenLiminal cannot enforce and verify the requested transition, workload bootstrap must stop before `exec`.

## Sets covered

For every requested capability the runtime removes and verifies the capability across all five Linux capability sets relevant to the current task:

- **Effective** — capabilities currently used by kernel permission checks;
- **Permitted** — upper bound for capabilities that may become effective;
- **Inheritable** — capabilities that can participate in `execve` transitions;
- **Bounding** — limits capabilities that can be regained through file capabilities across `execve`;
- **Ambient** — capabilities preserved across ordinary non-privileged `execve`.

## Enforcement order

```text
parse + normalize entire policy
        ↓
validate against /proc/sys/kernel/cap_last_cap
        ↓
read current capability sets
        ↓
preflight CAP_SETPCAP if bounding changes are needed
        ↓
PR_CAPBSET_DROP for requested bounding bits
        ↓
PR_CAP_AMBIENT_LOWER for requested ambient bits
        ↓
capset(2): clear Effective / Permitted / Inheritable
        ↓
capget + prctl read-back of all five sets
        ↓
return success only if every requested bit is absent
```

`CAPS_DROPPED` may be emitted by the process runner only after this function returns success. `IsolationConfig::apply_child()` propagates a capability-enforcement failure, so the event is unreachable on an unverified or failed drop.

## Name handling

The canonical public form is `CAP_*`, for example `CAP_NET_ADMIN`.

Historical GardenLiminal manifests and Pact metadata also used names without the prefix, such as `NET_ADMIN`. For backward compatibility both forms are accepted and normalized to the same capability number.

Unknown or empty names fail closed.

The current mapping covers Linux capability numbers 0–40, through `CAP_CHECKPOINT_RESTORE`. The running kernel is additionally checked through `/proc/sys/kernel/cap_last_cap`; a capability unsupported by that kernel is rejected before mutation.

## Bounding-set preflight

Dropping a bit from the capability bounding set requires `CAP_SETPCAP` in the caller's effective set. GardenLiminal checks this before making irreversible bounding-set changes.

If the requested policy needs a bounding-set transition but `CAP_SETPCAP` is not effective, bootstrap fails rather than applying only part of the policy.

## Post-condition

Success means the requested capability is absent from:

```text
Effective = false
Permitted = false
Inheritable = false
Bounding = false
Ambient = false
```

The implementation re-reads kernel state after mutation. A mismatch is an error, not a warning.

## Relationship to `no_new_privs`

The Seed path sets `PR_SET_NO_NEW_PRIVS` before capability enforcement and seccomp installation. Capability dropping does not replace `no_new_privs`, and `no_new_privs` does not replace capability dropping; they cover different privilege-transition boundaries.

## Privileged validation evidence

Normal CI includes pure regression coverage plus an isolated privileged Linux integration fixture. The fixture:

1. records the exact PR source SHA and the workflow/merge SHA;
2. records distro, kernel, architecture, Rust/Cargo versions, and `cap_last_cap`;
3. starts with `CAP_NET_RAW` present in Effective, Permitted, and Bounding sets;
4. sets `no_new_privs` and applies the GardenLiminal capability-drop path;
5. reads `/proc/thread-self/status` so evidence corresponds to the calling test task, not the Rust test process's thread-group leader;
6. verifies `CAP_NET_RAW` is absent from Effective, Permitted, Inheritable, Bounding, and Ambient sets;
7. executes a child shell and verifies the capability remains absent after `execve` and `NoNewPrivs` remains `1`;
8. uploads the environment and privileged test log as a GitHub Actions artifact even when the privileged test fails.

A successful fixture therefore supports the bounded claim:

> GardenLiminal enforces and verifies the requested Linux capability drop on the supported Seed path, and the privileged validation fixture demonstrates that the tested dropped capability is not regained across the tested `no_new_privs + execve` transition.

This is not a production sandbox certification and does not establish absence of sandbox escapes or correctness for untested kernels, privilege layouts, or execution paths.
