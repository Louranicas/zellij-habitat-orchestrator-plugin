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

## Headline metrics (v0.1.3)

| Metric | Value |
|---|---|
| Crates | 7 (`habitat-core`, `habitat-modules`, `habitat-bridge-client`, `habitat-plugin`, `orchestrator-kernel-sidecar`, `orchestrator-perceive`, `dcg-admit`) |
| Tests | 1134 host tests (all below the WASM line), `--all-targets` pedantic-clean, no `unsafe` code |
| Dashboard modules | 12 (incl. `orchestrator_witness`) |
| Zellij layouts | 4 (full-fleet, compact, minimal, factory-witness) |
| New organs | `orchestrator-perceive` (perception), `dcg-admit` (delegation governor) |
| CLI (`orch-kernelctl`) | read/write superset incl. `--read-only`, `snapshot-v2`, `latest_perceive` |
| License | MIT OR Apache-2.0 |
| Release | tag `v0.1.3`, HEAD `834625f…`, wasm sha `c5b9cce6…` (from-zero reproduced 2026-06-30) |

## Quick-start reading paths

- **"Give me the one-page brief"** → [[Executive Summary]]
- **"What does it do?"** → [[Architecture Schematics]] → [[Dashboard Modules]]
- **"How do I drive it?"** → [[Command Surface]]
- **"Is it safe?"** → [[Security & Admission Boundary]] → [[Diagnostics]]
- **"What's the durable engine?"** → [[Orchestrator Kernel Sidecar — Durable Admission Engine]]
- **"What's broken / next?"** → [[Bugs & Known Issues]] → [[Task Status & Roadmap]]

## The one constraint that shapes everything

`habitat-plugin` depends on `zellij_tile`, which **only compiles to
`wasm32-wasip1`** — so it cannot run host `cargo test`. All testable logic is
pushed *down* into the six host crates, which must never import `zellij_tile`.
The 1134 host tests live entirely below the WASM line; `build.sh` is the plugin
crate's only "test."
