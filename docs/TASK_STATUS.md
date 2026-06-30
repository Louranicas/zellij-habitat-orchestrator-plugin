# Task Status

Back to [README](../README.md) · [Docs index](INDEX.md)

This ledger reconciles the historical [PLAN.md](../PLAN.md) task list with the
standalone `zellij-habitat-orchestrator-plugin` repository state.

> **Current release: v0.1.3** (S1008937) — tag `v0.1.3`, HEAD `834625f`, both remotes synced,
> wasm sha `c5b9cce6…` (from-zero reproduced 2026-06-30). Adds the perception organ
> `orchestrator-perceive`, the Delegation-Capacity Governor `dcg-admit`, the
> `orchestrator_witness` panel, and `orch-kernelctl --read-only`. **7 crates · 12 modules ·
> 1134 host tests / 0 failed · `forbid(unsafe_code)` · pedantic-clean.** The verification
> receipts below (dated 2026-06-26) are the **v0.1.2 seal** record, preserved as history;
> the v0.1.3 seal is the Ultimate Orchestrator P0–P5 campaign (S1008937).

## Status Legend

| Status | Meaning |
| --- | --- |
| Complete | Implemented and locally verified inside this repository. |
| Verified external | Verified from local machine state outside the repository. |
| Blocked | Requires an operator grant, external service state, or a different repository. |
| Out of scope | Belongs to a sibling plugin or upstream service, not this standalone export. |

## Historical Plan Reconciliation

| Plan item | Current status | Evidence | Remaining action |
| --- | --- | --- | --- |
| Task 1: establish `habitat-plugin` as successor to `habitat-nexus` | Complete | [PLAN.md](../PLAN.md) records the successor decision; this standalone repo now ships the named plugin and release docs. | None for this repo. |
| Task 2: wire launcher keybind | Verified external | `~/.config/zellij/config.kdl` binds `Alt Shift h` to `file:~/.config/zellij/plugins/habitat-plugin-v0.1.3.wasm` (repointed from v0.1.2 on 2026-06-30); `Alt h` is already bound to `MoveFocusOrTab "left"`. | Do not overwrite `Alt h` without explicit operator approval; document `Alt Shift h` as the active launcher. |
| Task 3: ship 50+ meaningful tests for `habitat-plugin` | Complete | `cargo test` passes 1134 host tests across the 6 host crates in the standalone repo. | None for `habitat-plugin`; sibling plugin test plans remain separate. |
| Task 4: clarify plugin/ORAC ownership boundary | Complete | [Architecture](ARCHITECTURE.md) and [Security](SECURITY.md) separate Zellij rendering, bridge transport, sidecar durability, and operator-gated production actions. | None for this repo. |
| Task 5: add shared snapshot/dispatch path | Partially complete | `habitat-bridge-client::SnapshotClient` exists with snapshot fan-out tests; `cmd_pipe` exposes `snapshot`, `query`, `coherence`, and `status`. | ORAC-side TTL/auth/version-header decisions remain upstream service work. |

## Release Repository Tasks

| Task | Status | Evidence |
| --- | --- | --- |
| Create separate standalone repository | Complete | Local repo: `/home/louranicas/claude-code-workspace/zellij-habitat-orchestrator-plugin`. |
| Publish GitHub remote | Complete | <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin> |
| Publish GitLab remote | Complete | <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin> |
| Tag v0.1.3 | Complete | Tag `v0.1.3` points to release commit `831182e` (S1008937); tag `v0.1.2` → `2a32442d…` retained as prior release. |
| Add Deep-Diff-Forge-style documentation | Complete | [README](../README.md), [Architecture](ARCHITECTURE.md), [Operations](OPERATIONS.md), [Testing](TESTING.md), [Security](SECURITY.md), [Release](RELEASE.md). |
| Preserve bidirectional docs links | Complete | Every file in `docs/` links back to [README](../README.md) and [Docs index](INDEX.md). |

## Verification Tasks

