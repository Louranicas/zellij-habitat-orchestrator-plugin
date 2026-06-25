---
type: plan-of-record
title: habitat-zellij Plan of Record
tags: [plan, plugin, zellij, habitat]
date: 2026-04-22
session: 109
status: authoritative
supersedes: HABITAT_NEXUS_PLUGIN_SPEC_v2_1.md
---

# habitat-zellij — Plan of Record

> Back to: [README](README.md) · [CLAUDE.md](CLAUDE.md) · [CLAUDE.local.md](CLAUDE.local.md) · workspace [CLAUDE.md](../CLAUDE.md)
>
> Companion: [Hardening Plan](../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md) · [P0+P1 Audit](ai_docs/P0_P1_AUDIT_2026-04-22.md) · habitat-test-vault [[Plugin-Deployment-Assessment]]

This document is the canonical answer to the four questions the Plugin-Deployment-Assessment (2026-04-22) flagged as blockers for activating the `habitat-plugin.wasm` artefact deployed at `~/.config/zellij/plugins/habitat-plugin.wasm` (1252116 bytes, 2026-04-21).

---

## 1. Relationship to `habitat-nexus`

**`habitat-plugin` is the successor.** `habitat-nexus` is deprecated but retained on disk until `habitat-plugin` ships smoke tests (Task 3) and the Alt h keybind (Task 2) proves the replacement in production.

| Axis | habitat-nexus (retired candidate) | habitat-plugin (this) |
|---|---|---|
| Source layout | single `main.rs` (~2641 LOC) | 4 crates, modular trait-based |
| Testability | monolithic, zero test isolation | tests live in core/modules/bridge-client (WASM-free) |
| Spec doc | `the_maintenance_engine_v2/ai_docs/HABITAT_NEXUS_PLUGIN_SPEC_v2_1.md` | this file + [Hardening Plan](../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md) |
| Generations | `// G{1..7} RALPH: COMPLETE` markers, 0 tests | 7 modules built to `HabitatModule` trait; test infrastructure forthcoming |
| Polling model | 7-arm parallel `web_request` + worker deserialization | `run_command(curl)` per `DataSource`, tagged async context, dedup planned (Phase 3 hardening) |
| Pipe API | 6-command surface, rate-limited | 3-command surface (snapshot / query / status), 60s cooldown on `snapshot` |
| Persistence | `/data/nexus_state.json`, 12-tick saves | `serialize_state` / `restore_state` on module trait, hot-reload preserves state |
| Binary size | 1148307 bytes | 1252116 bytes (newer, +9% from modular overhead) |
| NA compliance | implicit | **explicit** — S1/S4/S6/S7/S8/S9/S11/S12 encoded at module boundary |
| Status | dark-deployed, no UX surface, no keybind | dark-deployed until Task 2 wires `Alt h` |

**Decision:** The deployment assessment's G-2 (HIGH severity: "no plan doc for habitat-plugin") resolves here. `habitat-nexus` is not archived yet — it stays on disk as a rollback anchor until `habitat-plugin` proves green under the 50-test suite (Task 3).

---

## 2. Which of habitat-nexus's 7 arms does this implement?

`habitat-nexus` described "7 parallel polling arms" as a transport concept. `habitat-plugin` reshapes the surface into **7 rendered modules** that own their own data sources — polling is still parallel, but per-module, decoupled, and keybound.

| habitat-nexus arm | habitat-plugin module | Primary endpoints | Poll interval |
|---|---|---|---|
| ORAC | `fleet_view` | `:8133/health`, `:8132/health` | 5s |
| PV2 | `coherence_gauge` | `:8132/field`, `:8133/coupling`, `:8133/hebbian` | 2s |
| SYNTHEX | `bridge_health` (partial — thermal band) | `:8133/bridges`, `:8133/thermal`, Nerve `:8083/health`, 11× probes | 5s + 30s |
| NexusBus | `event_feed` | `:8133/field` emergence.recent, `:8132/bus/events` | 5s + 10s |
| POVM | *not directly polled* — surfaced via ORAC blackboard in `event_feed` | (indirect) | — |
| CcFleet | `na_panel` | `:8132/field/proposals` | 10s |
| MeV2 | `session_timer` | `:8133/session-stats`, `:8133/tokens` | 10s |

Additional module not in habitat-nexus: `cmd_pipe` — event-driven pipe command handler with rate limit (S9).

