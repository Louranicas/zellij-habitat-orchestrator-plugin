# Bridge Client & Polling Engine

> Back to: [[MOC]] · [[Architecture Schematics]] · [[Dashboard Modules]] · source `crates/habitat-bridge-client/src/lib.rs`

`habitat-bridge-client` is the only WASM-safe HTTP + command transport. It
owns scheduling, stagger, and context routing for **all** endpoints across all
modules — both curl `DataSource`s and host-binary `CommandSource`s (D11).

## Why `run_command(curl)` and not an HTTP client

Inside `wasm32-wasip1` there is no native socket or HTTP stack available to
the plugin runtime. The only safe cross-boundary call is Zellij's
`run_command`, which spawns a host process and delivers its stdout/exit-code
back as a `RunCommandResult` event. The bridge wraps this with pinned curl
argv so the URL is the only variable.

## `BridgeClient` internals

```rust
pub struct BridgeClient {
    endpoints: Vec<ScheduledEndpoint>,
    elapsed: f64,        // wall-clock accumulator driven by Timer events
    stagger_idx: usize,  // position in the stagger-boot pass
    stagger_complete: bool,
}
```

Each `ScheduledEndpoint` carries `argv`, `interval_secs`, `tag`, `module_id`,
and `last_poll`. Two registration paths:

```
register_sources(Vec<DataSource>)
  → wraps URL in pinned curl argv:
    ["curl", "-s", "--max-time", "2", "--connect-timeout", "1", <url>]

register_command_sources(Vec<CommandSource>)
  → uses raw argv as-is (must be absolute path, no shell)
```

## Stagger-then-schedule (the anti-storm pattern)

Boot phase: endpoints are dispatched **one per Timer tick** until all have
fired at least once (`stagger_complete = false`). This prevents the
"all endpoints fire simultaneously on T=0" surge. After stagger: each
endpoint fires when `elapsed - last_poll >= interval_secs`.

> ⚠️ The stagger was added after the [[notes/CPU Saturation RCA Summary]]. It
> reduces but does not eliminate concurrent subprocess count — respect the 30s
> cadence on D11 witness sources.

## Result routing

`handle_result(exit_code, stdout, context)` extracts `tag` and `module_id`
from the context map, then:
- `exit_code != Some(0)` or empty stdout → `HabitatEvent::BridgeError { module_id, tag }`
- otherwise → `HabitatEvent::BridgeData { module_id, tag, raw: stdout.to_vec() }`

Each module's `handle_event` pattern-matches on its own `tag` to consume
exactly its data slice. This is the routing contract — no module ever sees
another module's raw bytes.

## Config validation (habitat-core `config.rs`)

`ModuleConfig::from_btree(BTreeMap)` is the single config-parse entry point
(Hardening P2, no panics). It returns `(ModuleConfig, Vec<ConfigWarning>)`:

- **URL validation**: must be non-empty and start with `http://` or `https://`.
  Bad URL → `ConfigWarning::InvalidUrl` + fall back to default.
- **Poll interval clamping**: `[1.0, 300.0]` seconds. Below/above → `ConfigWarning::PollIntervalClamped`. Non-numeric → `ConfigWarning::PollIntervalNotNumeric`.
- **Never panics**: config failure degrades to documented defaults; the plugin
  always boots.

Default URLs:
| Key | Default |
|---|---|
| `orac_url` | `http://127.0.0.1:8133` |
| `pv2_url` | `http://127.0.0.1:8132` |
| `synthex_url` | `http://127.0.0.1:8090` |
| `nerve_url` | `http://127.0.0.1:8083` |

## Poll intervals

| Module | Interval | Source |
|---|---|---|
| `coherence_gauge` | `coherence_poll` (default 2s) | PV2 field |
| `fleet_view`, `bridge_health`, `event_feed` | `health_poll` (default 5s) | service grid |
| `na_panel`, `session_timer` | `governance_poll` (default 10s) | proposals / stats |
| `orchestrator_kernel` | `kernel_poll` (default 30s) | sidecar snapshot |
| D11 witnesses (`fiber_cockpit`, `sphere_warden`) | 30s | host helper |

## Planned improvement (P3, not started)

URL dedup + `[STALE Xs]` indicator — when a curl poll returns the same bytes
as the previous response, surface a stale marker instead of re-rendering
identical data. See [[Task Status & Roadmap]].
