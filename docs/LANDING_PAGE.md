# GardenLiminal Landing Page Copy

## Hero

### Run untrusted workloads with a runtime that leaves an evidence trail

GardenLiminal is a Rust container runtime built around lifecycle auditability.
It isolates workloads with Linux primitives and emits structured events so the
execution boundary is observable instead of opaque.

**Primary promise**

- isolate workloads with explicit runtime controls
- keep a structured lifecycle trail
- connect runtime evidence to LiminalDB for persistent history

**CTA**

- Run the stack demo
- Review security status

## Problem

Most runtimes focus on execution first and evidence later.

Teams who care about verifiability usually end up stitching together:

- the runtime
- external monitoring
- audit tooling
- custom log pipelines

That increases complexity while still leaving open questions:

- what exactly happened between start and exit?
- which isolation steps were actually applied?
- where is the replayable evidence trail?

## Agitation

If the execution boundary is opaque, incidents get more expensive:

- postmortems take longer
- isolation guarantees are harder to explain
- security review becomes partly interpretive
- audit evidence is scattered across tools

For AI workloads and autonomous systems, that is a bad trade.

## Solution

GardenLiminal is designed so runtime lifecycle events are first-class output,
not an afterthought.

It combines:

- Linux namespace and cgroup isolation
- seccomp-oriented hardening paths
- runtime event emission
- optional persistence into LiminalDB

That gives you a runtime story centered on execution plus evidence.

## What you get

### 1. Audit-native runtime behavior

The runtime emits structured lifecycle events for critical stages of workload
execution.

### 2. Open isolation model

The implementation is visible and inspectable in Rust, rather than hidden
inside proprietary runtime plumbing.

### 3. Better evidence flow

With LiminalDB integration, the runtime can feed a persistent, replayable event
trail instead of relying on plain logs alone.

### 4. Clear hardening roadmap

The repository now documents what is implemented, what is partial, and what is
still planned in `docs/SECURITY_STATUS.md`.

## Who this is for

- teams running sensitive or untrusted workloads
- builders of AI runtime infrastructure
- engineers who want runtime evidence, not just runtime execution
- reviewers who care about open, inspectable isolation paths

## Proof and credibility

- explicit security posture in `docs/SECURITY_STATUS.md`
- benchmark scaffold in `docs/BENCHMARKS.md`
- stack walkthrough in `docs/STACK_DEMO.md`
- demo script for LiminalDB integration in `examples/demo-liminaldb.sh`
- NLnet-facing materials in `grants/`

## Call to action

Use GardenLiminal if you want a runtime that is not just about "running the
container" but about making the execution boundary visible and reviewable.

Start with:

- `docs/STACK_DEMO.md`
- `docs/SECURITY_STATUS.md`
- `docs/BENCHMARKS.md`
- `grants/README.md`

## FAQ

### Is this already a fully hardened production runtime?

No. The strongest current value is the architecture, evidence model, and open
hardening direction. The repository now documents that honestly.

### Why not just use Docker or containerd plus extra tools?

You can. GardenLiminal exists for cases where audit-native runtime behavior is
itself part of the product value.

### What is the biggest difference?

The pitch is not "another runtime." The pitch is "a runtime that treats
lifecycle evidence as a core primitive."
