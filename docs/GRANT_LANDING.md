# GardenLiminal Grant Landing

## GardenLiminal

### A container runtime that treats lifecycle evidence as first-class output

GardenLiminal is the runtime layer of the Liminal Stack.
It is an open-source Rust container runtime focused on workload isolation plus
structured lifecycle evidence.

For grant reviewers, the core value is:

- runtime-level auditability
- open isolation logic
- evidence flow into a replayable storage layer

## Why this matters

Most runtimes are optimized for execution first.
Evidence usually comes later through external tooling.

That leaves a gap at the execution boundary:

- which isolation steps were actually applied?
- what happened between process start and exit?
- where is the persistent evidence trail?

For untrusted or sensitive workloads, that gap matters.

## What GardenLiminal contributes

GardenLiminal gives the stack a runtime that combines:

- Linux namespace and cgroup isolation
- seccomp-oriented hardening paths
- structured lifecycle events
- optional persistence into LiminalDB

The important distinction is this:
the runtime is not presented only as an execution engine, but as an evidence
producer at the point where workloads actually run.

## Why it fits a grant

GardenLiminal is relevant as a commons-oriented infrastructure component because
it is:

- open-source runtime infrastructure
- reusable outside the rest of the stack
- designed around inspectability rather than opacity
- useful for downstream builders who care about verifiable execution

It is not just another runtime pitch.
It is a proposal for a more accountable execution layer.

## Current reviewer evidence

- [Stack demo](STACK_DEMO.md)
- [Security status](SECURITY_STATUS.md)
- [Benchmarks](BENCHMARKS.md)
- [NLnet materials](../grants/README.md)

## What reviewers should remember

A trustworthy stack needs more than storage and routing.
It also needs an execution layer that can show what happened at runtime.

GardenLiminal exists to make that boundary more:

- visible
- inspectable
- replayable
- open

## Best next milestone

The strongest next milestone is deeper hardening evidence, especially:

- capability dropping completion
- DNS implementation beyond the MVP stub
- stronger Linux-only runtime verification
- external security review
