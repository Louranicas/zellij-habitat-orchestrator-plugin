# Durable Lessons & Design Decisions

> Back to: [[MOC]] · source `NOTES.md` · `PLAN.md`

Verified invariants and design decisions that have survived review. These are
not aspirations — they are constraints the code was written to satisfy.

---

## L1 — Admission belongs in the sidecar, not the plugin

**Source:** `NOTES.md` (2026-06-25) + `kernel_pipe.rs` implementation.

Durable task admission belongs in `orchestrator-kernel-sidecar`. Zellij pipe
handling must not claim durability or append through an async bridge path.

Evidence: `EventLog::submit` in `crates/orchestrator-kernel-sidecar/src/lib.rs`
owns `SubmitRequest`/`SubmitResponse` and idempotency. `HabitatDashboard::
handle_kernel_pipe` in `crates/habitat-plugin/src/main.rs` returns
`NACK_USE_SIDECAR_SUBMIT` for all valid kernel pipe JSON — it forwards, it
never asserts durability itself.

**Never add a direct write path from the plugin to any durable store. If you
want durability, call `orch-kernelctl submit` and wait for `ACK_DURABLE`.**

---

## L2 — Config failure must never crash the plugin

**Source:** `habitat-core/src/config.rs` `ConfigWarning` pattern; Hardening
Plan §WS-0 P2; P2 completion (2026-04-24).

WASM unrecoverable panics are invisible to the operator — the plugin vanishes.
`ModuleConfig::from_btree` returns `(Self, Vec<ConfigWarning>)`, not
`Result`. Every field has a documented default. Warnings are logged at init
without aborting. Callers *should* surface warnings to the operator but the
plugin *will* boot regardless.

**Never add a `.expect()` / `.unwrap()` to the config parse path.**

---

## L3 — Observation is cheap; action crosses the sidecar boundary

**Source:** PLAN.md first principle + `Security & Admission Boundary`.

The plugin may poll, render, serialize pane state, and forward pipe messages.
It does not invent durability. `ACK_DURABLE` requires: event append + hash
update + idempotency resolution + policy-bound warrant. All four happen inside
the sidecar, and the sidecar's receipt is the proof.

This governs the three D11 witnesses especially: `sphere_warden` will never
auto-register a sphere until Luke ratifies the convention + anti-burst
discipline — the cost of getting it wrong (pswarm SIGABRT, uncontrolled
registration burst) outweighs the observation gap.

---

## L4 — The stagger exists to prevent the T=0 subprocess storm

**Source:** `BridgeClient::poll_due` implementation; [[notes/Bridge Client & Polling Engine]]; CPU-saturation RCA (S1008517).

Without stagger all endpoints fire simultaneously on the first Timer tick.
At 20 curl calls + 3 D11 witness processes across multiple plugin instances
this saturated the host (load ~3500 on 16 cores). `stagger_complete` boots
one endpoint per tick until all have had their first poll, then switches to
interval scheduling.

**Do not add new `DataSource`s or `CommandSource`s with sub-30s cadence
without checking the total concurrent subprocess count first.**

---

## L5 — `habitat-plugin` supersedes `habitat-nexus`

**Source:** PLAN.md §1; P0+P1 audit; [[notes/Plugin in Habitat Context (Factory Map)]].

`habitat-nexus` remains on disk as a rollback anchor only. The modular
trait-based architecture, isolated testability (host crates with zero
`zellij_tile` import), explicit NA compliance, and `run_command(curl)` 
transport (vs `web_request`) are the reasons for the succession.

`habitat-nexus-visualizer` (Obsidian/TS, always-on vault writer) is a
*companion*, not a competitor — it owns a different sink (vault notes) and
shares the same field. Do not merge their concerns.

---

## D1 — `cmd_pipe` pipe surface is 3 commands, not 6

The original `habitat-nexus` exposed 6 pipe commands. `habitat-plugin`
deliberately narrows to 3 (`snapshot` / `query` / `status`) with a 60s
cooldown on `snapshot`. Rate-limiting is enforced at the `cmd_pipe` module
level. Any expansion must go through an explicit audit (see
[[notes/P0 P1 Security Audit (2026-04-22)]] §P0.G9).

---

## D2 — WASM target locks all tests below the plugin crate

`habitat-plugin` imports `zellij_tile` → only compiles to `wasm32-wasip1` →
host `cargo test` cannot run it. This is a structural constraint, not a
shortcut. The 1134 host tests (v0.1.3) live in the six host crates —
`habitat-core`, `habitat-modules`, `habitat-bridge-client`,
`orchestrator-kernel-sidecar`, `orchestrator-perceive`, `dcg-admit` —
all of which must have **zero imports of `zellij_tile`**.

---

## D3 — Dead structs are deleted, not `#[allow]`'d

P1.G5 found 6 top-level dead response structs (33% of `responses.rs`).
Decision: delete before writing tests, not suppress. Reason: silent
`#[serde(default)]` on a dead struct silently absorbs real schema drift and
gives false test confidence. Fewer live structs means fewer surfaces for
schema-drift to hide on.
