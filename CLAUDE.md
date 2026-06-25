# Habitat Zellij Plugin — Claude Code Project Context

> **Back to:** [workspace CLAUDE.md](../CLAUDE.md) · [CLAUDE.local.md](CLAUDE.local.md) · [README](README.md)
>
> **Obsidian:** [[Habitat Zellij Plugin — Project Map]]
> **Hardening Plan:** [synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md](../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md)

---

## What this is

WASM dashboard plugin for Zellij — renders live ULTRAPLATE habitat telemetry (ORAC, PV2, SYNTHEX, 11 services) in a terminal pane. 4 Rust crates, 7 modules, 2,300 LOC.

## Architecture

```
habitat-zellij/
├── crates/
│   ├── habitat-core/        # HabitatModule trait, events, render primitives, response structs
│   ├── habitat-modules/      # 7 dashboard modules (fleet, health, coherence, events, cmd, na, timer)
│   ├── habitat-bridge-client/# Polls services via run_command(curl), dispatches BridgeData events
│   └── habitat-plugin/       # ZellijPlugin impl — wires modules + bridge, handles Timer/Key/Pipe
├── layouts/                  # KDL layout file
├── build.sh                  # Build + deploy to ~/.config/zellij/plugins/habitat-plugin.wasm
└── CLAUDE.md                 # This file
```

## Modules

| Module | File | Data Sources | Keybinds |
|--------|------|-------------|----------|
| `fleet_view` | `fleet_view.rs` | ORAC /health, PV2 /health | — |
| `bridge_health` | `bridge_health.rs` | ORAC /bridges, /thermal, 11× /health | — |
| `coherence_gauge` | `coherence_gauge.rs` | PV2 /field, ORAC /coupling, /hebbian | `c` toggle detail |
| `event_feed` | `event_feed.rs` | ORAC /field, PV2 /bus/events | `j`/`k` scroll, `g` top |
| `cmd_pipe` | `cmd_pipe.rs` | Zellij pipe commands | — |
| `na_panel` | `na_panel.rs` | ORAC /health | — |
| `session_timer` | `session_timer.rs` | Tick counter | — |
| `fiber_cockpit` | `fiber_cockpit.rs` | `command_sources()` self-poll → `bin/fiber-cockpit-snapshot` (tag `fiber_snapshot`); pipe `fiber-data` fallback | `j`/`k` select, `l`/Enter expand, `h` back, `g` top |
| `campaign_attention` | `campaign_attention.rs` | shared `fiber_snapshot` `BridgeData` (one feeder, two witnesses); pipe `fiber-data` fallback | `a` ack; pipes `attention-ack`/`-watch`/`-unwatch`/`-mine` |
| `sphere_warden` | `sphere_warden.rs` | `command_sources()` self-poll → `bin/zj-sphere-warden` (tag `sphere_warden`, read-only) | — (observe-only sensor) |

Global: `r` = force refresh, `q`/`Esc` = close plugin

> **Core trait `command_sources()` (D11, S1007594 — Luke-approved).** Additive default-`Vec::new()`
> method on `HabitatModule` (`habitat-core/src/module.rs`) + `CommandSource` struct; the bridge
> client schedules raw-argv local commands beside curl `DataSource`s (one stagger/interval loop,
> `register_command_sources`) and routes results through the same `BridgeData{tag}` path. The 7
> pre-existing modules are untouched (default opt-out). This is the substrate for the three D11
> witnesses' self-poll — `argv[0]` MUST be absolute (host execs directly, no shell).

> **`fiber_cockpit` (D11, S1007594)** — the agentic-factory coordination WITNESS. Floating-only
> instance (`-c "modules=fiber_cockpit"`); NOT in any layout/`load_plugins` while the Class C 9-tab
> crash is open (autostart is a Luke gate). Pipe-fed (no curl `DataSource`, no core-trait change);
> launch + feed via `bin/fiber-cockpit`. Boundary: zero writes — mechanically grep-gated. Plan:
> `ai_docs/plugin-plans/FIBER_COCKPIT_PLUGIN_PLAN_S1007594.md`. The `command_sources()` self-poll
> upgrade is now LIVE (Luke-approved S1007594) — the cockpit self-feeds; the pipe path remains a
> manual fallback.

> **`campaign_attention` (D11, S1007594)** — the agentic-factory AWARENESS layer (ambient alerts:
> fiber Δ, lease near-expiry, arming flip). Shares the `fiber-data` feed with `fiber_cockpit` (one
> feeder, two witnesses); `bin/fiber-cockpit` launches both in one floating pane by default
> (`FIBER_MODULES` overrides). Quiet→NEW digest state machine, `a`/`attention-ack` to acknowledge.
> Pipe-fed (no curl `DataSource`, no core-trait change). Boundary: zero writes — grep-gated. Plan:
> `ai_docs/plugin-plans/CAMPAIGN_ATTENTION_PLUGIN_PLAN_S1007594.md`. Self-poll now LIVE via the
> shared `fiber_snapshot` `BridgeData` feed; pipe path remains a manual fallback.

