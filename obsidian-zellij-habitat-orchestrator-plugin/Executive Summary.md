# Zellij Habitat Orchestrator Plugin — Executive Summary

> Back to: [[MOC]] · [[HOME]] · [[ULTRAPLATE Master Index]] · [[Factory Map — Zellij L0 & Witness Plugins (S1008584)]]
> Repo: `zellij-habitat-orchestrator-plugin/` · GitHub: `https://github.com/Louranicas/zellij-habitat-orchestrator-plugin`
> Vault: [[MOC]] · Created S1008807

---

## What it is

A dual-component system that gives the ULTRAPLATE Habitat a **terminal-native control surface and a durable action record** — both living inside the Zellij workspace, no browser required.

**Component 1 — WASM Dashboard** (`habitat-plugin.wasm`, ~1.3 MB)
A Rust plugin running inside a Zellij pane. It observes the Habitat in real time: 10 modules pull live telemetry from ORAC, PV2, SYNTHEX, and 16 services via timed `curl` calls and host helper binaries. It renders what it sees; it writes nothing.

**Component 2 — Orchestrator Kernel Sidecar** (`orch-kernelctl`)
A host-side CLI backed by a SQLite WAL event log. When an operator wants a durable action (not just observation), the request crosses the sidecar boundary, is hash-chained into the event log, checked against a policy warrant, and returns a cryptographic receipt (`ACK_DURABLE`). No action is durable until the sidecar confirms it.

---

## What it can do

| Capability | Detail |
|---|---|
| **Live Habitat dashboard** | 7 core modules: service health grid (16 services), Kuramoto field coherence (`r`, coupling, Hebbian), ORAC bridge/thermal state, NA proposals, session stats, event feed, pipe command log |
| **Factory coordination visibility** | 3 D11 witness modules: `fiber_cockpit` (hopf campaign fibers + kv-lease table), `campaign_attention` (ambient change alerts — fiber grew, lease near expiry, arming key flipped), `sphere_warden` (pane↔PV2 sphere coverage gap) |
| **Durable task admission** | `orch-kernelctl submit` records any task as an append-only, hash-chained event. Replay of the same canonical request returns the original receipt — idempotent by construction. Conflicts surfaced, never silently overwritten |
| **Policy-bound warrants** | A JSON policy file (`warrants.v2.json`) is hash-checked both ways before any task is admitted. One allowlisted recipe: `verify_chain`. No arbitrary shell execution |
| **Fail-closed pipe protocol** | `zellij pipe` commands return `NACK_USE_SIDECAR_SUBMIT` for valid kernel requests — the plugin explicitly refuses to claim durability. Invalid schema → `NACK_SCHEMA_INVALID` without attempting submission |
| **Hot-reload state persistence** | Modules serialise scroll position, selected campaign, last snapshot on `q`/`Esc` and restore on next `LaunchOrFocusPlugin` |
| **4 ready-made layouts** | Fleet (50/50, 7 modules), Compact (30/70, 3 modules), Minimal (4-row footer, 1 module), Factory-Witness (bash-poll panels for factory-status/wiring/proof-seal) |
| **1134 host tests (v0.1.3)** | 428 dcg-admit + 204 orchestrator-perceive + 362 modules + 74 core + 42 sidecar + 24 bridge-client / 0 failed; `forbid(unsafe_code)`, `--all-targets` pedantic-clean. (v0.1.2 sealed at 365 tests / score 90/90; zero-touch verifier 12 gates PASS) |

---

## How it value-adds to the Zellij Habitat

**1. Single pane of truth, no context switch**
Before: checking Habitat health meant `curl` commands in a shell, Obsidian notes, and mental model stitching. Now: `Alt Shift h` floats the dashboard in any tab — 16-service grid, field coherence, Hebbian state, NA proposals — in one bounded pane that disappears when not needed.

**2. The factory fabric made visible**
The three D11 witnesses surface what was previously invisible without bespoke tooling: which campaign fibers are live, which leases are near expiry, which arming keys are set, and how many Zellij panes lack PV2 sphere registrations. This is the coordination medium rendered as a live terminal UI.

**3. Observation and action are architecturally separated**
The WASM boundary enforces the discipline: the plugin *can't* write — it runs sandboxed in `wasm32-wasip1` with no socket stack and no disk access beyond Zellij's own plugin-state. Durable actions are explicitly routed to the sidecar, which appends, hashes, and receipts them. Observation is cheap and always safe; durability is deliberate and always provable.

**4. Audit trail by construction**
Every admitted task is an append-only SQLite event with a sha256 hash-chain linking it to its predecessor. `verify-chain` checks the full chain in one command. Replay returns the original receipt. The chain is the proof — no separate logging required.

**5. Replaces a brittle predecessor cleanly**
`habitat-nexus` was a 2,641-LOC monolith with zero test isolation. `habitat-plugin` is a 4-crate workspace: `habitat-core` (trait + types), `habitat-modules` (10 modules), `habitat-bridge-client` (scheduler), `habitat-plugin` (Zellij wiring). Tests live in the three host crates — fully runnable on `x86_64`. A CI gate is a `cargo test --workspace` away.

---

## In one sentence

The Orchestrator Plugin gives the Zellij Habitat **eyes** (live telemetry dashboard), a **nervous system** (factory coordination visibility), and a **spine** (durable, hash-chained, policy-bound task admission) — all from inside the terminal, with observation and action provably separated by the WASM boundary.

---

## Dive deeper

- [[Architecture Schematics]] — 5-crate map, runtime planes, WASM boundary
- [[Orchestrator Kernel Sidecar — Durable Admission Engine]] — hash chain, policy warrants, idempotency
- [[Dashboard Modules]] — all 10 modules with data sources and keybinds
- [[Security & Admission Boundary]] — fail-closed table, observe-only witnesses
- [[notes/D11 Witnesses — Source Deep Dive]] — `fiber_cockpit`, `campaign_attention`, `sphere_warden` source detail
- [[notes/Task Status Ledger (v0.1.2)]] — full gate results and operator-gated next steps
