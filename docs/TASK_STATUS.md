# Task Status

Back to [README](../README.md) · [Docs index](INDEX.md)

This ledger reconciles the historical [PLAN.md](../PLAN.md) task list with the
current standalone `zellij-habitat-orchestrator-plugin` repository state. It is
the release-local status source for v0.1.2 documentation, verification, and
remaining gated work.

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
| Task 2: wire launcher keybind | Verified external | `~/.config/zellij/config.kdl` binds `Alt Shift h` to `file:~/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm`; `Alt h` is already bound to `MoveFocusOrTab "left"`. | Do not overwrite `Alt h` without explicit operator approval; document `Alt Shift h` as the active launcher. |
| Task 3: ship 50+ meaningful tests for `habitat-plugin` | Complete | `cargo test --workspace` passes 365 tests in the standalone repo. | None for `habitat-plugin`; sibling plugin test plans remain separate. |
| Task 4: clarify plugin/ORAC ownership boundary | Complete | [Architecture](ARCHITECTURE.md) and [Security](SECURITY.md) separate Zellij rendering, bridge transport, sidecar durability, and operator-gated production actions. | None for this repo. |
| Task 5: add shared snapshot/dispatch path | Partially complete | `habitat-bridge-client::SnapshotClient` exists with snapshot fan-out tests; `cmd_pipe` exposes `snapshot`, `query`, `coherence`, and `status`. | ORAC-side TTL/auth/version-header decisions remain upstream service work. |

## Release Repository Tasks

| Task | Status | Evidence |
| --- | --- | --- |
| Create separate standalone repository | Complete | Local repo: `/home/louranicas/claude-code-workspace/zellij-habitat-orchestrator-plugin`. |
| Publish GitHub remote | Complete | <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin> |
| Publish GitLab remote | Complete | <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin> |
| Tag v0.1.2 | Complete | Tag `v0.1.2` points to initial release commit `2a32442d51d20f262b58f993bd2c1cddd2acdcf1`. |
| Add Deep-Diff-Forge-style documentation | Complete | [README](../README.md), [Architecture](ARCHITECTURE.md), [Operations](OPERATIONS.md), [Testing](TESTING.md), [Security](SECURITY.md), [Release](RELEASE.md). |
| Preserve bidirectional docs links | Complete | Every file in `docs/` links back to [README](../README.md) and [Docs index](INDEX.md). |

## Verification Tasks

| Gate | Status | Evidence |
| --- | --- | --- |
| Markdown link integrity | Complete | Repository-local link checker returned `MARKDOWN_LINKS_OK`. |
| Rust formatting | Complete | `cargo fmt --all --check` passed. |
| Rust compile | Complete | `cargo check --workspace` passed. |
| Rust test suite | Complete | `cargo test --workspace` passed with 365 tests. |
| Deep-Diff-Forge documentation review | Complete | Initial docs review reported 7 changed files, 549 additions, 110 deletions, and `semantic_fallbacks=0`; current task-ledger patch review reports 4 changed files, 92 additions, 1 deletion, and `semantic_fallbacks=0`. |
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
| Production readiness | Blocked | Workspace receipt `receipts/production-readiness/factory-production-readiness-20260625T231656360605Z.json` reports `blocked`; production status is degraded. | `factory-status --mode production` must be green. |
| Kernel production arm grant | Blocked | Default kernel readiness receipt `receipts/production-readiness/factory-production-readiness-20260625T231656172181Z.json` reports missing real kernel production arm grant. | Operator supplies a valid production grant for `zellij-orchestrator-kernel-v012`. |
| Promotion | Blocked | Production status remains degraded because Tailwright is `DARK` and blocks novelty promotion. | Resolve Tailwright probe/dissent and rerun production readiness. |
| Rollback execution | Blocked by design | Dry-run rollback verifier is available; execution is explicitly operator-gated. | Explicit rollback approval and matching rollback target. |
| Service restart, binary restore, lease claim, production soak | Blocked by design | [Security](SECURITY.md) states these are not side effects of clone/build/test. | Explicit operator approval and fresh production readiness proof. |

## Current Recommendation

Treat v0.1.2 as source-distribution complete and production-promotion blocked.
The next highest-leverage work is outside this repo:

1. Resolve Tailwright so `factory-status --mode production` stops reporting a
   degraded novelty-promotion surface.
2. Produce a real, scoped production arm grant for
   `zellij-orchestrator-kernel-v012`.
3. Re-run the read-only production readiness verifier.
4. Only then decide whether to run an explicit promotion, rollback, restart, or
   production soak.
