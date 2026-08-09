# Linux capability enforcement contract

Status: implemented in the GardenLiminal single-process Seed path; privileged validation evidence is tracked separately.

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

For every requested capability the runtime removes and verifies the capability across all five Linux capability sets relevant to the current process:

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

`CAPS_DROPPED` may be emitted by the process runner only after this function returns success.

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

## Current evidence boundary

The implementation and pure regression tests are part of normal CI. A privileged Linux fixture is still required to publish a canonical before/after evidence pack containing:

- exact source SHA;
- distro/kernel/architecture;
- Rust toolchain;
- requested policy;
- before/after capability state;
- proof that a child `execve` does not regain a requested dropped capability.

Until that evidence pack exists, the defensible claim is:

> GardenLiminal implements kernel capability dropping with in-process post-state verification; privileged environment validation is still being expanded.

This is not a production sandbox certification and does not establish absence of sandbox escapes.
