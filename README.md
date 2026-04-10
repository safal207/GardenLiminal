# GardenLiminal

> A process isolation runtime where every lifecycle event is a first-class citizen.

GardenLiminal runs processes in isolated containers using Linux namespaces, cgroups v2, and seccomp — and emits a structured audit trail for every step it takes. It is designed to work natively with [LiminalDB](https://github.com/safal207/LiminalBD), a biological-inspired event store that remembers the full history of your workloads.

```
seed planted → namespaces created → cgroups applied →
capabilities dropped → process started → process exited
        ↓
  every step flows into LiminalDB as an impulse
```

---

## Why GardenLiminal?

Most container runtimes treat observability as an afterthought — you bolt on Falco, Sysdig, or auditd separately. GardenLiminal takes the opposite approach: **the audit trail is built into the runtime itself.**

Every isolation step emits a structured JSON event. Those events flow into LiminalDB where they are stored as impulses, queryable via LQL, and replayable as a timeline. You get compliance-grade observability without a separate agent.

| Feature | containerd / Podman | GardenLiminal |
|---|---|---|
| Audit trail | External agent required | Built-in, per step |
| Security policies | Flat profiles | Versioned Pacts (`web-api@1`) |
| Event persistence | Logs only | LiminalDB impulses |
| Memory safety | Go (GC) | Rust (no GC, no unsafe allocs) |
| Rootless by default | Optional | Yes |

---

## Concepts

GardenLiminal uses a botanical metaphor that maps directly to its architecture:

| Term | Meaning |
|---|---|
| **Seed** | A single-process workload manifest (YAML) |
| **Garden** | A multi-container pod — several Seeds sharing network and volumes |
| **Sprout** | A running isolated process |
| **Pact** | A versioned security policy (`drop_caps`, `seccomp_profile`) |
| **Impulse** | A lifecycle event sent to LiminalDB |

The name *Liminal* refers to the threshold state — the boundary between the host OS and the isolated environment. GardenLiminal lives and operates in that boundary, recording everything that crosses it.

---

## Features

- **Full Linux isolation** — user, pid, uts, ipc, mnt, net namespaces
- **Resource limits** — CPU shares, memory, PID limits via cgroups v2
- **Rootless mode** — UID/GID mapping, no root required for process isolation
- **OverlayFS** — multi-layer rootfs for containers
- **OCI image support** — import and unpack OCI image layers
- **5 volume types** — emptyDir (disk/tmpfs), hostPath, namedVolume, config, secret
- **Secrets management** — tmpfs-backed, strict permissions (0400), value masking in logs
- **Versioned security policies** — Pacts with seccomp profiles and capability lists
- **Service discovery** — DNS schema `service-name.pod-name.garden`
- **Prometheus metrics** — HTTP exporter on `127.0.0.1:9464`
- **LiminalDB integration** — every event sent as a WebSocket impulse

---

## Quick Start

### Prerequisites

- Rust 1.70+
- Linux kernel 5.10+ with cgroups v2
- User namespaces enabled

```bash
# Check user namespace support
cat /proc/sys/kernel/unprivileged_userns_clone   # should be 1
```

### Build

```bash
cargo build --release
# binary: ./target/release/gl
```

### Run a container

```bash
# Inspect a seed manifest
./target/release/gl inspect -f examples/seed-busybox.yaml

# Run with in-memory event store (stdout)
sudo ./target/release/gl run -f examples/seed-busybox.yaml --store mem

# Run with LiminalDB (events persist, queryable via LQL)
sudo ./target/release/gl run -f examples/seed-busybox.yaml --store liminal
```

### Run a pod (multi-container Garden)

```bash
sudo ./target/release/gl garden run -f examples/garden-echo.yaml --store mem
```

---

## LiminalDB Integration

GardenLiminal connects to [LiminalDB](https://github.com/safal207/LiminalBD) via WebSocket and sends every lifecycle event as an impulse.

```bash
# Start LiminalDB
liminal-cli

# Run a container — events flow into LiminalDB automatically
LIMINAL_URL=ws://127.0.0.1:8787 \
  sudo -E ./target/release/gl run -f examples/seed-busybox.yaml --store liminal

# Query the event history via LQL
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

See `examples/demo-liminaldb.sh` for the full end-to-end demo.

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
  drop_caps: ["NET_ADMIN", "SYS_ADMIN"]
  seccomp_profile: "minimal"
user:
  uid: 1000
  gid: 1000
  map_rootless: true
store:
  kind: "liminal"
```

---

## Lifecycle Events

Every isolation step emits a structured JSON event:

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

Full event sequence:

```
RUN_CREATED → SEED_LOADED → NS_CREATED → MOUNT_DONE →
CGROUP_APPLIED → IDMAP_APPLIED → CAPS_DROPPED →
SECCOMP_ENABLED → PROCESS_START → PROCESS_EXIT
```

For pods (Gardens):

```
POD_NET_READY → CONTAINER_START × N → CONTAINER_EXIT × N → POD_EXIT
```

---

## Security Policies (Pacts)

Pacts are versioned security profiles referenced by name:

```yaml
security:
  seccomp_profile: "web-api@1"
  drop_caps: ["NET_ADMIN", "SYS_ADMIN"]
```

Built-in pacts: `minimal`, `web-api@1`. Custom pacts can be loaded at runtime.

---

## CLI Reference

```bash
# Single process
gl inspect -f seed.yaml          # validate manifest
gl prepare -f seed.yaml          # check prerequisites
gl run -f seed.yaml --store mem  # run with isolation

# Pod (multi-container)
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

```
gl (binary)
├── CLI (clap)
├── Seed / Garden Parser (YAML)
├── Isolation Layer
│   ├── Namespaces (user, pid, uts, ipc, mnt, net)
│   ├── Mounts (OverlayFS, chroot, bind mounts)
│   ├── UID/GID Mapping (rootless)
│   ├── Cgroups v2 (cpu, memory, pids)
│   ├── Capabilities (drop)
│   ├── Seccomp profiles
│   └── Network (bridge gl0, veth, IPAM 10.44.0.0/16)
├── Pod Supervisor (lifecycle, restart policies, crash loop detection)
├── Volume Manager (emptyDir, hostPath, namedVolume, config, secret)
├── Secrets (tmpfs, 0400 permissions, version support)
├── Metrics (Prometheus HTTP on :9464)
├── Event System (structured JSON, all lifecycle steps)
└── Store
    ├── Memory (in-process, events to stdout)
    └── LiminalDB (WebSocket impulses → persistent, queryable)
```

---

## Project Structure

```
src/
├── main.rs          # Entry point
├── cli.rs           # CLI interface
├── seed.rs          # Seed & Garden config parser
├── events.rs        # Event model + builders
├── process.rs       # Process runner (fork/exec/wait/reap)
├── pod.rs           # Pod supervisor
├── metrics.rs       # Prometheus metrics
├── isolate/         # Isolation primitives
│   ├── ns.rs        # Namespaces
│   ├── mount.rs     # Mounts
│   ├── overlay.rs   # OverlayFS
│   ├── idmap.rs     # UID/GID mapping
│   ├── cgroups.rs   # Cgroups v2
│   ├── caps.rs      # Capabilities
│   ├── seccomp.rs   # Seccomp
│   ├── net.rs       # Network (bridge, veth, IPAM)
│   └── dns.rs       # Service discovery
├── store/           # Storage backends
│   ├── mem.rs       # In-memory store
│   ├── liminal.rs   # LiminalDB WebSocket adapter
│   ├── cas.rs       # Content-addressable storage (OCI)
│   ├── pacts.rs     # Security policies
│   └── oci.rs       # OCI image parsing
├── volumes/         # Volume management
└── secrets/         # Secrets management
```

---

## Development

```bash
# Run tests
cargo test

# Debug logging
RUST_LOG=debug sudo ./target/release/gl run -f examples/seed-busybox.yaml

# Full integration demo with LiminalDB
./examples/demo-liminaldb.sh
```

**Requirements:** Linux kernel 5.10+, cgroups v2, Rust 1.70+

---

## Roadmap

- [x] Single process isolation (namespaces, cgroups, seccomp)
- [x] Multi-container pods (Garden + OverlayFS + network)
- [x] OCI image support
- [x] Volume management (5 types)
- [x] Secrets management (tmpfs, versioned)
- [x] Prometheus metrics exporter
- [x] LiminalDB WebSocket integration
- [ ] `pivot_root` (replace current `chroot`)
- [ ] Full seccomp filter implementation
- [ ] CNI plugin support
- [ ] LiminalDB auth (API key handshake)

---

## License

MIT

---

## Ecosystem

GardenLiminal is part of a two-project ecosystem:

- **GardenLiminal** — the runtime. Plants seeds, grows gardens, records every moment.
- **[LiminalDB](https://github.com/safal207/LiminalBD)** — the memory. Stores impulses, replays timelines, queries history.
