# Habitat Zellij Plugin — Local Session State

> **Back to:** [CLAUDE.md](CLAUDE.md) · [workspace CLAUDE.md](../CLAUDE.md) · [README](README.md)
> **Obsidian vault:** [dedicated vault MOC](obsidian-zellij-habitat-orchestrator-plugin/MOC.md) (in-repo dedicated vault, created S1008798)
> **Hardening Plan:** [synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md](../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md)
> **🟢 Wave-16 plugin-grid expansion 13→14 — S1005032, 2026-05-25 — LIVE.** `bridge_health.rs` SERVICES (line 38) + PROBE_PATHS (line 210) gained `(8142, "WFE")` + `(8142, "/health")` for the new workflow-trace `wf-daemon` habitat service. Plugin grid now renders `V3 Nerve TL SX V8 VMS POVM RM PV2 ORAC Inj WFE ME PSw` (14 services). `habitat-modules` test suite 91/91 passing post-edit; `cargo clippy -- -D warnings` clean. `habitat-plugin.wasm` rebuilt via `./build.sh` (1.2M); deployed to `~/.config/zellij/plugins/habitat-plugin.wasm` with "Hot-reloaded in active session"; verified WFE string baked into wasm + live screenshot showed `ALL UP 14/14 (14 probed)` with green `WFE` indicator. Port story: first attempt 8141 was wrong (HABITAT-CONDUCTOR's reserved port, down via `auto_start=false`); re-ported to 8142 (verified free across 4 surfaces). Project-side anchor: [`../the-workflow-engine/ai_docs/WAVE_16_WF_DAEMON_DESIGN_S1005032.md`](../the-workflow-engine/ai_docs/WAVE_16_WF_DAEMON_DESIGN_S1005032.md) + vault [[Wave-16 — wf-daemon Habitat Service Shape S1005032]]. stcortex `workflow_trace_completion_s1004115` mem **19192**; injection.db `causal_chain` id **135**.

> **🟢 v0.1.3 SHIPPED + LIVE (S1008937 · doc-reconciled S1009130 / 2026-06-30).** Tag `v0.1.3`,
> HEAD `834625f`, both remotes synced, wasm sha `c5b9cce6…` (from-zero reproduced). Live zellij
> config repointed v0.1.2→v0.1.3 (`config.kdl` `Alt Shift h` keybind + `synth-orchestrator.kdl` ×2).
> **7 crates · 12 dashboard modules · 1134 host tests / 0 failed · `forbid(unsafe_code)` · pedantic-clean.**
> v0.1.3 adds the perception organ `orchestrator-perceive`, the delegation-capacity governor
> `dcg-admit`, the `orchestrator_witness` panel, and `orch-kernelctl --read-only`. The two-arc
> hardening/participation plan below (v0.1.0→v0.1.2) is **historical** — that arc is complete.

---

> **Last saved:** 2026-06-30 (S1009130 — v0.1.3 documentation reconciliation, verified 1134 tests)
> **Current state:** v0.1.3 deployed + live · 1134 host tests passing / 0 failed · 7 crates · 12 modules ·
>   perception + delegation-governance organs + witness panel · `--all-targets` pedantic-clean
> **Next action:** live hot-reload of the running FLEET pane in session `considerate-pheasant` (operator);
>   optional commit+push of doc updates to standalone remotes.

> _Below: the v0.1.0→v0.1.2 hardening/participation ledger — preserved as historical record._

## Plan Status

| Phase | Status | Description |
|-------|--------|-------------|
| P0: Backup + security | ✅ COMPLETE (2026-04-22) | v0.1.0.wasm saved (SHA 2be5217c…); cmd_pipe audit — cmd-injection NOT present (firewalled by arch); ANSI injection MEDIUM → Phase 4 |
| P1: Constraints | ✅ COMPLETE (2026-04-22) | WASM boundary PASS (host cargo check 3.47s); floating instances NOT reproducible (downgraded to observation); 6/18 top-level response structs dead (33%) |
| P2: Tests | ✅ COMPLETE (2026-04-24) | **163 tests** (20 bridge-client + 52 core + 91 modules) · 8 dead structs deleted · **51 pre-existing pedantic errors cleared** · 4 crate-level `#[allow]` with named lint+justification+removal date · `ConfigWarning` enum + `from_btree(BTreeMap) → (Self, Vec<ConfigWarning>)` · URL validation + poll-interval clamp [1.0, 300.0] · Tier-1 fixtures (6 JSON files) · Tier-2 live-snapshot script (15/15 endpoints) · per-module count: fleet_view 17, coherence_gauge 16, bridge_health 15, session_timer 14, na_panel 12, event_feed 11, cmd_pipe 6 · WASM release build clean 7.08s · all 4 charter gate stages green |
| P3: Drift + polling | NOT STARTED | Phase 2 schema drift (`LiveDataCheck` trait + `has_live_data()`) + Phase 3 URL dedup + `[STALE Xs]` indicator |
| P4: Modules + docs | NOT STARTED | cmd_pipe ANSI sanitizer (P0 G9 residual MEDIUM) + event_feed HashSet dedup + bridge_health content validation |
| P5: Self-model | NOT STARTED | NA-Z1 PluginHealth + NA-Z2 sphere registration |
| P6: Participation | NOT STARTED | NA-Z3 temporal depth + NA-Z4 module autonomy |

## P0+P1 Audit

Full report: [`ai_docs/P0_P1_AUDIT_2026-04-22.md`](ai_docs/P0_P1_AUDIT_2026-04-22.md) · covers cmd_pipe flow trace, WASM boundary evidence, floating-instance inventory, dead-struct table.

## Bidirectional Anchors

- Hardening plan: [`synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md`](../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md)
- Obsidian vault (dedicated, in-repo): [MOC](obsidian-zellij-habitat-orchestrator-plugin/MOC.md) — 12 notes, created S1008798
- ULTRAPLATE Master Index: [[ULTRAPLATE Master Index]] (Tier 2 entry)
- Workspace CLAUDE.md: [`~/claude-code-workspace/CLAUDE.md`](../CLAUDE.md) (Orchestrator layout section)
- Synthex-v2 CLAUDE.local.md: [`synthex-v2/CLAUDE.local.md`](../synthex-v2/CLAUDE.local.md) (startup reminder)

## Bootstrap (fresh context)

```bash
cd ~/claude-code-workspace/habitat-zellij
cat CLAUDE.md           # architecture + constraints
cat CLAUDE.local.md     # this file — plan status + anchors
# Then read the plan if starting hardening work:
cat ../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md
```

<!-- INSIGHTS-S1000146-WORKFLOW-ADDITIONS -->

---

## Concurrent File Editing

When editing shared markdown files (especially in fleet/multi-pane scenarios), prefer atomic `bash` append (`cat >> file` or `echo '...' >> file`) over the Edit tool. Other panes may be writing concurrently and Edit will fail on stale content. Only use Edit for files you have exclusive access to.

## Verification Discipline

- Before writing new helper methods (e.g., `sweep`, `cleanup`, `compact`, `purge`), grep the codebase for existing equivalents and surface what exists first; ask whether to extend vs. create new.
- Before fixing reported findings, FP-verify against source first — many cross-agent findings turn out to be already fixed.
- After applying fixes, always run the full quality gate (`cargo test`, `cargo clippy -- -D warnings`, `cargo check`) before declaring complete. Report exact test counts (e.g., `1830/1830 passing, zero warnings`).

## Avoid Over-Engineering

When recommending architectural changes, start with the simplest integration (blackboard pattern, additive wiring) before suggesting major refactors of core state structs (e.g., `OracState`). Ask before proposing changes that touch >5 files or core state types.

## Quality Gates

- Always run the full test suite and quality gates (clippy, fmt, lint) after multi-file changes before declaring complete.
- Report exact test counts in completion summaries.
- Minimum 50+ tests per module unless otherwise specified.
- After any toolchain upgrade (rustc, clippy), expect new lints; run the full gate script and fix all clippy errors before declaring done. Verify PATH in both `.bashrc` and gate scripts points to the upgraded toolchain.

## Documentation Persistence

- After completing significant work, save findings/schematics to the Obsidian vault with bidirectional wikilinks.
- Update relevant `INDEX.md` files when adding notes.
- Verify all wikilinks resolve before considering documentation complete.

## Git Workflow

- After completing hardening or feature work, commit and push to BOTH GitHub and GitLab remotes unless told otherwise.
- Include test pass counts and quality gate status in commit messages.

## Recurring Loops & Cron

- When a recurring/cron loop's work is complete (convergence, G7, end-of-life signal detected), proactively recommend `CronDelete` or cancellation.
- Recognize duplicate/stale prompts from cron firings and skip rather than re-executing completed work.
