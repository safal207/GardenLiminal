# GardenLiminal - Commons Fund Repository Brief

## Repository

- Project: `GardenLiminal`
- URL: https://github.com/safal207/GardenLiminal
- Role in stack: runtime and isolation layer
- License: `MIT`

## Positioning

GardenLiminal is the execution boundary of the Liminal Stack. It focuses on
running untrusted workloads with explicit isolation controls and built-in audit
evidence rather than treating observability as a separate afterthought.

For NLnet, the strongest value is as a reusable infrastructure component that
improves verifiability at the runtime layer. It can be used independently, but
becomes more powerful when paired with DAO_lim and LiminalBD in a trustworthy
stack.

## What reviewers should notice

- runtime-native audit trail for lifecycle events
- namespace, capability, and seccomp-based isolation model
- LiminalBD integration for persistent event history
- focus on self-hosted, inspectable infrastructure for AI workloads
- MIT licensing suitable for commons-fund positioning

## Proposed grant-facing scope

Within the shared stack application, GardenLiminal is the work package for:

- security hardening and external review
- improved seccomp profile set and runtime policy handling
- auth, DNS isolation, and CNI/networking work
- end-to-end stack demo with persistent audit evidence

## Submission notes

- Keep comparisons against Docker/containerd factual and modest.
- Avoid claiming completed features that are still roadmap items in README.
- Use this brief together with `NLNET_COMMONS_APPLICATION.md` for submission.
