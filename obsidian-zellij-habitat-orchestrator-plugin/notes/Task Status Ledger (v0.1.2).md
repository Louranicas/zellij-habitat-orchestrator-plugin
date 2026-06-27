# Task Status Ledger (v0.1.2)

> Back to: [[MOC]] · [[Task Status & Roadmap]] · [[Release & Provenance]]
> Source: `docs/TASK_STATUS.md` (GitHub-canonical) · dated 2026-06-26

Authoritative reconciliation of the historical PLAN.md task list with the
current standalone repo state. Updated with each release. See
[[Task Status & Roadmap]] for the high-level P0–P6 arc view.

---

## Historical Plan Reconciliation

| Plan item | Status | Evidence | Remaining |
|---|---|---|---|
| Task 1: `habitat-plugin` as successor to `habitat-nexus` | **Complete** | PLAN.md + standalone repo | None |
| Task 2: launcher keybind | **Verified external** | `~/.config/zellij/config.kdl` binds `Alt Shift h` (not `Alt h` — taken by `MoveFocusOrTab "left"`) | Do not overwrite `Alt h` without explicit operator approval |
| Task 3: 50+ meaningful tests | **Complete** | `cargo test --workspace` → 365 tests | None |
| Task 4: plugin/ORAC ownership boundary | **Complete** | Architecture.md + Security.md separate the four planes | None |
| Task 5: shared snapshot/dispatch path | **Partially complete** | `SnapshotClient` exists + fan-out tests; `cmd_pipe` exposes `snapshot`/`query`/`coherence`/`status` | ORAC-side TTL/auth/version-header decisions remain upstream service work |

---

## Release Repository Tasks

| Task | Status |
|---|---|
| Create standalone repo | Complete — `/home/louranicas/claude-code-workspace/zellij-habitat-orchestrator-plugin` |
| GitHub remote | Complete — `https://github.com/Louranicas/zellij-habitat-orchestrator-plugin` |
| GitLab remote | Complete — `https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin` |
| Tag v0.1.2 | Complete — `v0.1.2` @ commit `2a32442d51d20f262b58f993bd2c1cddd2acdcf1` |
| Deep-Diff-Forge-style docs | Complete — README, Architecture, Operations, Testing, Security, Release |
| Bidirectional docs links | Complete — every `docs/` file links back to README + Docs index |

---

## Verification Gate Results

| Gate | Status | Evidence |
|---|---|---|
| Markdown link integrity | PASS | `MARKDOWN_LINKS_OK` |
| `cargo fmt --all --check` | PASS | clean |
| `cargo check --workspace` | PASS | clean |
| `cargo test --workspace` | PASS | **365 tests** |
| Deep-Diff-Forge review | PASS | 7 changed files, 549 additions, 110 deletions, `semantic_fallbacks=0` |
| Governance cap verifier | PASS | `receipts/score-cap-verifier/…json` → `verdict=pass, failed_checks=[]`, lifts cap 82→90 |
| Zero-touch verifier | PASS | `receipts/orch-kernel-v012-zero-touch-verify-20260626T085030Z/summary.json` → `PASS, score=90 cap=90, sha ca7cee84…` |
| Pipe terminality (Mode A) | PASS | `receipts/orch-kernel-v012-live-pipe-proof-20260626T084548Z/summary.json` → `PASS, sha 79470cfe…` |
| WFE2 gate | PASS | `the-workflow-engine-v2/scripts/gate.sh` |
| LEV3 gate | PASS | `advanced-tool-chaining-area/loop-engine-v3/scripts/gate.sh` |
| Loom policy check | PASS | `just loom-policy-check` |
| Factory security scan | PASS | `just factory-security-json habitat-zellij` → zero findings |
| Factory proof seal | PASS | `just factory-proof-seal` → hash `191c39ff…` |

---

## Operator-Gated / Production Tasks

| Task | Status | Condition to unblock |
|---|---|---|
| Production readiness | **Complete read-only** | `receipts/production-readiness/…json` → `ready_for_explicit_approval, blockers=[]` | Explicit approval still required before any new mutating action |
| Kernel production arm grant | **Complete read-only** | `receipts/production-grants/zellij-orch-kernel-v012-prod-readiness-20260626T082108Z.json` validates in latest readiness join | Refresh grant if it expires before action-specific rejoin |
| Persistent plugin promotion reproduction | **Complete** | Promotion receipt → `PROMOTED_PENDING_REPRODUCTION`; live pipe proof → `PASS` | None for reproduction; new mutations still require explicit approval |
| Novelty promotion | **Blocked** | Tailwright diagnosis `HONEST_DARK` — policy blocks, not service outage | Operator policy decision required |
| Rollback execution | **Blocked by design** | Dry-run receipt → `dry_run_ready` | Explicit rollback approval + matching rollback target |
| Service restart / binary restore / lease claim / production soak | **Blocked by design** | Security.md: not a side effect of clone/build/test | Explicit operator approval + fresh production readiness proof |

---

## Current Recommendation (from docs/TASK_STATUS.md)

> Treat v0.1.2 as **source-distribution complete** and
> **read-only `ready_for_explicit_approval` at score 90**.
>
> Next highest-leverage work:
> 1. Preserve the above-90 boundary until cold-start/restart reproduction and a substrate-emitted score above 90 are independently sealed.
> 2. Keep Tailwright novelty promotion blocked unless operator policy deliberately waives the `HONEST_DARK` state.
> 3. Re-run read-only production readiness verifier with a fresh scoped grant if the current grant expires before a specific approved production action.
> 4. Only then decide whether to run explicit promotion, rollback, restart, or production soak.

---

## Wave history (for release provenance)

| Wave / Session | Change |
|---|---|
| Wave-16 S1005032 | Added WFE to 14-service grid (`bridge_health.rs` SERVICES + PROBE_PATHS) |
| S1007594 | D11 witnesses (`fiber_cockpit`, `campaign_attention`, `sphere_warden`) + `command_sources()` trait extension |
| S1007736 | Default `modules=` surface updated to include D11 witnesses |
| S1008620 | Zellij PTY double-panic fix deployed (`a4c68619`); bidi-wiring P2 LIVE |
| S1008736 | Orchestrator kernel deployment framework, v0.1.2 seal, score=90 |
| S1008798 | Dedicated Obsidian vault created (this vault) |

---

## See also

- [[Task Status & Roadmap]] — P0–P6 arc summary
- [[notes/Score & Fitness Framework]] — how the score=90 was computed
- [[Release & Provenance]] — v0.1.2 identity and publish steps