**Delta from habitat-nexus spec v2.1:**
- No outbound `ArmCommand` batch POST (nexus §Outbound) — deferred until Task 5 `/dispatch` endpoint lands.
- No in-plugin alert detectors (nexus §Alert) — ORAC already runs STDP + HebbianSaturation + DispatchLoop + ConsentCascade + CoherenceLock detectors server-side (Session 060 Ignition). Duplicating them in WASM would violate S6 (raw-weight visibility) and NA-Z1 (plugin sphere-like autonomy).
- No worker deserialization off main thread — WASM `wasm32-wasip1` has no thread spawn; instead, `BridgeClient` uses tagged async contexts via `run_command`.

---

## 3. UX intent

**Floating, keybound, terminal-native.** The plugin is a pane a developer summons on demand — it is not always-on. Always-on observation is the job of the Obsidian plugin (`habitat-nexus-visualizer`), which runs continuously inside the vault.

| Surface | Behaviour |
|---|---|
| Launcher | `Alt h` chord (Task 2) — `LaunchOrFocusPlugin` floating, 95%w × 70%h |
| Full-tab layout | `zellij --layout layouts/habitat-fleet.kdl` for a dedicated dashboard session |
| Compact mode | `layout_mode "compact"` (3 modules: fleet_view + coherence_gauge + event_feed) |
| Minimal mode | `layout_mode "minimal"` (just `coherence_gauge` as a status footer) |
| Close gesture | `q` / `Esc` — triggers `serialize_state` for hot-reload fidelity |
| Manual refresh | `r` — forces poll of all registered `DataSource` entries |
| Event feed scroll | `j` / `k` / `g` — down / up / jump-to-top |
| Coupling detail | `c` — toggles raw weight matrix visibility (default hidden per S6) |

**Split-brain gate with the Obsidian plugin:**
- habitat-plugin reads real-time field state, writes **nothing to disk** (except `serialize_state` snapshot).
- habitat-nexus-visualizer reads the same field state, writes vault notes (emergence + sphere + digest) under consent gate.
- Neither duplicates the other's work; they share the field but own different sinks. Task 5 (`/dispatch`) will let both read a single aggregated snapshot instead of polling independently.

---

## 4. Relationship to the shared ORAC snapshot

ORAC already owns `/dispatch` for the fleet dispatch queue (pending/assigned/completed tasks). The shared aggregate lives at `/snapshot` instead — same intent, clearer path. After Task 5 (SHIPPED 2026-04-22):

- `fleet_view` consumes `/snapshot.orac` + `/snapshot.pv2` (one curl instead of two).
- `bridge_health` consumes `/snapshot.thermal` + `/snapshot.nerve` + `/snapshot.services[]` (one curl instead of 13).
- `coherence_gauge` consumes `/snapshot.pv2` (one curl; the coupling + hebbian deep-dives still hit ORAC directly when the module needs them).
- `event_feed` continues to poll `:8132/bus/events` directly (event stream is additive, not suitable for point-in-time aggregation).

**Target:** 18 curls per 10s cycle → ~6 curls per 10s (~3× traffic reduction).

**Server:** `orac-sidecar/src/m3_hooks/m10_hook_server.rs::snapshot_handler` — aggregates ORAC cached field state + PV2 `/health` + SYNTHEX `/v3/thermal` + Nerve `/health` + 11 service probes. Remote sub-probes run in parallel via `tokio::join!` / `JoinSet` with 1s timeout; failing probes contribute `null` sub-objects rather than stalling the aggregate.

**Client:** `habitat-bridge-client::SnapshotClient` — registers one route per sub-object, fans the aggregate JSON out into module-tagged `BridgeData` events that look identical to a per-endpoint curl response to downstream modules. No module changes required to start consuming. Test coverage in `crates/habitat-bridge-client/src/lib.rs::snapshot_tests` (5 tests: fan-out, null-skip, missing-subkey, data-verbatim, argv-contract).

**Migration:** plugin opts in per-session. No config change default — `habitat-plugin` continues to ship with its per-endpoint `DataSource` list until a future release flips to `SnapshotClient` as the primary transport.

---

## 5. Execution order (aligned with Luke's 5-task directive, 2026-04-22)

| # | Task | This plan's section | Source of truth |
|---|---|---|---|
| 1 | Author this PLAN.md | §§1-4 | this file |
| 2 | Wire `Alt h` keybind | §3 (UX intent) | `~/.config/zellij/config.kdl` |
| 3 | 50 meaningful tests per plugin | §6 (below) | `crates/habitat-*/tests/` + habitat-nexus + habitat-obsidian |
| 4 | Fix `sphere_id: unknown` in ORAC emergence detector | (out of scope for this plan — ORAC change) | `orac-sidecar/` |
| 5 | Add ORAC `/dispatch` + teach plugins | §4 (/dispatch) | orac-sidecar + `habitat-bridge-client` |

