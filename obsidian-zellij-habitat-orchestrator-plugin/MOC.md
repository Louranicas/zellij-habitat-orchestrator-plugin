# Zellij Habitat Orchestrator Plugin — Map of Content

> Back to: [[CLAUDE.md]] · [[CLAUDE.local.md]] · in-repo [README](../README.md) · [docs/INDEX](../docs/INDEX.md)

Dedicated vault for **`zellij-habitat-orchestrator-plugin` v0.1.2** — a Zellij WASM
dashboard + durable orchestrator-kernel sidecar. This vault is the Obsidian
companion to the in-repo `docs/` set; the repo `docs/` remain the canonical
source, these notes are the navigable, cross-linked mirror.

**Identity:** v0.1.2 · tag `v0.1.2` · commit `2a32442d…` · wasm sha `4dcd8c60…` ·
5 crates · ~10,780 LOC · 365 tests · MIT OR Apache-2.0 · standalone-push only.

---

## Entry & Orientation

- [[HOME]] — vault entry, quick-start reading paths, headline metrics
- [[Release & Provenance]] — v0.1.2 identity, remotes, tag, wasm sha, publish checklist

## Architecture

- [[Architecture Schematics]] — 5-crate map, runtime planes, admission boundary (Mermaid)
- [[Orchestrator Kernel Sidecar — Durable Admission Engine]] — event log, hash chain, idempotency, policy warrants, recipes
- [[Dashboard Modules]] — the 11 modules, data sources, keybinds
- [[Command Surface]] — `orch-kernelctl` CLI, Zellij pipe protocol, scripts, layouts

## Security & Quality

- [[Security & Admission Boundary]] — fail-closed model, policy-hash gate, observe-only witnesses
- [[Diagnostics]] — gate matrix, proof scripts, expected pass signals
- [[Task Status & Roadmap]] — P0–P6 hardening arcs, what's done vs gated

## Operations & Failure Modes

- [[Problem Solving]] — runbook for common build/launch/sidecar failure modes
- [[Bugs & Known Issues]] — known issues + linked RCAs (CPU saturation, memory exhaustion)

## Deep-dive notes (`notes/`)

- [[notes/P0 P1 Security Audit (2026-04-22)]] — full cmd_pipe trace, WASM boundary, dead-struct audit
- [[notes/Bridge Client & Polling Engine]] — `BridgeClient` internals, stagger, result routing, config validation
- [[notes/Plugin in Habitat Context (Factory Map)]] — live session topology, decision map, vs `habitat-nexus`
- [[notes/Durable Lessons & Design Decisions]] — L1–L5 invariants + D1–D3 design decisions (from NOTES.md + PLAN.md)
- [[notes/CPU Saturation RCA Summary]] — subprocess storm root cause, fixes, standing constraints

---

## External canonical anchors (live in main vault / workspace ai_docs)

- `[[Factory Map — Zellij L0 & Witness Plugins (S1008584)]]` — factory-map framing of the witness pane
- `[[CPU Saturation — fiber-cockpit Subprocess Storm (S1008517)]]` — canonical `ai_docs/CPU_SATURATION_RCA_S1008517.md`
- `[[Zellij Habitat Memory-Exhaustion Crash — RCA (S1008630)]]` — canonical `ai_docs/ZELLIJ_0443_SERVER_PTY_DOUBLE_PANIC_RCA_S1008630.md`
- `[[ULTRAPLATE Master Index]]` — Tier-2 entry (to be wired)
