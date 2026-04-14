# GardenLiminal Grant Readiness

## Positioning

GardenLiminal is the runtime layer of the Liminal Stack: a Rust container
runtime with audit-native isolation, lifecycle event persistence, and tight
integration with LiminalDB.

## Why it fits NGI Zero Commons Fund

- infrastructure component, not a single end-user product
- security and auditability are built into the runtime path
- addresses a clear gap in trustworthy AI workload execution
- integrates cleanly with the storage and routing layers of the stack

## Grant-facing strengths visible in the repository

- strong technical README with isolation and audit model
- existing [GRANT_PITCH.md](../GRANT_PITCH.md)
- existing [NLNET_APPLICATION.md](../NLNET_APPLICATION.md)
- Cargo metadata declares `MIT` in [Cargo.toml](../Cargo.toml)

## Readiness notes

- The repository now includes the MIT license text to match Cargo metadata.
- Existing grant material referenced the closed `NGI Zero Entrust` program and
  overstated the licensing status of LiminalDB. Those issues have been corrected
  in the updated application draft.

## Recommended next fixes before submission

- align all grant text with `NGI Zero Commons Fund`
- keep the stack-wide licensing statements consistent across all grant docs