---

## 6. Test strategy for Task 3 (50+ meaningful tests, no test-fit)

The 7-11h Hardening Plan Phase 1 calls for "~400 LOC" of tests. This plan sharpens that to a **behavioral contract** per module, so test count is emergent from behavior coverage, not invented to hit 50.

**Non-negotiables (aligned with workspace AP23 "test-fit drift"):**
- No test that passes because a mock returns what the test asserts. Mocks assert **mock invocation**, real types assert **parse + invariant**.
- No test that exercises only the happy path. Every parser test has a malformed-input counterpart.
- No test that uses `unwrap()` on the value under test (AP-guarded via `clippy::unwrap_used` deny).
- Property-based tests for pure functions (render primitives, config clamping, backoff arithmetic).

**Per-crate budget (total ≥50 per plugin):**

| Crate | Target | Focus |
|---|---:|---|
| `habitat-core/render.rs` | 10 | `truncate`, `fmt_num`, `thermal_band` branches, `cycle_indicator` phase wheel, small-terminal edge cases (1,1 / 5,30 / 100,200) |
| `habitat-core/config.rs` | 6 | URL validation, poll-interval clamping `[1.0, 300.0]`, malformed TOML, env overlay precedence, default completeness |
| `habitat-core/responses.rs` | 8 | Real-fixture JSON → struct round-trip (one per response type), `has_live_data` detector, silent-default sentinel |
| `habitat-core/events.rs` + `module.rs` | 4 | Event dispatch ordering, module trait default impls, state snapshot/restore round-trip |
| `habitat-bridge-client/lib.rs` | 8 | `handle_result` exit/stdout matrix, backoff progression, context-tag routing, stale-data flag, URL dedup |
| `habitat-modules/*` | 14 (2 per module × 7 modules) | Event → render round-trip, edge state (empty data, stale data), plus property tests where pure |
| **habitat-plugin total** | **≥50** | — |

**habitat-nexus** (separate plugin) and **habitat-nexus-visualizer** (Obsidian/TS) get their own 50 tests in Task 3 — breakdown in that task's work.

**Quality gate after every test batch:**
```bash
cargo check --target x86_64-unknown-linux-gnu -p habitat-core -p habitat-modules -p habitat-bridge-client
cargo clippy -p habitat-core -p habitat-modules -p habitat-bridge-client -- -D warnings
cargo clippy -p habitat-core -p habitat-modules -p habitat-bridge-client -- -D warnings -W clippy::pedantic
cargo test --lib -p habitat-core -p habitat-modules -p habitat-bridge-client --release
```

No test may pass alongside a clippy warning. No clippy allow-override may be added to make a test pass — if the lint fires, the code is wrong.

---

## 7. Open decisions deferred to Task 5

1. **`/dispatch` cache TTL** — 1s (tightest reasonable) vs 5s (matches longest module poll). ORAC owns the answer; likely 1s with 304 Not Modified on unchanged snapshots.
2. **`/dispatch` auth** — habitat is localhost-only for now, but `/dispatch` is the first potential cross-host surface. Deferred to ORAC's auth story (not yet defined).
3. **Schema version header** — `X-Habitat-Dispatch-Version: 1` so the plugin can reject mismatches instead of silent-defaulting per AP02.

---

## 8. Rollback plan

Task 2 is the only step with blast radius outside this workspace (it edits `~/.config/zellij/config.kdl`). The edit is:
- Additive (new `bind "Alt h"` block inside existing session mode).
- Scoped to one chord that has no current binding.
- Reversible by removing the block.

The binary at `~/.config/zellij/plugins/habitat-plugin.wasm` already has a backup alongside: `habitat-plugin-v0.1.0.wasm` (SHA `2be5217c…`, noted in P0+P1 audit). Rollback is `\cp -f habitat-plugin-v0.1.0.wasm habitat-plugin.wasm` + `zellij action start-or-reload-plugin`.

---

## 9. Bidirectional anchors

- Habitat CLAUDE.md: [workspace](../CLAUDE.md) § Zellij plugins (pending propagation)
- Habitat-test-vault: [[Plugin-Deployment-Assessment]] § G-2 (this plan closes the gap)
- Hardening plan: [ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md](../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md)
- Audit: [ai_docs/P0_P1_AUDIT_2026-04-22.md](ai_docs/P0_P1_AUDIT_2026-04-22.md)
- Successor-relation: `habitat-nexus/PROMPT.md` (retained for historical context; not the source of truth for habitat-plugin behaviour)
