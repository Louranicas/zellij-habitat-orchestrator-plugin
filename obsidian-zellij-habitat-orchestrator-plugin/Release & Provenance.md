# Release & Provenance

> Back to: [[MOC]] · in-repo [docs/RELEASE](../docs/RELEASE.md)

## v0.1.3 identity (current)

| Field | Value |
|---|---|
| Version | `0.1.3` |
| Tag | `v0.1.3` |
| Release commit | `831182e` — *"release: v0.1.3 — Ultimate Orchestrator perception/governance organs + witness panel (S1008937)"* |
| Synced HEAD (both remotes) | `834625f` — *"docs(README): cross-reference to v0.1.3 codebase"* |
| WASM sha | `c5b9cce69d1a39525efbc0ee73d2c34549b1e2cc4de7da6d02f1cf42c44f9789` |
| Reproduction | from-zero `wasm32-wasip1` release build (35.2s) byte-identical to deployed wasm — verified S1009130 (2026-06-30) |
| License | MIT OR Apache-2.0 |
| Zellij API compat | `0.43.x` plugin API |
| Rust target | `wasm32-wasip1` |

**v0.1.3 adds** (on top of the v0.1.2 dashboard + sidecar): the perception organ
(`orchestrator-perceive` → `perceive.snapshot`), the Delegation-Capacity Governor
(`dcg-admit`: 4-guard admission + saga compensation + `width = min(semaphore,
model-tier, budget, antichain)`), the read-only `orchestrator_witness` dashboard
panel, and the `orch-kernelctl --read-only` non-mutating superset. 7 crates ·
**1134 host tests** · `--all-targets` pedantic-clean · `forbid(unsafe_code)`.

### v0.1.2 identity (prior release)

| Field | Value |
|---|---|
| Version | `0.1.2` |
| Tag | `v0.1.2` |
| Release commit | `2a32442d51d20f262b58f993bd2c1cddd2acdcf1` |
| Seal commit (readiness receipts) | `ecfbee3` — *"docs: seal v0.1.2 readiness receipts"* |
| WASM sha | `4dcd8c60eede6545ab2c22a4fbf5ec6c6063f9cf662aae568642a8d716db6bc7` |
| License | MIT OR Apache-2.0 |
| Zellij API compat | `0.43.x` plugin API |
| Rust target | `wasm32-wasip1` |

## Remotes (standalone-push ONLY)

- GitHub: <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin>
- GitLab: <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin>

> ⚠️ **Standalone-only push discipline.** This repo lives *inside* the workspace
> tree but has its own remotes. Never commit it via the superproject; push to its
> own GitHub + GitLab remotes only. (Scar rule `feedback_morph_ir_engine_standalone_only`.)

## Readiness status (2026-06-26, S1008736)

- Promotion receipt: `PROMOTED_PENDING_REPRODUCTION`.
- Fresh reproduction proof: `PASS` (valid `NACK_USE_SIDECAR_SUBMIT`, invalid
  `NACK_SCHEMA_INVALID`).
- Zero-touch verify: `PASS`, 12/12 gates, score 90 cap 90.
- Production readiness: `ready_for_explicit_approval`, `blockers=[]`.
- Tailwright novelty promotion: `HONEST_DARK` policy-blocked.
- Framework: `ai_docs/ZELLIJ_ORCHESTRATOR_KERNEL_DEPLOYMENT_FRAMEWORK_S1008736.md`.

## Release status

Release-ready **as a source distribution**. Production arming, promotion, service
restart, rollback execution, and long-running soak remain **operator-gated**
workflows — not implicit side effects of cloning or building.

## What ships in v0.1.3

- 12 dashboard modules incl. `orchestrator_witness` ([[Dashboard Modules]])
- 4 Zellij layouts (full-fleet, compact, minimal, factory-witness), ops/proof scripts ([[Command Surface]])
- Perception organ `orchestrator-perceive` + Delegation-Capacity Governor `dcg-admit`
- Sidecar: `orch-kernelctl` (+ `--read-only`, `snapshot-v2`, `latest_perceive`),
  `orch-kerneld`, durable event log, replay, verify-chain, idempotency, policy hash
  checks ([[Orchestrator Kernel Sidecar — Durable Admission Engine]])
- 1134 Rust host tests · `forbid(unsafe_code)` · pedantic-clean

## See also

- [[Task Status & Roadmap]] — what's next beyond v0.1.3
- [[Diagnostics]] — the gates that gate a release
