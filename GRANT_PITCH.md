# GardenLiminal — Grant Application Pitch

## One-line summary

An open-source container runtime written in Rust that makes compliance-grade audit trails a built-in primitive, not an afterthought.

---

## The problem

Modern containerisation infrastructure — containerd, Podman, CRI-O — was designed for speed and compatibility, not for auditability. When a regulated organisation (healthcare, finance, critical infrastructure) needs to answer "what exactly happened inside this container, and when?", they must bolt on a separate security agent (Falco, Sysdig, Tetragon). That agent runs in a different process, may miss events, adds its own attack surface, and requires ongoing maintenance.

The root cause is architectural: today's runtimes do not treat lifecycle events as first-class data.

---

## The solution

**GardenLiminal** is a Linux container runtime that emits a structured, immutable event for every isolation step it takes — before any process code runs. There is no agent to deploy, no sidecar to maintain. The audit trail is the runtime.

```
namespace created   → { event: NS_CREATED,      ts, run_id }
cgroup limits set   → { event: CGROUP_APPLIED,   ts, run_id, limits }
capabilities dropped→ { event: CAPS_DROPPED,     ts, run_id, caps }
seccomp applied     → { event: SECCOMP_ENABLED,  ts, run_id, profile }
process started     → { event: PROCESS_START,    ts, run_id, pid }
process exited      → { event: PROCESS_EXIT,     ts, run_id, code }
```

Events are persisted in **LiminalDB** — a companion event store built on the same principles — and are queryable via LQL, its own query language.

---

## Technical design

| Layer | Technology | Security property |
|---|---|---|
| Runtime language | Rust | Memory safety — no buffer overflows, no use-after-free |
| Root filesystem | `pivot_root(2)` | Correct mount isolation (not `chroot` escape-prone) |
| Syscall filtering | seccomp BPF (3 profiles) | Kernel attack surface reduction |
| Capabilities | `PR_SET_NO_NEW_PRIVS` + drop list | Privilege containment |
| Rootless | User namespaces + UID/GID mapping | No root on host required |
| Audit persistence | LiminalDB via WebSocket | Tamper-evident event log |
| Metrics | Prometheus HTTP | Standard observability |

The codebase is ~6 000 lines of Rust, MIT licensed, with 100% passing integration tests.

---

## What makes it different

Most container runtimes were designed to run workloads. GardenLiminal was designed to **remember** them.

The project grew from a need to understand what lives at the threshold — the liminal space — between a host operating system and an isolated process. Every event the runtime emits is a record of a moment at that boundary. LiminalDB stores those moments as living impulses that can be replayed, queried, and analysed.

This is not a feature added on top of an existing runtime. It is the design intent from the first line of code.

---

## Current state (April 2026)

- [x] Full Linux isolation: namespaces, cgroups v2, `pivot_root`, seccomp BPF, capabilities
- [x] Multi-container pods (Garden) with restart policies and crash loop detection
- [x] OCI image support (layer unpacking, content-addressable storage)
- [x] 5 volume types: emptyDir, hostPath, namedVolume, config, secret
- [x] Secrets: tmpfs-backed, strict Unix permissions (0400), version rotation
- [x] Versioned security policies (Pacts): `minimal`, `web-api@1`, custom
- [x] Service discovery: `service-name.pod-name.garden` DNS schema
- [x] Prometheus metrics exporter (`:9464`)
- [x] LiminalDB integration: events over WebSocket as impulses
- [ ] LiminalDB auth handshake (API key)
- [ ] CNI plugin support
- [ ] Full DNS UDP server

---

## Requested funding and use of funds

| Item | Purpose |
|---|---|
| LiminalDB auth integration | Complete the secure channel between runtime and event store |
| DNS UDP server | Replace `/etc/hosts` fallback with real resolver |
| CNI plugin support | Standard network interface for cluster deployments |
| Security audit | Independent review of isolation primitives |
| Documentation site | Lower the barrier for new users and contributors |

---

## Why now

The container security landscape is consolidating around eBPF-based observability tools. GardenLiminal takes a complementary approach: instead of observing from outside, it instruments from within. The two approaches are not competing — they are complementary. A runtime that natively emits structured events is a better source of truth than kernel tracing alone.

Rust is increasingly the language of choice for security-critical infrastructure. The Linux kernel itself is accepting Rust. Writing a container runtime in Rust — with memory safety, no GC, and explicit ownership — is the right technical foundation for the next decade of infrastructure tooling.

---

## Ecosystem

- **GardenLiminal** (this project): the runtime
- **LiminalDB** ([github.com/safal207/LiminalBD](https://github.com/safal207/LiminalBD)): the event store

Both projects are open-source, MIT licensed, written in Rust, and share a common philosophical foundation: processes have a lifecycle, and that lifecycle deserves to be remembered.

---

## Contact

Repository: [github.com/safal207/GardenLiminal](https://github.com/safal207/GardenLiminal)
LiminalDB:  [github.com/safal207/LiminalBD](https://github.com/safal207/LiminalBD)
