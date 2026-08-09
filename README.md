# GardenLiminal

> A process isolation runtime where every lifecycle event is a first-class citizen.

GardenLiminal runs local workloads through Linux namespace, cgroup, mount/`pivot_root`, `no_new_privs`, and seccomp primitives — and emits a structured audit trail for the lifecycle steps it can verify. It is designed to work natively with [LiminalDB](https://github.com/safal207/LiminalDB), an event store that remembers and replays workload history.

```text
seed planted → namespaces/mounts configured →
no_new_privs + seccomp policy → process started → process exited
        ↓
  verified lifecycle events flow into LiminalDB as impulses
```

---

## Why GardenLiminal?

Most container runtimes treat observability as a separate layer — you commonly add Falco, Sysdig, auditd, or another agent. GardenLiminal explores the opposite design: **make isolation lifecycle evidence part of the runtime path itself.**

Isolation steps emit structured JSON events. Those events can flow into LiminalDB where they are stored as impulses, queryable via LQL, and replayable as a timeline.

> GardenLiminal is an early-stage runtime and is not a production sandbox certification. See [`docs/ISOLATION_HARDENING_AUDIT_2026-08-09.md`](docs/ISOLATION_HARDENING_AUDIT_2026-08-09.md) for the current security/evidence boundary.

| Feature | containerd / Podman | GardenLiminal |
|---|---|---|
| Audit trail | External tooling commonly used | Built into the runtime lifecycle path |
| Security policies | Runtime/profile specific | Versioned Pact metadata + built-in seccomp profiles |
| Event persistence | Logs / external integrations | LiminalDB impulses |
| Memory safety | Go / systems components | Rust |
| Rootless path | Supported by mature runtimes | Experimental UID/GID mapping path; validation pending |

---

## Concepts

GardenLiminal uses a botanical metaphor that maps directly to its architecture:

| Term | Meaning |
|---|---|
| **Seed** | A single-process workload manifest (YAML) |
| **Garden** | A multi-container pod — several Seeds sharing network and volumes |
| **Sprout** | A running isolated process |
| **Pact** | A versioned security-policy record |
| **Impulse** | A lifecycle event sent to LiminalDB |

The name *Liminal* refers to the threshold state — the boundary between the host OS and the isolated environment.

---

## Features

- **Linux isolation primitives** — user, pid, uts, ipc, mnt, and optional net namespaces
- **Mount isolation** — private mount propagation plus `pivot_root` implementation
- **Resource limits** — CPU shares, memory, PID limits via cgroups v2
- **Rootless mapping path** — UID/GID mapping implementation; kernel-pinned validation still pending
- **Seccomp BPF profiles** — built-in `strict`, `minimal`, and `default` allow-list profiles via `seccompiler`
- **Fail-closed capability policy** — non-empty capability-drop requests are currently rejected until kernel enforcement is implemented
- **OverlayFS** — multi-layer rootfs for containers
- **OCI image support** — import and unpack OCI image layers
- **5 volume types** — emptyDir (disk/tmpfs), hostPath, namedVolume, config, secret
- **Secrets management** — tmpfs-backed, strict permissions (0400), value masking in logs
- **Service discovery** — DNS schema `service-name.pod-name.garden`
- **Prometheus metrics** — HTTP exporter on `127.0.0.1:9464`
- **LiminalDB integration** — lifecycle events can be sent as WebSocket impulses

---

## Quick Start

### Prerequisites

- Rust 1.70+
- Linux kernel 5.10+ with cgroups v2
- User namespaces enabled for rootless experiments

```bash
cat /proc/sys/kernel/unprivileged_userns_clone
```

### Build

```bash
cargo build --release
# binary: ./target/release/gl
```

### Run a container

```bash
./target/release/gl inspect -f examples/seed-busybox.yaml
sudo ./target/release/gl run -f examples/seed-busybox.yaml --store mem
```

### Run with LiminalDB

```bash
sudo ./target/release/gl run -f examples/seed-busybox.yaml --store liminal
```

### Run a pod (multi-container Garden)

```bash
sudo ./target/release/gl garden run -f examples/garden-echo.yaml --store mem
```

---

## LiminalDB Integration

GardenLiminal connects to [LiminalDB](https://github.com/safal207/LiminalDB) via WebSocket and sends lifecycle events as impulses.

```bash
# Start LiminalDB
liminal-cli

# Run a container — events flow into LiminalDB
LIMINAL_URL=ws://127.0.0.1:8787 \
  sudo -E ./target/release/gl run -f examples/seed-busybox.yaml --store liminal

# Query event history
echo '{"cmd":"lql","q":"SELECT * WHERE type = EVENT LIMIT 20"}' \
  | websocat -n1 ws://127.0.0.1:8787

# Replay the timeline
echo '{"cmd":"mirror.timeline","top":50}' \
  | websocat -n1 ws://127.0.0.1:8787
```

Configure the LiminalDB endpoint:

```bash
export LIMINAL_URL=ws://192.168.1.10:8787
```

See `examples/demo-liminaldb.sh` for the GardenLiminal → LiminalDB demo path.

---

## Seed Configuration

```yaml
apiVersion: v0
kind: Seed
meta:
  name: demo-busybox
  id: demo-001
rootfs:
  path: ./examples/rootfs-busybox
entrypoint:
  cmd: ["/bin/sh", "-c", "echo hello && uname -a"]
  env: ["PORT=8080"]
  cwd: "/"
limits:
  cpu:
    shares: 256
  memory:
    max: "128Mi"
  pids:
    max: 64
security:
  hostname: "seed-demo"
  drop_caps: []
  seccomp_profile: "minimal"
user:
  uid: 1000
  gid: 1000
  map_rootless: false
store:
  kind: "mem"
```

Current security boundary:

- `seccomp_profile` accepts only `strict`, `minimal`, or `default`; unknown names fail closed;
- non-empty `drop_caps` currently fails closed because kernel capability enforcement is not implemented yet;
- rootless mapping should be treated as experimental until the dedicated kernel-pinned validation pack exists.

---

## Lifecycle Events

Every verified lifecycle step can emit a structured JSON event:

```json
{
  "ts": "2025-10-28T12:00:00Z",
  "level": "info",
  "run": "550e8400-e29b-41d4-a716-446655440000",
  "seed": "demo-001",
  "event": "PROCESS_EXIT",
  "code": 0,
  "msg": "Process exited with code 0"
}
```

Typical sequence:

```text
RUN_CREATED → SEED_LOADED → CGROUP_APPLIED → NS_CREATED →
MOUNT_DONE → [IDMAP_APPLIED] → [SECCOMP_ENABLED] →
PROCESS_START → PROCESS_EXIT
```

`CAPS_DROPPED` is reserved for a future path where a non-empty capability policy has actually been enforced successfully. The runtime does not emit that event for an empty policy.

For pods (Gardens):

```text
POD_NET_READY → CONTAINER_START × N → CONTAINER_EXIT × N → POD_EXIT
```

---

## Security Policies

There are currently two related concepts:

1. **Seed runtime seccomp profiles** — `strict`, `minimal`, `default`; these compile to BPF through `seccompiler`.
2. **Pact records** — versioned policy metadata stored by `PactStore` (for example `minimal@1`, `web-api@1`). The Pact model is broader than the currently wired runtime enforcement path.

Capability lists in policy metadata must not be read as evidence of kernel capability enforcement. Non-empty `drop_caps` requests fail closed until Phase 1 of the isolation hardening plan is implemented.

---

## CLI Reference

```bash
# Single process
gl inspect -f seed.yaml
gl prepare -f seed.yaml
gl run -f seed.yaml --store mem

# Pod
gl garden inspect -f garden.yaml
gl garden run -f garden.yaml
gl garden stats -f garden.yaml

# Volumes
gl volume create <name>
gl volume ls
gl volume rm <name>

# Secrets
gl secret create <name> --from-literal key=value
gl secret get <name> --version 1
gl secret rm <name> --version 1

# Network & diagnostics
gl net status
```

---

## Architecture

```text
gl (binary)
├── CLI (clap)
├── Seed / Garden Parser (YAML)
├── Isolation Layer
│   ├── Namespaces (user, pid, uts, ipc, mnt, net)
│   ├── Mounts (OverlayFS, pivot_root, chroot fallback, bind mounts)
│   ├── UID/GID Mapping (experimental rootless path)
│   ├── Cgroups v2 (cpu, memory, pids)
│   ├── no_new_privs
│   ├── Capabilities (policy surface; kernel enforcement pending)
│   ├── Seccomp BPF profiles
│   └── Network (bridge gl0, veth, IPAM 10.44.0.0/16)
├── Pod Supervisor (lifecycle, restart policies, crash loop detection)
├── Volume Manager (emptyDir, hostPath, namedVolume, config, secret)
├── Secrets (tmpfs, 0400 permissions, version support)
├── Metrics (Prometheus HTTP on :9464)
├── Event System (structured lifecycle evidence)
└── Store
    ├── Memory
    └── LiminalDB (WebSocket impulses)
```

---

## Project Structure

```text
src/
├── main.rs
├── cli.rs
├── seed.rs
├── events.rs
├── process.rs
├── pod.rs
├── metrics.rs
├── isolate/
│   ├── ns.rs
│   ├── mount.rs
│   ├── overlay.rs
│   ├── idmap.rs
│   ├── cgroups.rs
│   ├── caps.rs
│   ├── seccomp.rs
│   ├── net.rs
│   └── dns.rs
├── store/
│   ├── mem.rs
│   ├── liminal.rs
│   ├── cas.rs
│   ├── pacts.rs
│   └── oci.rs
├── volumes/
└── secrets/
```

---

## Development

```bash
cargo test
RUST_LOG=debug sudo ./target/release/gl run -f examples/seed-busybox.yaml
./examples/demo-liminaldb.sh
```

**Requirements:** Linux kernel 5.10+, cgroups v2, Rust 1.70+

---

## Isolation hardening status

- [x] Namespace/cgroup isolation implementation
- [x] Private mount propagation
- [x] `pivot_root` implementation
- [x] Seccomp BPF profile implementation (`strict`, `minimal`, `default`)
- [x] Unknown seccomp profile fails closed
- [x] Non-enforced capability requests fail closed instead of producing false success evidence
- [ ] Kernel-enforced capability dropping + post-condition verification
- [ ] Host-supervisor / workload namespace lifecycle redesign (Issue #6)
- [ ] Rootful `pivot_root` and seccomp validation pack tied to exact kernel/source revisions
- [ ] Rootless UID/GID mapping validation pack
- [ ] CNI plugin support
- [ ] LiminalDB auth (API key handshake)

See [`docs/ISOLATION_HARDENING_AUDIT_2026-08-09.md`](docs/ISOLATION_HARDENING_AUDIT_2026-08-09.md).

---

## License

MIT

---

## Ecosystem

- **GardenLiminal** — runtime/isolation research implementation.
- **[LiminalDB](https://github.com/safal207/LiminalDB)** — durable event/evidence memory.