| Gate | Status | Evidence |
| --- | --- | --- |
| Markdown link integrity | Complete | Repository-local link checker returned `MARKDOWN_LINKS_OK`. |
| Rust formatting | Complete | `cargo fmt --all --check` passed. |
| Rust compile | Complete | `cargo check --workspace` passed. |
| Rust test suite | Complete | `cargo test` passed with 1134 host tests / 0 failed (v0.1.3; was 365 at the v0.1.2 seal). |
| Deep-Diff-Forge documentation review | Complete | Initial docs review reported 7 changed files, 549 additions, 110 deletions, and `semantic_fallbacks=0`; current task-ledger patch review reports 4 changed files, 92 additions, 1 deletion, and `semantic_fallbacks=0`. |
| Governance cap verifier | Complete | Workspace receipt `receipts/score-cap-verifier/orch-kernel-score-cap-verifier-20260626T051008Z.json` reports `verdict=pass`, `failed_checks=[]`, and authorizes the narrow gate-only cap lift from 82 to 90. |
| Zero-touch verifier classification | Complete | Workspace receipt `receipts/orch-kernel-v012-zero-touch-verify-20260626T085030Z/summary.json` reports `PASS`; score framework is `score=90 cap=90`; sha256 `ca7cee840071412cac354ec1fc668299ef182009ecde6a13f55acdd7ae5994e6`. |
| Pipe terminality | Complete for Mode A | Workspace receipt `receipts/orch-kernel-v012-live-pipe-proof-20260626T084548Z/summary.json` reports `PASS`; valid kernel pipe payloads return terminal `NACK_USE_SIDECAR_SUBMIT` without synchronous sidecar CLI execution inside the Zellij `CliPipe` window; sha256 `79470cfe6a67faf577ce29a13aff0feabcd6f67a66d8ac3622dada8a2d6e9ba2`. |
| WFE2 gate | Complete | `the-workflow-engine-v2/scripts/gate.sh` passed after repo-local target-dir hardening. |
| LEV3 gate | Complete | `advanced-tool-chaining-area/loop-engine-v3/scripts/gate.sh` passed after repo-local target-dir hardening. |
| Loom policy check | Complete | `just loom-policy-check` passed. |
| Factory security scan for `habitat-zellij` | Complete | `just factory-security-json habitat-zellij` returned zero findings after the README false-positive fix. |
| Factory proof seal | Complete | `just factory-proof-seal` passed with final hash `191c39ffccbdfec09023728376f91e6af7266c3b89a4a9f581a8676a96facf2b`. |

## Operator-Gated Or Production Tasks

These tasks are intentionally not auto-completed by this repository. They cross
the boundary from source distribution into live production state.

| Task | Status | Evidence | Required next condition |
| --- | --- | --- | --- |
| Production readiness | Complete read-only | Workspace receipt `receipts/production-readiness/factory-production-readiness-20260626T085100575771Z.json` reports `ready_for_explicit_approval` with `blockers=[]`; zero-touch verifier, production status, security, wiring, substrate, rollback readiness, and production arm grant pass; sha256 `393d3982d3f625a51a83861315ed80c41fda6d4f691290d602651a9780ad8651`. | Explicit approval is still required before any new mutating production action. |
| Kernel production arm grant | Complete read-only | Scoped grant `receipts/production-grants/zellij-orch-kernel-v012-prod-readiness-20260626T082108Z.json` validates in the latest readiness join without arming or mutating runtime state. | Refresh the grant if it expires before an action-specific readiness rejoin. |
| Persistent plugin promotion reproduction | Complete | Promotion receipt `receipts/orch-kernel-persistent-promotion-20260626T083117Z/promotion.json` reports `PROMOTED_PENDING_REPRODUCTION`; fresh reproduction proof `receipts/orch-kernel-v012-live-pipe-proof-20260626T084548Z/summary.json` reports `PASS`. | None for reproduction; new production mutations still require explicit approval. |
| Novelty promotion | Blocked | Tailwright diagnosis `receipts/tailwright-diagnosis/factory-tailwright-diagnosis-20260626T044341240587Z.json` classifies the DARK as `HONEST_DARK`; novelty promotion remains blocked by policy, not by a production-service outage. | Operator policy decides whether novelty promotion remains blocked; production-service readiness no longer treats this as a service outage. |
| Rollback execution | Blocked by design | Latest dry-run receipt `receipts/rollback-drill/factory-rollback-drill-20260625T234502039802Z.json` reports `dry_run_ready`; execution is explicitly operator-gated. | Explicit rollback approval and matching rollback target. |
| Service restart, binary restore, lease claim, production soak | Blocked by design | [Security](SECURITY.md) states these are not side effects of clone/build/test. | Explicit operator approval and fresh production readiness proof. |

## Current Recommendation

Treat v0.1.3 as source-distribution complete and read-only
`ready_for_explicit_approval` (v0.1.2 was sealed at score 90; v0.1.3 adds the
perception/governance organs + witness panel on top, 1134 host tests).
The next highest-leverage work is outside this repo:

1. Preserve the above-90 boundary until cold-start/restart reproduction and a
   substrate-emitted score above 90 are independently sealed.
2. Keep Tailwright novelty promotion blocked unless operator policy deliberately
   waives the `HONEST_DARK` state.
3. Re-run the read-only production readiness verifier with a fresh scoped grant
   if the current grant expires before a specific approved production action.
4. Only then decide whether to run an explicit promotion, rollback, restart, or
   production soak.
