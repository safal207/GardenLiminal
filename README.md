# GardenLiminal Codex

**Codex** is a lightweight process isolation runtime that launches processes in isolated containers using Linux namespaces, cgroups v2, and seccomp. It integrates with Liminal-DB for event persistence and provides structured JSON event logging.

## Project Status

**Current:** Iteration 2 (In Progress) - Garden (Pod + Layers + Network)
**Completed:** Iteration 1 - Sprout + Pulse (Single Process Isolation)

## Features

### Iteration 1 - Sprout + Pulse (Complete)
- **Namespace Isolation**: user, pid, uts, ipc, mnt, net namespaces
- **Resource Limits**: CPU shares, memory limits, PID limits via cgroups v2
- **Security**: Capability dropping, seccomp profiles, no_new_privs
- **Rootless Mode**: UID/GID mapping for unprivileged execution
- **Event Logging**: Structured JSON events to stdout and persistent storage
- **Storage Backends**: In-memory store (MVP) and Liminal-DB adapter (stub)

### Iteration 2 - Garden (In Progress)
- **Pod Concept**: Multiple containers (processes + sidecars) in one Garden
- **OverlayFS**: Multi-layer rootfs with lower/upper/work directories
- **Network Isolation**: bridge (gl0) + veth pairs + IP allocation (IPAM)
- **Security Policies (Pacts)**: Versioned security policies from Store
- **Metrics Collection**: Periodic cgroups metrics (memory, CPU, PIDs)
- **Restart Policies**: Never/OnFailure/Always with exponential backoff

## Architecture

```
gl (binary)
├── CLI (clap)
│   ├── inspect - Validate and print seed config
│   ├── prepare - Check prerequisites
│   └── run - Execute isolated process
├── Seed Parser (YAML config)
├── Isolation Layer
│   ├── Namespaces (user, pid, uts, ipc, mnt, net)
│   ├── Mounts (rootfs, /proc, bind mounts)
│   ├── UID/GID Mapping (rootless)
│   ├── Cgroups v2 (cpu, memory, pids)
│   ├── Capabilities (drop)
│   └── Seccomp (minimal, default, strict)
├── Process Runner (fork/exec/wait/reap)
├── Event System (JSON events)
└── Storage Layer
    ├── Memory Store (in-memory + stdout)
    └── Liminal Store (stub for future integration)
```

## Installation

### Prerequisites

- Rust 1.70+ (2021 edition)
- Linux kernel 5.10+ with cgroups v2
- User namespaces enabled (`/proc/sys/kernel/unprivileged_userns_clone = 1`)

### Build

```bash
cargo build --release
```

The binary will be at `./target/release/gl`.

## Usage

### Inspect a Seed Configuration

Validate and print normalized seed configuration:

```bash
./target/release/gl inspect -f examples/seed-busybox.yaml
```

### Prepare Environment

Check that paths and cgroups are available:

```bash
sudo ./target/release/gl prepare -f examples/seed-busybox.yaml
```

### Run a Process

Execute a process with full isolation:

```bash
sudo ./target/release/gl run -f examples/seed-busybox.yaml --store mem
```

Options:
- `--store mem` - Use in-memory store (events to stdout)
- `--store liminal` - Use Liminal-DB adapter (stub)

## Seed Configuration Format

See `examples/seed-busybox.yaml` for a complete example.

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
  env: ["RUST_LOG=info"]
  cwd: "/"
limits:
  cpu:
    shares: 256
  memory:
    max: "128Mi"
  pids:
    max: 64
net:
  enable: true
mounts:
  - type: proc
    target: /proc
security:
  hostname: "seed-demo"
  drop_caps: ["NET_ADMIN", "SYS_ADMIN"]
  seccomp_profile: "minimal"
user:
  uid: 1000
  gid: 1000
  map_rootless: true
logging:
  mode: "json"
store:
  kind: "mem"
```

## Event Types

Events are emitted as JSON lines to stdout and stored in the configured backend:

- `RUN_CREATED` - Run record created
- `SEED_LOADED` - Seed manifest loaded
- `NS_CREATED` - Namespaces created
- `MOUNT_DONE` - Mounts configured
- `CGROUP_APPLIED` - Cgroups limits applied
- `IDMAP_APPLIED` - UID/GID mapping applied
- `CAPS_DROPPED` - Capabilities dropped
- `SECCOMP_ENABLED` - Seccomp filter enabled
- `PROCESS_START` - Process started (PID)
- `PROCESS_EXIT` - Process exited (code)
- `PROCESS_FAILED` - Process failed (error)

### Example Event

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

## Rootfs Setup

For testing, you can create a minimal rootfs with busybox:

```bash
mkdir -p examples/rootfs-busybox/{bin,proc,dev,sys,tmp}

# Download busybox static binary
wget https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox -O examples/rootfs-busybox/bin/busybox
chmod +x examples/rootfs-busybox/bin/busybox

