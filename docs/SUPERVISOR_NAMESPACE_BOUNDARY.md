# Supervisor / workload namespace boundary

Status: implementation under validation for Issue #6.

## Security objective

The host-side GardenLiminal supervisor must not enter workload namespaces or the workload cgroup. Its responsibilities are control-plane work only:

- persist lifecycle/audit evidence;
- prepare host-side cgroup resources;
- install rootless UID/GID maps for a blocked child;
- move the host-visible child PID into its workload cgroup;
- release the child only after those parent-side transitions succeed;
- wait for and account for the workload exit.

The workload task owns the isolated execution boundary.

## Process shape

```text
host supervisor
  host namespaces
  host audit/store connectivity
        |
        | clone(CLONE_NEWPID | CLONE_NEWUTS | CLONE_NEWIPC |
        |       CLONE_NEWNS [+ CLONE_NEWUSER] [+ CLONE_NEWNET])
        v
blocked workload task
  host-visible PID = child pid
  namespace PID = 1
        ^
        |
parent installs UID/GID map when rootless
parent moves child host PID into prepared cgroup when requested
        |
        | local FD_CLOEXEC Unix control socket: GO
        v
workload bootstrap
  verify mapped identity
  capture/verify namespace IDs
  set hostname
  configure mounts
  set no_new_privs
  enforce + verify capability policy
  install seccomp
  send local lifecycle notices to supervisor
        |
        | execve
        v
workload (PID 1 in target PID namespace)
```

## Namespace rules

Every Seed workload receives distinct PID, UTS, IPC and mount namespaces.

`net.enable: true` adds a distinct network namespace. With networking disabled, the workload retains the supervisor network namespace.

`user.map_rootless: true` adds a user namespace. The host supervisor writes `/proc/<child>/setgroups`, `uid_map`, and `gid_map` before releasing the child. Rootful Seeds deliberately do not request `CLONE_NEWUSER`, avoiding an unmapped root identity.

## PID 1 semantics

The cloned workload bootstrap is PID 1 in the newly-created PID namespace and remains PID 1 across `execve`. GardenLiminal treats any observed namespace PID other than 1 as a fail-closed bootstrap error.

This makes PID-init semantics explicit. It does not yet claim full init-system behavior such as advanced signal forwarding or zombie reaping for arbitrary daemon trees; those can be layered later if required by workloads.

## Cgroup ownership

Native Seed cgroup setup now has two phases:

1. the supervisor creates/configures the cgroup limits without joining it;
2. after clone returns the host-visible child PID, the supervisor writes that PID to `cgroup.procs` before releasing the child.

`CGROUP_APPLIED` is emitted only after this move succeeds. Empty limits do not produce a cgroup or a `CGROUP_APPLIED` event.

## Audit/store boundary

Persistent Store writes stay on the host-side supervisor. The child does not call LiminalDB after entering workload namespaces. It sends bounded lifecycle notices over a local Unix socketpair marked `FD_CLOEXEC`.

A successful `execve` closes the child-side evidence channel. The supervisor then waits on the host-visible child PID and records the final lifecycle state itself.

This keeps host audit connectivity outside a workload network namespace and prevents a fresh workload network socket from being used as the evidence transport.

## Runtime evidence checks

Before clone, the supervisor records its own `/proc/thread-self/ns/*` identities. The child reports its namespace identities over the local control channel. GardenLiminal validates:

- child namespace PID is exactly 1;
- PID/UTS/IPC/mount namespace IDs differ from the supervisor;
- user namespace differs iff rootless mapping is requested;
- network namespace differs iff `net.enable` is true.

After the child exits, the supervisor re-reads its own namespace identities and fails the run if any changed.

## Privileged CI fixture

The privileged namespace-boundary test additionally proves on the CI kernel that:

- the supervisor namespace IDs are unchanged before/after;
- a rootless, network-isolated child observes the requested mapped UID/GID;
- the child is PID 1 in a distinct PID namespace;
- user/UTS/IPC/mount/network namespaces are distinct;
- the supervisor can still create a fresh TCP connection in the host network namespace;
- a fresh TCP socket created by the isolated child cannot reach the host-side loopback listener.

CI records exact source/workflow SHA, kernel, distro, architecture and Rust toolchain and uploads the privileged log as an artifact.

## Scope boundary

This is defensive local runtime hardening. It is not a sandbox-escape test, does not target external systems, and does not claim production container-runtime certification across untested kernels or privilege layouts.
