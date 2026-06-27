# Release & Provenance

> Back to: [[MOC]] · in-repo [docs/RELEASE](../docs/RELEASE.md)

## v0.1.2 identity

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

## What ships in v0.1.2

- 11 dashboard modules ([[Dashboard Modules]])
- 4 Zellij layouts, 20 ops/proof scripts ([[Command Surface]])
- Sidecar: `orch-kernelctl`, `orch-kerneld`, durable event log, replay,
  verify-chain, idempotency, policy hash checks ([[Orchestrator Kernel Sidecar — Durable Admission Engine]])
- 365 Rust tests

## See also

- [[Task Status & Roadmap]] — what's next beyond v0.1.2
- [[Diagnostics]] — the gates that gate a release
