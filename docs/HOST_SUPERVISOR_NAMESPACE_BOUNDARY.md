# Host supervisor outside workload namespaces

Status: architecture implemented on the Seed `ProcessRunner` path; privileged lifecycle evidence is generated in CI.

## Trust boundary

GardenLiminal separates durable host authority from workload authority:

```text
host supervisor
  │  owns Store / LiminalDB connection
  │  remains in host namespaces and host cgroup
  │
  ├─ AF_UNIX lifecycle channel (close-on-exec)
  │
  └─ namespace bootstrap
       │  enters PID/mount/UTS/IPC and optional network namespace
       │  enters a user namespace only for rootless mapping
       │  configures uid_map/gid_map when rootless
       │
       └─ fork after CLONE_NEWPID
            └─ workload PID 1
                 ├─ hostname + pivot_root/mount setup
                 ├─ mapped identity when rootless
                 ├─ no_new_privs
                 ├─ capability enforcement
                 ├─ seccomp
                 └─ exec workload
```

## Why two forks

`CLONE_NEWPID` does not move its caller into the new PID namespace. It changes the PID namespace used for subsequently created children. The bootstrap therefore enters the namespace set and then forks once more. The first child is required to observe namespace PID `1` before isolation policy or workload `exec` continues.

## Durable audit / network boundary

Only the host supervisor receives the configured `Store` object. The namespace bootstrap and workload do not hold a Store/LiminalDB adapter. They send structured lifecycle messages over an inherited local Unix socket.

The socket is marked `FD_CLOEXEC`, so successful workload `exec` closes it. The bootstrap retains its endpoint long enough to report final outcome.

This has an important network property: enabling a workload network namespace cannot move or sever the supervisor's LiminalDB connection because that connection is created and used only from the host side.

## Cgroup boundary

The host supervisor prepares the workload cgroup and limits but does not add itself to it. After the first fork the host moves the namespace-bootstrap PID into that cgroup; its workload descendants inherit membership.

## Fail-closed bootstrap

Namespace creation, rootless mapping, second fork, PID-1 verification, mount/pivot, mapped identity, no_new_privs, capability enforcement and seccomp all happen before workload `exec`. A failure reports `PROCESS_FAILED` when the control channel is available and exits the bootstrap path with the reserved bootstrap-failure code rather than falling through to the workload.

A malformed/broken supervisor control channel causes the host to terminate the bootstrap process group and mark the run failed.

## Rootless identity mapping

For a rootless Seed, host UID/GID are captured before namespace entry. Immediately after creating the user namespace, the bootstrap writes bounded one-entry `uid_map` and `gid_map` values. The PID-1 child enters the mapped workload identity only after privileged mount setup and verifies the resulting UID/GID.

`IDMAP_APPLIED` contains the kernel-read map strings and effective workload UID/GID.

## Evidence contract

The privileged lifecycle fixture runs a real `ProcessRunner` twice with a static BusyBox workload and `net.enable=true`:

1. **rootful:** PID/mount/UTS/IPC/net namespaces must differ from the host while the user namespace remains the host user namespace;
2. **rootless:** PID/mount/UTS/IPC/net/user namespaces must differ from the host and the kernel-read UID/GID maps must match the requested single-ID mapping.

The evidence Store checks the current namespace snapshot on every Store method call. Any Store call occurring from a workload namespace fails the fixture.

Both modes require:

- host namespace IDs unchanged before/after the run;
- workload namespace PID `1`;
- workload namespace IDs differ where required;
- `PROCESS_START` contains `pid1=true`;
- Store calls remain host-side;
- status flow includes `Running` and `Exited(0)`;
- exact source/workflow SHA and environment metadata are uploaded.

## Scope boundary

This is a defensive local runtime boundary. It does not establish production sandbox certification or absence of sandbox escapes. The Pod/Garden multi-container supervisor is a separate execution path and is not silently covered by this Seed-path claim.
