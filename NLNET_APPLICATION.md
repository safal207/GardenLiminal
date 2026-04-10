# NLNet NGI Zero Entrust — Application

> This document is a draft application for the NLNet NGI Zero Entrust fund.
> Submit at: https://nlnet.nl/entrust/
> Deadline: check current call on nlnet.nl

---

## 1. Project name

**GardenLiminal: Audit-native container isolation runtime**

---

## 2. Website / repository

- https://github.com/safal207/GardenLiminal
- https://github.com/safal207/LiminalBD

---

## 3. Abstract (max 200 words)

GardenLiminal is a Linux container runtime written in Rust that treats audit events as a core primitive. Every isolation step — namespace creation, cgroup enforcement, capability dropping, seccomp activation, process start and exit — emits a structured, timestamped event before any user code runs.

These events are persisted in LiminalDB, a companion event store that retains the full lifecycle history of every workload. Together, the two projects provide a compliance-grade audit trail without requiring external monitoring agents.

Current container runtimes (containerd, Podman, CRI-O) were designed for execution, not accountability. Organisations in regulated sectors — healthcare, finance, critical infrastructure — must attach separate observability tools (Falco, Sysdig) to achieve audit coverage. These tools observe from outside and may miss events during startup or crash scenarios.

GardenLiminal eliminates this gap by instrumenting from within. The runtime itself is the audit source. Written entirely in Rust, it provides memory safety guarantees in a security-critical code path — the boundary between the host kernel and isolated processes.

The requested funding will complete the integration between GardenLiminal and LiminalDB, add CNI networking support, and fund an independent security review.

---

## 4. Have you been involved with projects or organisations relevant to this topic before?

I am the sole author of both GardenLiminal and LiminalDB. Both projects were created independently as open-source Rust software addressing process isolation and event persistence.

GardenLiminal implements Linux namespaces, cgroups v2, seccomp BPF filtering, pivot_root-based mount isolation, OCI image support, volume management, secrets handling, and Prometheus metrics — approximately 6 000 lines of Rust.

LiminalDB implements an event store with WebSocket transport, CBOR/JSON encoding, a custom query language (LQL), journal-based persistence with Sled backend, and timeline replay — approximately 8 000 lines of Rust across 8 crates.

Both projects are MIT licensed and under active development.

---

## 5. Requested amount

**EUR 30 000**

---

## 6. Project description

### Problem

Container runtimes are part of the trusted computing base (TCB) of any system that uses them. A vulnerability or misconfiguration at the isolation boundary can expose the host kernel to untrusted code. Regulated industries require evidence that isolation was correctly applied — not just that it was configured.

Today's runtimes log minimally and do not retain a structured record of what happened between "container created" and "container exited". The gap is filled by external agents that attach via eBPF or ptrace, adding complexity, attack surface, and operational cost.

### Solution

GardenLiminal is designed so that the runtime itself produces the audit record. The event system is not a logging add-on — it is woven into the isolation sequence. Each step emits a JSON event before proceeding to the next:

```
NS_CREATED → CGROUP_APPLIED → CAPS_DROPPED → SECCOMP_ENABLED → PROCESS_START → PROCESS_EXIT
```

Events flow over WebSocket to LiminalDB, where they are stored as impulses — queryable, replayable, and persistent. LiminalDB provides LQL (Liminal Query Language) for structured queries and a timeline mirror for full replay of container history.

### What the funding will cover

| Milestone | Description | Amount |
|---|---|---|
| M1: LiminalDB auth | Implement API key handshake between runtime and event store, preventing unauthorized event injection | EUR 5 000 |
| M2: DNS server | Replace /etc/hosts fallback with a real UDP DNS resolver for service discovery within Gardens | EUR 4 000 |
| M3: CNI plugin support | Standard Container Network Interface integration for production cluster deployments | EUR 6 000 |
| M4: Security audit | Independent review of isolation primitives (pivot_root, seccomp, namespaces, capabilities) by an external auditor | EUR 8 000 |
| M5: Documentation site | User guide, architecture docs, and contributor onboarding to grow the community | EUR 4 000 |
| M6: Conformance tests | End-to-end test suite verifying isolation correctness under adversarial conditions | EUR 3 000 |
| **Total** | | **EUR 30 000** |

### Timeline

12 months from grant start. Milestones delivered sequentially, each with a public report.

---

## 7. Comparison with existing projects

| Project | Approach | Audit capability |
|---|---|---|
| **containerd** (CNCF) | General-purpose runtime, Go | External logging only |
| **Podman** (Red Hat) | Daemonless, Go | journald integration, no structured events |
| **gVisor** (Google) | User-space kernel, Go | Syscall interception, no lifecycle audit |
| **Kata Containers** (OpenInfra) | MicroVM isolation | VM-level logs, no per-step events |
| **Falco** (Sysdig/CNCF) | eBPF runtime monitoring | Observes from outside, requires separate deployment |
| **GardenLiminal** | Audit-native runtime, Rust | Every isolation step emits structured event from within |

GardenLiminal is not competing with containerd or Podman — it occupies a different niche. It is designed for environments where auditability matters more than ecosystem compatibility. The long-term goal is to provide a runtime that can be used alongside OCI-compatible tools while adding the audit layer they lack.

---

## 8. Technical challenges

1. **pivot_root correctness in user namespaces**: The interaction between `pivot_root(2)`, mount propagation, and rootless containers has subtle failure modes. Our implementation handles bind-mount → pivot → lazy unmount, but needs adversarial testing.

2. **Seccomp profile completeness**: The current three profiles (strict, minimal, default) cover common workloads. Production use requires per-workload customisation and potential integration with OCI seccomp profile format.

3. **WebSocket reliability**: The connection between GardenLiminal and LiminalDB must handle network partitions gracefully. Current implementation includes offline fallback and reconnection, but needs WAL-based buffering for guaranteed delivery.

4. **DNS resolution in isolated namespaces**: The `.garden` DNS schema works via `/etc/hosts` today. A real UDP resolver that operates inside the network namespace requires careful handling of socket permissions in rootless mode.

---

## 9. How does this project relate to the goals of NGI Zero Entrust?

NGI Zero Entrust funds "technologies that enhance trust in the internet". Container isolation is a trust boundary — the user trusts that their workload is isolated from the host and from other workloads. GardenLiminal makes that trust **verifiable** by producing an audit trail of every action the runtime takes to enforce isolation.

The project is:
- **Open source** (MIT license)
- **Security-focused** (Rust, seccomp BPF, pivot_root, capability dropping)
- **Privacy-respecting** (secrets in tmpfs, value masking in logs, strict file permissions)
- **Interoperable** (OCI images, Prometheus metrics, WebSocket transport)
- **European-value-aligned** (transparency, auditability, user control)

The combination of a memory-safe runtime language and built-in audit trail directly addresses the NGI goal of trustworthy infrastructure for the next generation internet.
