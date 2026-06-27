# Dashboard Modules

> Back to: [[MOC]] · [[Architecture Schematics]] · source `crates/habitat-modules/src/`

11 built-in dashboard modules, each with isolated state, event handling,
rendering, and tests. Default surface (S1007736):
`fleet_view,bridge_health,fiber_cockpit,campaign_attention,sphere_warden`.

## The 11 modules

| Module | Shows | Data sources | Keybinds |
|---|---|---|---|
| `fleet_view` | ORAC/PV2 health + field overview | ORAC `/health`, PV2 `/health` | — |
| `bridge_health` | 14-service grid, thermal band, bridge/breaker status | ORAC `/bridges`, `/thermal`, 14× `/health` | — |
| `coherence_gauge` | Field coherence, coupling, Hebbian context | PV2 `/field`, ORAC `/coupling`, `/hebbian` | `c` toggle detail |
| `event_feed` | Emergence + bus-event stream w/ confidence/TTL cues | ORAC `/field`, PV2 `/bus/events` | `j`/`k` scroll, `g` top |
| `na_panel` | Governance proposals + consent/attribution markers | ORAC `/health` | — |
| `session_timer` | Session + token state | tick counter | — |
| `cmd_pipe` | Rate-limited Zellij pipe command handling | Zellij pipe | — |
| `campaign_attention` | Campaign lease/arming/new-state ambient alerts (Quiet→NEW digest) | shared `fiber_snapshot` `BridgeData` | `a` ack |
| `fiber_cockpit` | Fiber/campaign navigation + lease visibility | self-poll `bin/fiber-cockpit-snapshot` @5s (tag `fiber_snapshot`) | `j`/`k` select, `l`/Enter expand, `h` back, `g` top |
| `sphere_warden` | Live pane↔PV2-sphere coverage gap — **observe-only** | self-poll `bin/zj-sphere-warden` @30s | — |
| `orchestrator_kernel` | Kernel snapshot/status rendering | sidecar snapshot | — |
| *global* | — | — | `r` refresh, `q`/`Esc` close |

## The grid (`bridge_health`)

Renders 14 services: `V3 Nerve TL SX V8 VMS POVM RM PV2 ORAC Inj WFE ME PSw`
(Wave-16 added `(8142, "WFE")` — S1005032). Live screenshot at deploy showed
`ALL UP 14/14`.

## The three D11 "witnesses" (S1007594)

`fiber_cockpit`, `campaign_attention`, `sphere_warden` are the agentic-factory
witnesses — **read-only self-pollers** added via the `command_sources()` trait
extension on `HabitatModule` (additive, default `Vec::new()` — the 7 original
modules are untouched). Key invariants:

- Self-poll helpers are **absolute-path host execs** (`argv[0]` must be absolute —
  the host runs them directly, no shell).
- All three are **mechanically grep-gated to zero writes**.
- `sphere_warden` is **observe-only**: it surfaces the pane↔PV2 coverage gap (live
  spheres are `domain:session:pane`; Zellij exposes only `terminal_N`) but **never
  registers a sphere** — auto-registration is held pending Luke ratifying the
  sphere-id convention + anti-burst discipline (the pswarm SIGABRT scar). It reads
  + surfaces the `warden.enabled` arming key but issues no `register`.

> ⚠️ The `fiber_cockpit` self-poll is the root of the **CPU-saturation storm** —
> see [[Bugs & Known Issues]].

## Module trait

`HabitatModule` (in `habitat-core/src/module.rs`) defines the lifecycle: event
handling, render, key handling, plus the additive `command_sources()` →
`Vec<CommandSource>` (D11, Luke-approved S1007594). The bridge client schedules
raw-argv local commands beside curl `DataSource`s in one stagger/interval loop
(`register_command_sources`) and routes results through the same `BridgeData{tag}`
path.

## See also

- [[Command Surface]] — pipe commands these modules respond to
- [[Architecture Schematics]] — where modules sit relative to the bridge + WASM line
