# HOME — Zellij Habitat Orchestrator Plugin

> Back to: [[MOC]] · in-repo [README](../README.md)

## What this is

Two components fused into one repo, divided by a hard security boundary:

1. **A terminal-native observability dashboard** — a Zellij WASM plugin that
   renders live Habitat telemetry (ORAC, PV2, 14 services, campaigns, fibers,
   spheres) inside a pane.
2. **A durable admission kernel** — a host-side SQLite-backed sidecar that is the
   *only* component permitted to perform irreversible action, and only by writing
   a hash-chained, idempotent, policy-checked event.

## First principle

> **Observation must be cheap, attributed, and reversible; durable action must
> cross the sidecar boundary and return proof.**

The plugin may render, poll, serialize pane state, and forward pipe messages. It
does not invent durability. `ACK_DURABLE` belongs to the sidecar submit path only,
after event append, hash-chain update, idempotency resolution, and policy-bound
warrant checks. See [[Security & Admission Boundary]].

## Headline metrics (v0.1.2)

| Metric | Value |
|---|---|
| Crates | 5 (`habitat-core`, `habitat-modules`, `habitat-bridge-client`, `habitat-plugin`, `orchestrator-kernel-sidecar`) |
| Rust LOC | ~10,780 |
| Tests | 365 (all below the WASM line) |
| Dashboard modules | 11 |
| Zellij layouts | 4 |
| Ops/proof scripts | 20 |
| CLI subcommands (`orch-kernelctl`) | 8 |
| License | MIT OR Apache-2.0 |
| Release | tag `v0.1.2`, commit `2a32442d…`, wasm sha `4dcd8c60…` |

## Quick-start reading paths

- **"What does it do?"** → [[Architecture Schematics]] → [[Dashboard Modules]]
- **"How do I drive it?"** → [[Command Surface]]
- **"Is it safe?"** → [[Security & Admission Boundary]] → [[Diagnostics]]
- **"What's the durable engine?"** → [[Orchestrator Kernel Sidecar — Durable Admission Engine]]
- **"What's broken / next?"** → [[Bugs & Known Issues]] → [[Task Status & Roadmap]]

## The one constraint that shapes everything

`habitat-plugin` depends on `zellij_tile`, which **only compiles to
`wasm32-wasip1`** — so it cannot run host `cargo test`. All testable logic is
pushed *down* into the three host crates, which must never import `zellij_tile`.
The 365 tests live entirely below the WASM line; `build.sh` is the plugin crate's
only "test."