# Create symlinks for common commands
cd examples/rootfs-busybox/bin
./busybox --install .
cd ../../..
```

Or use an existing container rootfs:

```bash
# Extract from Docker image
docker export $(docker create busybox) | tar -C examples/rootfs-busybox -xvf -
```

## Storage Backends

### Memory Store (MVP)

- In-memory storage of seeds and runs
- Events written to stdout as JSON lines
- No persistence across restarts

### Liminal-DB Store (Stub)

- Placeholder for future Liminal-DB integration
- Currently mirrors events to stdout
- Ready for API integration

To implement Liminal-DB integration, update `src/store/liminal.rs` with:
- Connection pool initialization
- API client for seed/run/event persistence
- Error handling and retries

## Development

### Project Structure

```
.
├── Cargo.toml
├── src/
│   ├── main.rs          # Entry point
│   ├── cli.rs           # CLI interface
│   ├── seed.rs          # Seed config parser
│   ├── events.rs        # Event model
│   ├── process.rs       # Process runner
│   ├── isolate/
│   │   ├── mod.rs       # Isolation coordinator
│   │   ├── ns.rs        # Namespaces
│   │   ├── mount.rs     # Mounts
│   │   ├── idmap.rs     # UID/GID mapping
│   │   ├── cgroups.rs   # Cgroups v2
│   │   ├── caps.rs      # Capabilities
│   │   └── seccomp.rs   # Seccomp
│   └── store/
│       ├── mod.rs       # Store trait
│       ├── mem.rs       # Memory store
│       └── liminal.rs   # Liminal-DB adapter
├── examples/
│   ├── seed-busybox.yaml
│   └── rootfs-busybox/
└── README.md
```

### Running Tests

```bash
cargo test
```

### Logging

Set `RUST_LOG` for detailed logging:

```bash
RUST_LOG=debug ./target/release/gl run -f examples/seed-busybox.yaml
```

## Limitations (MVP)

This is an MVP (Iteration 1) with the following limitations:

1. **Capabilities**: Stub implementation (relies on no_new_privs)
2. **Seccomp**: Stub implementation (planned for future iterations)
3. **Network**: Creates network namespace but no veth/bridge setup
4. **Mount**: Uses chroot instead of pivot_root
5. **Liminal-DB**: Stub adapter (integration pending)

These will be addressed in future iterations.

## Iteration 2 - Garden (Pod) - In Progress

Garden introduces multi-container pods with shared networking and advanced features.

### Components Implemented

1. **Data Types** (`src/seed.rs`):
   - `Garden` - Pod manifest with multiple containers
   - `Container` - Individual container configuration
   - `ContainerRootfsConfig` - Support for path or OverlayFS layers
   - `RestartPolicy` - Never/OnFailure/Always

2. **OverlayFS Support** (`src/isolate/overlay.rs`):
   - Multi-layer lower directories
   - Upper/work directory management
   - Automatic mount/unmount

3. **Network Isolation** (`src/isolate/net.rs`):
   - Bridge creation and management (gl0)
   - veth pair setup (host <-> container)
   - IP allocation (simple IPAM 10.44.0.0/24)
   - DNS configuration
   - Network namespace management

4. **Content-Addressable Storage** (`src/store/cas.rs`):
   - Layer indexing by digest
   - Path resolution for OCI layers

5. **Security Policies** (`src/store/pacts.rs`):
   - Versioned security policies (Pacts)
   - Pre-loaded profiles: "minimal", "web-api@1"
   - Seccomp profile definitions
   - Capability lists

6. **Metrics Collection** (`src/metrics.rs`):
   - Periodic cgroups metrics collection
   - Memory, CPU, PIDs tracking
   - Per-container and aggregated metrics

7. **Event System** (`src/events.rs`):
   - Pod-level events: POD_NET_READY, POD_EXIT, POD_HEALTH
   - Container-level events: CONTAINER_START, CONTAINER_EXIT
   - Metric events with structured data

### Example: Garden Pod

See `examples/garden-echo.yaml` for a complete pod definition with:
- Multiple containers (api + sidecar-logger)
- OverlayFS layers for api container
- Network configuration (bridge mode, IP, DNS)
- Security policies (pacts)
- Resource limits per container
- Restart policy

### Iteration 2 Status: ✅ COMPLETE

All core components implemented and functional:

- [x] **Pod supervisor implementation** (`src/pod.rs`)
  - Container lifecycle management (Init → Starting → Running → Exited → Backoff)
  - Restart policies: Never/OnFailure/Always
  - Exponential backoff with configurable base/factor/max
  - Crash loop detection (max 20 restarts per 10 minutes)

- [x] **CLI commands**
  - `gl garden inspect -f <file>` - Validate and print garden manifest
  - `gl garden run -f <file>` - Run pod with full isolation

- [x] **Network setup**
  - Bridge creation (gl0)
  - veth pair per pod
  - IP allocation (IPAM)
  - DNS configuration

- [x] **Metrics collection**
  - Framework ready for periodic collection
  - Memory, CPU, PIDs tracking
  - Per-container and aggregated metrics

- [x] **Event system**
  - Pod-level events: POD_NET_READY, POD_EXIT
  - Container events: CONTAINER_START, CONTAINER_EXIT
  - Metric events

### Quick Start - Garden (Pod)

```bash
# Inspect a garden manifest
./target/debug/gl garden inspect -f examples/garden-echo.yaml

# Run a pod (requires sudo for network/cgroups)
sudo ./target/debug/gl garden run -f examples/garden-echo.yaml --store mem --metrics-interval 2
```

## Roadmap

- **Iteration 1**: ✅ Complete - Single process isolation (Sprout + Pulse)
- **Iteration 2**: ✅ Complete - Multi-container pods (Garden + OverlayFS + Network)
- **Iteration 3**: OCI image pull, registry integration, image layers
- **Iteration 4**: Liminal-DB full integration, persistent event/metrics storage
- **Iteration 5**: CNI plugins, advanced networking, service mesh ready

## License

MIT

## Credits

Generated with Claude Code
https://claude.com/claude-code
