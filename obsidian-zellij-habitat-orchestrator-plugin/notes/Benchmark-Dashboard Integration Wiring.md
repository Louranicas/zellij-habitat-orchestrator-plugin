# Benchmark-Dashboard Integration Wiring

> Back to: [[MOC]] · [[Executive Summary]] · [[Orchestrator Kernel Sidecar — Durable Admission Engine]]
> Canonical spec: `Benchmark-Dashboard/docs/08_PLUGIN_INTEGRATION_WIRING.md` (v2: R1–R6 integrated · §15 ledger · §16 cross-ref)
> Final-vision schematics: `Benchmark-Dashboard/docs/09_FINAL_VISION_SCHEMATICS.md` (L1/L2/dataflow/launch/state-machine/compat-matrix · Mermaid · `launch dashboard` seamless-run proof)
> Status: ✅ **BUILT + VERIFIED** (S1008811, 2026-06-27) — 6 files / ~190 additive lines · 121/121 smokes · live `/api/plugin` + `/api/eval` confirmed (F6 pass, F7 FAIL = liveness guard working). Not committed (awaiting go).

> **✅ Dual-frame gap analysis (3 ship-blockers) now INTEGRATED into the plan body + verified against source (spec §16):**
> 1. **N-2 liveness≠presence:** tier-1 derives `last_event_age_s` from `created_at` (`"<epoch_ms>Z"`, `lib.rs:1502`); badge shows "last advance Δt ago"; **eval F7** (weight 1) fails on a frozen chain. The live 27h-stale DB now correctly reads red.
> 2. **N-1 substrate-write boundary:** **two-tier read** — tier-1 liveness/counts/edges via genuinely `mode=ro` sqlite (no mutation, 15s); tier-2 authoritative chain-integrity via `orch-kernelctl snapshot-v2` (the RW `initialize()` open) at **60s**. Upstream `--read-only` flag requested (§15.4).
> 3. **N-3+C-4+C-5 honesty bundle:** `pipe.*` rendered as disclosed *unmeasured note* (not a live posture); edge matcher fixed (`MEASURED`=coherent); eval **B6 dropped** (tautology), replaced by F7; fitness gauge relabelled "Integrity Ladder (derived)" (N-4 collinearity).
>
> Every query/table/column/format in the plan maps to a sidecar source line in **spec §16** (e.g. queue_depth `lib.rs:1192-1195`, edges `lib.rs:1164-1170`, age-format `lib.rs:1501-1502`). Corrected the false "binary absent" claim (it's installed).

The plan to wire this plugin's **durable orchestrator-kernel sidecar** into the habitat's
web **Benchmark-Dashboard** (`127.0.0.1:8088`) as a read-only telemetry panel + folded-in
eval cases. The dashboard becomes the sidecar's **web witness** — reading the durable
hash-chain the WASM plugin deliberately cannot expose (no socket/disk in `wasm32-wasip1`).

---

## The two-surface relationship

| | This plugin (WASM) | Benchmark-Dashboard (Python) |
|---|---|---|
| Reads durable sidecar log? | **No** — WASM sandbox | **Yes** — shells `orch-kernelctl` |
| Lifetime | ephemeral (Zellij pane) | persistent + charted |
| Durable write | `NACK_USE_SIDECAR_SUBMIT` | none (read-only panel) |

The sidecar's `snapshot-v2` schema already ships a `dashboard_truth` block
(`measured_only`, `stale_fields`) — **the integration was anticipated by the schema**
(`crates/orchestrator-kernel-sidecar/src/lib.rs:426-456`).

## The read bridge (one shell-out)

`orch-kernelctl snapshot-v2 --json` → returns, in a single call:
- `sidecar.verify_chain_ok` — **live** hash-chain verification (`lib.rs:712,756`)
- `fitness.score` — integrity ladder: `0.0` chain-broken · `0.74` edge-gap · `0.80` healthy
- `edges[]` — measured witness states with `evidence_ref`
- `pipe.mode = "A_FAIL_CLOSED"` — the fail-closed admission posture
- `dashboard_truth.{measured_only,stale_fields}` — projection honesty

Binary path (per `scripts/orch-kernel-deploy.sh`): `~/.local/bin/orch-kernelctl`.
State dir: `$ORCH_KERNEL_STATE_DIR` else `<workspace>/Orchestrator/operator-kernel/state`
(live `orchestrator-kernel.sqlite`, 7.4 MB).

## The eval bridge (the value-add)

`verify_chain_ok` becomes eval case **F6** (faithfulness, weight 3) and sidecar reachability
+ fail-closed pipe becomes **B6** (bridge accuracy, weight 2) in the dashboard's existing
Agent-Eval suite — charted over time via the `eval_run` trend. No-green-without-receipt
applied to the integration itself: a dead sidecar fails honestly, never silently.

## Deploy shape (additive — ~129 lines, 6 files, no refactor)

| File | Change |
|---|---|
| `collectors.py` | `_run(env_extra)` + `plugin()` collector + F6/B6 eval cases |
| `bench_dashboard.py` | 1 route: `/api/plugin` |
| `web/index.html` | "Orch Kernel" tab + section |
| `web/app.js` | `LOADERS.plugin` + `loadPlugin()` |
| `config/dashboard.json` | `poll_intervals.plugin: 15` |
| `test/test_collectors.py` | 4 fail-soft smokes |

Prerequisite: `bash scripts/orch-kernel-deploy.sh` (build+install `orch-kernelctl`).
Full mechanical sequence + rollback + Mermaid schematics in the canonical spec.

## See also

- [[Orchestrator Kernel Sidecar — Durable Admission Engine]] — the engine being surfaced
- [[Security & Admission Boundary]] — why read-only (no `submit` from the dashboard)
- [[notes/D11 Witnesses — Source Deep Dive]] — the witness edges the panel renders
- [[notes/Build Deploy Rollback Pipeline]] — `orch-kernel-deploy.sh` prerequisite
