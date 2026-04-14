# NLnet NGI Zero Commons Fund - Application

> This document is a draft application for the NLnet NGI Zero Commons Fund.
> `NGI Zero Entrust` is closed as of January 2026.
> Check the active call at: https://nlnet.nl/commonsfund/

## 1. Project name

**GardenLiminal: Audit-native container isolation runtime**

## 2. Website / repository

- https://github.com/safal207/GardenLiminal
- https://github.com/safal207/LiminalBD

## 3. Abstract

GardenLiminal is a Linux container runtime written in Rust that treats audit
events as a core primitive. Every isolation step, from namespace creation and
cgroup enforcement to capability dropping, seccomp activation, process start,
and process exit, emits a structured event before user workload code runs.

These events can be persisted in LiminalDB, a companion event store that
retains the lifecycle history of workloads and supports replay through a mirror
timeline. Together, the projects provide a path toward compliance-grade runtime
observability without requiring a separate monitoring agent.

Current runtimes such as containerd and Podman were designed primarily for
execution, not accountability. In regulated sectors, teams often attach
external monitoring tools to fill the audit gap. GardenLiminal takes the
opposite approach: the runtime itself becomes the source of structured evidence.

The requested funding would support hardening the runtime, finishing missing
networking and auth integrations, improving documentation, and funding an
independent security review.

## 4. Relevant background

The author is the builder of both GardenLiminal and LiminalDB. GardenLiminal
implements Linux namespaces, cgroups v2, seccomp BPF filtering, `pivot_root`
mount isolation, OCI image handling, volume management, secrets handling, and
Prometheus metrics in Rust.

LiminalDB provides the companion storage layer with protocol surfaces, timeline
replay, and SDK access paths. GardenLiminal depends on that data layer for the
full audit-native story.

GardenLiminal is MIT licensed and under active development. LiminalDB is now
aligned to an Apache-2.0 repository license, which makes the stack materially
cleaner for an open infrastructure funding application.

## 5. Requested amount

**EUR 30,000**

## 6. Project description

### Problem

Container runtimes are part of the trusted computing base of any system that
executes untrusted workloads. A vulnerability or misconfiguration at the
isolation boundary can expose the host kernel or neighboring workloads.

At the same time, most runtimes do not retain a structured, replayable record
of what happened between "container created" and "container exited". Teams fill
that gap using external agents, which adds complexity, attack surface, and
operational cost.

### Solution

GardenLiminal is designed so that the runtime itself produces the audit record.
The event system is not a logging add-on; it is woven into the isolation
sequence:

```text
NS_CREATED -> CGROUP_APPLIED -> CAPS_DROPPED -> SECCOMP_ENABLED -> PROCESS_START -> PROCESS_EXIT
```

Events flow to LiminalDB, where they can be queried and replayed as a timeline.
This makes runtime behavior more inspectable and easier to verify.

### What the funding will cover

| Milestone | Description | Amount |
|---|---|---|
| M1: LiminalDB auth | Implement API key handshake between runtime and event store | EUR 5,000 |
| M2: DNS server | Replace `/etc/hosts` fallback with a real UDP DNS resolver | EUR 4,000 |
| M3: CNI plugin support | Standard CNI integration for cluster deployments | EUR 6,000 |
| M4: Security audit | Independent review of namespaces, `pivot_root`, seccomp, and capability handling | EUR 8,000 |
| M5: Documentation site | User guide, architecture docs, and contributor onboarding | EUR 4,000 |
| M6: Conformance tests | End-to-end test suite for isolation and audit behavior | EUR 3,000 |
| **Total** |  | **EUR 30,000** |

### Timeline

12 months from grant start, with public milestone reports.

## 7. Comparison with existing projects

| Project | Approach | Audit capability |
|---|---|---|
| containerd | General-purpose runtime | External logging only |
| Podman | Daemonless runtime | Journald integration, no structured lifecycle evidence |
| gVisor | User-space kernel | Strong isolation focus, not lifecycle audit-first |
| Kata Containers | MicroVM isolation | VM-level logging, not per-step runtime evidence |
| Falco | eBPF monitoring | External observation, separate deployment |
| GardenLiminal | Audit-native runtime | Structured lifecycle evidence from within the runtime |

GardenLiminal is not trying to replace all existing runtimes. It is aimed at
the part of the ecosystem where auditability, explainability, and safety at the
execution boundary matter more than maximum compatibility.

## 8. Technical challenges

1. `pivot_root` correctness in user namespaces and mount propagation edge cases.
2. Seccomp profile completeness and profile customization.
3. Reliable delivery of runtime events to LiminalDB during failures.
4. DNS service discovery inside isolated namespaces in rootless mode.

## 9. Relation to NGI Zero Commons Fund

NGI Zero Commons Fund supports open digital commons and trustworthy internet
infrastructure. Container isolation is a trust boundary: users trust that their
workloads are separated from the host and from each other. GardenLiminal makes
that trust more verifiable by emitting an explicit audit trail of the actions
the runtime takes to enforce isolation.

The project is relevant because it is:

- infrastructure-focused
- security-oriented
- privacy-conscious in its secrets and runtime handling
- interoperable with existing tooling and data layers
- aimed at transparency and user control

## 10. Submission notes

- Recheck all stack-wide grant language so it refers to the final repository
  licenses consistently.
- Keep the application framed around digital commons and trustworthy
  infrastructure rather than startup positioning.
