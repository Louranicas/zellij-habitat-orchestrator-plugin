# Zellij Habitat Orchestrator Plugin

Release name: `zellij-habitat-orchestrator-plugin`

Version: `0.1.2`

Modular, hot-reloadable Zellij WASM plugin for the ULTRAPLATE Habitat. Terminal-native interface to coherence, emergence, fleet state, and governance across 11 services.

> **Status (2026-04-24):** S110 P0-P2 + P3 foundations shipped. 183 tests across 3 host-target crates (habitat-core 72 · habitat-modules 91 · habitat-bridge-client 20). 51 pre-existing pedantic debts cleared. WASM release build clean in 7.08s. Added: `LiveDataCheck` trait + 13 `has_live_data()` impls, `stale_tag()` render helper, `ConfigWarning` enum with URL parse + poll-clamp validation, Tier-1 hand-crafted JSON fixtures at `crates/habitat-core/tests/fixtures/`, Tier-2 `scripts/capture-fixtures.sh` capturing 15 live endpoints.
>
> **Plan:** see `~/projects/shared-context/Comms Layer Unification Plan — 2026-04-24.md` · Obsidian: [[Comms Layer Unification Plan v3]] · [[Comms Layer Unification — Architectural Schematics]] · WS-6 habitat-wire (consumer of the new `/bus/ws`) deferred to S120+.

## Architecture

```
habitat-zellij/
├── Cargo.toml                        # workspace root
├── crates/
│   ├── habitat-core/                 # trait, events, render, config, 23 serde structs
│   ├── habitat-modules/              # 7 built-in modules
│   ├── habitat-bridge-client/        # async HTTP via run_command + context tags
│   └── habitat-plugin/               # WASM cdylib entrypoint
├── layouts/
│   ├── habitat-fleet.kdl             # full 7-module view
│   ├── habitat-compact.kdl           # 3-module slim view
│   └── habitat-minimal.kdl           # just coherence_gauge footer
└── build.sh                          # build + deploy + hot-reload
```

## Built-in Modules

| Module | Endpoints | Interval | Purpose |
|--------|-----------|----------|---------|
| `fleet_view` | ORAC `/health`, PV2 `/health` | 5s | RALPH cycle, field r, thermal, STDP contextual |
| `coherence_gauge` | PV2 `/field`, ORAC `/coupling`, ORAC `/hebbian` | 2s | ASCII bar for r, aggregated coupling (S6) |
| `bridge_health` | ORAC `/bridges`, ORAC `/thermal`, Nerve `/health`, 11 service probes | 5s + 30s | Thermal context band (S11), breaker state, service grid |
| `event_feed` | ORAC `/field` (emergence.recent), PV2 `/bus/events` | 5s + 10s | Scrollable log with confidence coloring (S4), TTL filter (S5) |
| `na_panel` | PV2 `/field/proposals` | 10s | Sovereignty · consent · governance |
| `session_timer` | ORAC `/session-stats`, ORAC `/tokens` | 10s | Uptime, tools, token budget |
| `cmd_pipe` | (internal — receives PipeMessage) | event-driven | Bidirectional command handler with rate limit (S9) |

## NA-Compliant Design

This plugin is a participant in the system it observes, not a spectator. Architectural choices:

- **S1 Observation budget:** ~18 curls per 10s cycle (~50 KB bandwidth), staggered on cold start
- **S4 Confidence indicators:** Every emergence event displayed with confidence percentage, color-coded green/yellow/red
- **S6 Coupling aggregation:** Raw weights hidden by default (press `c` to toggle debug detail). Aggregate-only display protects the learning substrate from human optimization pressure
- **S7 RALPH cycle:** Phase shown as rotating indicator `R·A·L·P·H` with current phase highlighted, never as a progress bar
- **S8 Quiet connect:** First tick polls endpoints one-per-tick over 30s instead of 11-parallel curl burst
- **S9 Rate limiting:** `cmd_pipe` enforces 60s cooldown on `snapshot` command
- **S11 Thermal context:** Temperature shown as band (NORMAL/COOL/HOT/CRITICAL) with color, never raw number alone
- **S12 STDP context:** LTP/LTD hidden at 0 spheres ("idle"), hinted at 1 ("warming up"), ratios shown at 2+