> **`sphere_warden` (D11, S1007594)** — the agentic-factory SENSE organ. Self-polls
> `bin/zj-sphere-warden` (read-only) and renders the live pane↔PV2-sphere coverage gap (the D7
> field-under-population diagnosis). **OBSERVE-ONLY**: it never registers a sphere — auto-registration
> is deferred pending Luke ratifying the sphere-id convention (live spheres are `domain:session:pane`;
> Zellij exposes only `terminal_N`) and anti-burst discipline (pswarm SIGABRT scar). The arming key
> `warden.enabled` is read + surfaced but the helper issues no `register`. Boundary: sensor reads
> only — grep-gated (no `register`/`deregister`/write). Plan:
> `ai_docs/plugin-plans/SPHERE_WARDEN_PLUGIN_PLAN_S1007594.md`. Launch in the witness pane via
> `FIBER_MODULES=fiber_cockpit,campaign_attention,sphere_warden bin/fiber-cockpit`.

## Build & Deploy

```bash
# Build WASM + deploy to Zellij plugins dir
./build.sh

# Manual build
CARGO_TARGET_DIR=/tmp/habitat-zellij-target cargo build \
  --target wasm32-wasip1 --release -p habitat-plugin

# Test (host target — core, modules, bridge-client only, NOT plugin crate)
cargo test --lib -p habitat-core -p habitat-modules -p habitat-bridge-client
```

## Quality Gate

```bash
cargo check -p habitat-core -p habitat-modules -p habitat-bridge-client && \
cargo clippy -p habitat-core -p habitat-modules -p habitat-bridge-client -- -D warnings && \
cargo test --lib -p habitat-core -p habitat-modules -p habitat-bridge-client && \
./build.sh
```

## Key Constraints

- `habitat-plugin` depends on `zellij_tile` (wasm32-wasip1 only) — cannot run `cargo test` on it natively
- Tests live in `habitat-core`, `habitat-modules`, `habitat-bridge-client` — these must NOT import `zellij_tile`
- Bridge client polls via `run_command(curl ...)` — the only WASM-safe way to do HTTP
- Plugin binary: `~/.config/zellij/plugins/habitat-plugin.wasm` (~1.2 MB)
- Cargo target dir: `/tmp/habitat-zellij-target` (avoids workspace target pollution)

## Current Status

- **v0.1.0** — working, deployed, rendering live data in Orchestrator tab (bottom-right, 25% pane)
- **Default surface (S1007736, 2026-06-15):** `modules=` default is now
  `fleet_view,bridge_health,fiber_cockpit,campaign_attention,sphere_warden` (was
  `fleet_view,bridge_health`) — the main dashboard surfaces the **agentic-factory E2E
  metrics by default** (hopf fibers/campaigns, ambient lease/arm alerts, live pane↔PV2
  sphere coverage). The three D11 witnesses are read-only self-pollers (absolute-path
  host helpers `bin/fiber-cockpit-snapshot` @5s, `bin/zj-sphere-warden` @30s). A layout
  may still override `modules` to trim. wasm sha `22f33e67…`; plugin crate now
  **clippy-clean** (cleared latent `useless_format` + `manual_is_multiple_of`).
- **340 host tests** passing (24 core + 74 bridge-client + 242 modules), zero clippy warnings — hardening-plan P2 complete
- **Known issues:** silent schema drift (all `#[serde(default)]`), polling overhead (~20 curl/cycle), 7 orphan floating instances, cmd_pipe security unaudited
- **Plan:** two-arc hardening + participation, ~18h total — see [hardening plan](../synthex-v2/ai_docs/HABITAT_ZELLIJ_PLUGIN_HARDENING_PLAN.md)

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


---

## Session Insights Doctrine (S1007542 /insights, 2026-06-10 — propagated from root CLAUDE.md)

- **Verification / read-back:** after any file append or write to memory/charter/session-state files, ALWAYS read back the file to verify the write persisted — never trust echo/append success messages.
- **Verification / capabilities:** before claiming a CLI flag, path, or capability does not exist, verify against the actual binary/source — not just `--help` output. Self-correct with live evidence.
- **Output style:** keep responses concise and chunk large outputs; save long deliverables to files rather than dumping them inline (output-token truncation is the top transcript-loss cause).
- **Scope discipline:** stay scoped to the directory/service named in the task; do not drift into unrelated services or issues. Confirm before touching anything outside the stated target.
- **Environment quirk:** use `/usr/bin/grep` and `/usr/bin/rg` directly rather than shell aliases, which produce corrupted/mangled output in this environment.
