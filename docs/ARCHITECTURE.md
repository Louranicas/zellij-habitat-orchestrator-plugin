# Architecture

Back to [README](../README.md) · [Docs index](INDEX.md)

`zellij-habitat-orchestrator-plugin` is a Rust workspace with a Zellij WASM
plugin at the edge and a durable sidecar behind the admission boundary.

## Crates

| Crate | Role |
| --- | --- |
| `habitat-core` | Shared contracts: events, module trait, config parsing, render helpers, response structs. |
| `habitat-bridge-client` | Tagged command transport through Zellij `run_command`; snapshot fan-out into module-scoped bridge events. |
| `habitat-modules` | Built-in dashboard modules with isolated state, event handling, rendering, and tests. |
| `habitat-plugin` | Zellij WASM entrypoint, module orchestration, key handling, pipe handling, sidecar response formatting. |
| `orchestrator-kernel-sidecar` | Durable submit path, event log, hash-chain replay, policy hash checks, idempotency, built-in no-shell recipe execution. |

## Runtime Planes

```text
operator
  |
  v
Zellij pane / pipe
  |
  v
habitat-plugin.wasm  ---- read-only probes ----> ORAC / PV2 / services
  |
  | kernel submit only
  v
orch-kernelctl / orchestrator-kernel-sidecar
  |
  v
durable event log + hash chain + replay verification
```

## Module Surface

| Module | Primary Responsibility |
| --- | --- |
| `fleet_view` | ORAC/PV2 health and field overview. |
| `coherence_gauge` | Field coherence, coupling, Hebbian context. |
| `bridge_health` | Service grid, thermal band, bridge/breaker status. |
| `event_feed` | Emergence and bus-event stream with confidence/TTL cues. |
| `na_panel` | Governance proposals and consent/attribution markers. |
| `session_timer` | Session and token state. |
| `cmd_pipe` | Rate-limited Zellij pipe command handling. |
| `campaign_attention` | Campaign lease/arming/new-state attention surface. |
| `fiber_cockpit` | Fiber/campaign navigation and lease visibility. |
| `sphere_warden` | Sphere closure and observe-only guardrail rendering. |
| `orchestrator_kernel` | Kernel snapshot/status rendering. |

## Admission Boundary

The WASM plugin can receive a Zellij pipe, but durable admission is not complete
until the sidecar returns a durable response. The sidecar owns:

- canonical request hashing;
- idempotency replay;
- append-only event persistence;
- hash-chain verification;
- fixed recipe allowlist;
- policy hash validation.

See [Security](SECURITY.md) for the fail-closed rules.