## Build

```bash
bash build.sh
```

Output: `~/.config/zellij/plugins/habitat-plugin.wasm` (~1.2 MB, wasm32-wasip1, opt-level=s + lto + strip).

Automatic hot-reload in active Zellij session.

## Launch

```bash
zellij --layout ~/claude-code-workspace/habitat-zellij/layouts/habitat-fleet.kdl
```

## Pipe Commands

```bash
# Trigger context snapshot (rate limited to 1/min)
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n snapshot

# Query sphere detail
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n query -- "sphere-alpha"

# Status dump
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n status
```

## Keybinds (when plugin pane focused)

| Key | Action |
|-----|--------|
| `r` | Manual refresh all data sources |
| `c` | Toggle coupling matrix detail (coherence_gauge) |
| `j` / `k` | Scroll event_feed down / up |
| `g` | Jump to top of event_feed |
| `q` / `Esc` | Close plugin, snapshot state |

## Hot-Reload

```bash
# Edit any module, then:
bash build.sh
```

The build script calls `zellij action start-or-reload-plugin` automatically.

Module state persists across reload via `serialize_state()` / `restore_state()`.

## Configuration (via KDL)

All values can be overridden per-layout:

```kdl
plugin location="file:~/.config/zellij/plugins/habitat-plugin.wasm" {
    modules "fleet_view,coherence_gauge,bridge_health,event_feed,na_panel,session_timer,cmd_pipe"
    orac_url "http://127.0.0.1:8133"
    pv2_url "http://127.0.0.1:8132"
    synthex_url "http://127.0.0.1:8090"
    nerve_url "http://127.0.0.1:8083"
    coherence_poll "2.0"
    health_poll "5.0"
    governance_poll "10.0"
    layout_mode "full"          # or "compact" or "minimal"
    show_consent_states "true"
    show_attribution "true"
}
```

## Dependencies

- `zellij-tile = "0.43.1"`
- `serde` / `serde_json`
- Target: `wasm32-wasip1` (rustc 1.78+)

## Traps Avoided

1. `run_command()` context tags route async responses back to correct module
2. `set_timeout()` called at END of timer handler to prevent race
3. Bridge URLs are stored internally without a URL scheme prefix; examples include the scheme only for readability
4. PV2 IPC bus handled via separate habitat-telegram binary, not WASM (WASM has no Unix socket)
5. SyntheX v1 health is `/api/health`, not `/health`
6. ME V2 is `:8180`, not `:8080`
7. Pswarm V2 is `:10002`, not `:10001`
8. POVM endpoint is `/pathways` (plural), not `/pathway`
9. PV2 `/field` uses capital `K`, ORAC `/field` uses lowercase `k`
10. Nerve `/health` returns plain text "ok", not JSON

## Bidirectional anchors

- **Plan hub (Obsidian):** [[Comms Layer Unification Plan v3]]
- **Architecture (Obsidian):** [[Comms Layer Unification — Architectural Schematics]] — 9 Mermaid diagrams
- **Canonical plan:** `~/projects/shared-context/Comms Layer Unification Plan — 2026-04-24.md`
- **Charter:** `~/projects/shared-context/Coding Excellence Charter — 2026-04-24.md`
- **Session state:** `CLAUDE.local.md` (plan status table P0–P6)
- **P0+P1 audit:** `ai_docs/P0_P1_AUDIT_2026-04-22.md`
- **Sibling plugins:**
  - `~/claude-code-workspace/habitat-obsidian/README.md` (Obsidian-side plugin, WS-5 retargeted)
  - `~/claude-code-workspace/pane-vortex/README.md` (bus daemon producing events consumed by this plugin via future habitat-wire WS-6)
- **Bus reference schemas:** `~/claude-code-workspace/pane-vortex/schemas/bus-events/INDEX.md`
- **E2E verification:** `~/claude-code-workspace/pane-vortex/scripts/verify-comms-unification-e2e.sh`
- **POVM brief:** memory `be0697a0-b3a8-4c6c-909f-1ad2f9c528eb` at POVM :8125 · pathways under `habitat_comms_unification_*`
