# Rootful isolation post-condition evidence

Status: privileged CI validation for the existing `pivot_root`, `no_new_privs`, and seccomp controls.

## Purpose

This pack proves bounded kernel-visible post-conditions for isolation primitives already implemented in GardenLiminal. It is a validation layer, not a claim that the full runtime lifecycle is complete.

## `pivot_root` post-condition

The evidence probe runs in a short-lived child process and:

1. records its original mount namespace ID;
2. unshares a new mount namespace;
3. makes `/` recursively private so mount propagation cannot reach the host;
4. creates an isolated temporary rootfs with an inside sentinel and a host-only sibling sentinel;
5. calls the production `mount::pivot_root_to()` implementation;
6. verifies the new-root sentinel is visible as `/inside-root`;
7. verifies `/.old_root` is absent after the lazy unmount/removal;
8. verifies the host-only sentinel is no longer reachable through its pre-pivot absolute path.

The workflow separately records the shell's host mount namespace before and after the probe and requires the IDs to be identical.

## `no_new_privs + seccomp` post-condition

A separate process:

1. reads `PR_GET_NO_NEW_PRIVS` and expects `0` initially;
2. calls the production `ns::set_no_new_privs()` implementation;
3. verifies `PR_GET_NO_NEW_PRIVS == 1`;
4. installs the production `minimal` seccomp profile;
5. verifies `PR_GET_SECCOMP == 2` (`SECCOMP_MODE_FILTER`);
6. calls `socket(2)`, which is intentionally absent from the minimal profile;
7. requires the kernel to reject that syscall with `EPERM`.

This demonstrates that the filter is installed and enforcing a denial, not merely that a BPF program can be constructed.

## Evidence pinning

The GitHub Actions artifact records:

- exact PR source SHA;
- workflow/merge SHA;
- Linux kernel and architecture;
- distro;
- Rust/Cargo versions;
- host mount namespace before/after the destructive child probe;
- `pivot_root` post-condition log;
- `no_new_privs + seccomp` post-condition log.

## Scope boundary

This pack does **not** close Issue #6. The current production `ProcessRunner` still performs namespace creation before the first `fork()`, so the host supervisor lifecycle remains a separate P1 architecture change.

The defensible claim after a green evidence run is:

> GardenLiminal's tested `pivot_root`, `no_new_privs`, and minimal-seccomp primitives have privileged kernel-visible post-condition evidence on the recorded Linux environment.

It is not a production sandbox certification and does not establish absence of sandbox escapes.
