# Task Status & Roadmap

> Back to: [[MOC]] · in-repo [docs/TASK_STATUS](../docs/TASK_STATUS.md) · [PLAN.md](../PLAN.md)

> **🟢 v0.1.3 SHIPPED (S1008937).** The v0.1.0→v0.1.2 two-arc plan below is **historical** —
> that arc completed. v0.1.3 added the perception organ `orchestrator-perceive`, the
> Delegation-Capacity Governor `dcg-admit`, the `orchestrator_witness` panel, and
> `orch-kernelctl --read-only` (7 crates · 12 modules · 1134 host tests). See [[Release & Provenance]].

Two-arc plan (v0.1.0→v0.1.2, historical): **Hardening** (5 phases) then **Participation** (NA-Z1→Z7).
Hardening plan: `synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md`.

## Hardening arc

| Phase | Status | Description |
|---|---|---|
| P0: Backup + security | ✅ COMPLETE (2026-04-22) | v0.1.0.wasm backed up (SHA 2be5217c…); cmd_pipe audit — cmd-injection NOT present (firewalled); ANSI injection MEDIUM → Phase 4 |
| P1: Constraints | ✅ COMPLETE (2026-04-22) | WASM boundary PASS; floating instances not reproducible (downgraded); 6/18 response structs dead (33%) |
| P2: Tests | ✅ COMPLETE (2026-04-24) | 163→ (now 365) tests; 8 dead structs deleted; 51 pedantic debts cleared; `ConfigWarning` validation; URL + poll-interval clamp [1.0, 300.0]; Tier-1/2 fixtures; all 4 gate stages green |
| P3: Drift + polling | ⛔ NOT STARTED | schema drift (`LiveDataCheck` + `has_live_data()`) + URL dedup + `[STALE Xs]` indicator → closes [[Bugs & Known Issues]] KI-2, KI-3 |
| P4: Modules + docs | ⛔ NOT STARTED | `cmd_pipe` ANSI sanitizer (closes KI-1) + `event_feed` HashSet dedup + `bridge_health` content validation |
| P5: Self-model | ⛔ NOT STARTED | NA-Z1 PluginHealth + NA-Z2 sphere registration |
| P6: Participation | ⛔ NOT STARTED | NA-Z3 temporal depth + NA-Z4 module autonomy |

## Sidecar roadmap (kernel)

- ✅ P2.0/P2.1 substrate: state paths, SQLite WAL, canonical hashing, replay,
  chain verify, constrained built-in recipe.
- ⛔ Long-lived UDS server — `orch-kerneld` is currently a snapshot-printing stub;
  the durable daemon is a later P2.0 increment after the CLI substrate seals.
- ⛔ Real effectors beyond `verify_chain` — admission proven first, execution
  deliberately deferred. See [[Orchestrator Kernel Sidecar — Durable Admission Engine]].

## Wave history

- **Wave-16 (S1005032):** `bridge_health` grid 13→14 — added `(8142, "WFE")`
  wf-daemon service. 91/91 modules tests, clippy clean, wasm redeployed.
- **S1007594 (D11 witnesses):** `command_sources()` self-poll trait; `fiber_cockpit`,
  `campaign_attention`, `sphere_warden` added read-only.
- **S1007736:** default module surface expanded to the 5-module agentic-factory set.

## Operator-gated (not repo work)

The remaining production items — arming, promotion, service restart, rollback
execution, long-running soak — are explicit operator workflows requiring Luke's
`factory.authorize.*` arming + separate receipts. They are **not** doc/code gaps.

## See also

- [[Release & Provenance]] — current sealed release
- [[Bugs & Known Issues]] — which phase closes each open issue
