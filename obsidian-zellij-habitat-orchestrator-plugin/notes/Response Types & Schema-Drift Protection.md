# Response Types & Schema-Drift Protection

> Back to: [[MOC]] · [[Architecture Schematics]] · [[notes/Bridge Client & Polling Engine]]
> Source: `crates/habitat-core/src/responses.rs`

`responses.rs` defines the **12 live wire-schema structs** used across the
dashboard modules. Every field carries `#[serde(default)]` — the intentional
drift-tolerance design — and the `LiveDataCheck` trait gives modules a
first-order way to detect whether they are rendering real data or serde
zero-defaults.

---

## The silent-default problem

All structs use `#[serde(default)]` so a broken/empty upstream response
succeeds to parse rather than returning `Err`. This means:

> A module that lost its upstream service will render a confidently-styled
> zeroed struct (`"0 services up"`, `"r = 0.0"`) rather than an error state.

The `LiveDataCheck` trait and `has_live_data()` method fix this:

```rust
pub trait LiveDataCheck {
    fn has_live_data(&self) -> bool;
}
```

Each struct chooses a **discriminant field** that is reliably non-zero in a
real response (e.g., `OracHealth.ralph_gen`, `Pv2Field.tick`). If that field
is at its default, `has_live_data()` returns `false` and the module can render
`[NO DATA]` / stale indicator instead.

> **P3 OPEN WORK:** `has_live_data()` is implemented on the structs but modules
> do not yet call it. Wiring it into stale-tag rendering is P3. See
> [[Task Status & Roadmap]].

---

## The 12 live structs

### `OracHealth`

ORAC `/health` response. Discriminant: `ralph_gen > 0`.

```
status: String, ralph_gen: u64, ralph_phase: String, ralph_fitness: f64,
ralph_converged: bool, field_r: f64, sphere_count: u64,
hebbian_ltp_total: u64, hebbian_ltd_total: u64, emergence_events: u64,
breakers: HashMap<String, BreakerInfo { state, consecutive_failures, successes }>
```

**Golden fixture** (nominal):
```json
{ "status": "healthy", "ralph_gen": 26068, "ralph_phase": "Recognize",
  "ralph_fitness": 0.664, "ralph_converged": false, "field_r": 0.0,
  "sphere_count": 0, "hebbian_ltp_total": 38, "hebbian_ltd_total": 34,
  "emergence_events": 3449,
  "breakers": { "orac_pv2_bridge": { "state": "Closed", "consecutive_failures": 0, "successes": 900 } } }
```

`#[allow(clippy::struct_excessive_bools)]` — mirrors the ORAC wire schema's 5
boolean flags. Refactoring to an enum would diverge from upstream JSON.

### `OracBridges`

ORAC `/bridges` — bridge open/closed/half-open counts. Used by `bridge_health`.

### `OracThermal`

ORAC `/thermal` — thermal PID state. Used by `bridge_health`.

### `EmergenceState`

ORAC `/field` emergence.recent — recent emergence events. Used by `event_feed`.

### `HebbianState`

ORAC `/hebbian` — Hebbian LTP/LTD totals. Used by `coherence_gauge`.

### `CouplingState`

ORAC `/coupling` — inter-sphere coupling weights. Used by `coherence_gauge`.
`show_coupling_detail` in config hides the raw matrix (S6 compliance).

### `TokenState`

ORAC `/session-stats` token budget. Used by `session_timer`.

### `SessionStats`

ORAC `/session-stats` session stats. Used by `session_timer`.

### `Pv2Health`

PV2 `/health`. Used by `fleet_view`.

### `Pv2Field`

PV2 `/field` — Kuramoto order parameter `r`, coupling `K`, sphere count, tick.
Discriminant: `tick > 0`.

**Golden fixture** (nominal):
```json
{ "r": 0.95, "K": 2.5, "sphere_count": 3, "spheres": 3,
  "tick": 1157938, "k_modulation": 1.05, "total_memories": 48 }
```

### `BusEvents`

PV2 `/bus/events` — event stream. Used by `event_feed`.

### `Proposals`

ORAC `/field/proposals` — NA proposals. Used by `na_panel`.

---

## The 6 dead structs (deleted in P1.G5 / P2)

`RalphState`, `Pv2Spheres`, `Pv2BridgesHealth`, `BusInfo`,
`SynthexThermal` (+children `SynthexHeatSource`, `ThermalAdjustments`),
`PovmHealth` — all removed. Rationale: see
[[notes/P0 P1 Security Audit (2026-04-22)]] §P1.G5 and
[[notes/Durable Lessons & Design Decisions]] §D3.

---

## Fixture strategy

Tier-1 fixtures live in `crates/habitat-core/tests/fixtures/` and
`crates/habitat-modules/tests/fixtures/`. They are **real wire captures**,
not hand-constructed JSON, so tests prove the parse path against actual
upstream schema:

| File | Tests |
|---|---|
| `orac_health_nominal.json` | `OracHealth` parse + `has_live_data` |
| `orac_health_active.json` | active-RALPH variant |
| `pv2_field_nominal.json` | `Pv2Field` parse |
| `pv2_field_idle.json` | idle variant |
| `proposals_single.json` | `Proposals` single-entry |
| `bus_events_mixed.json` | mixed event types |
| `fiber_snapshot_golden.json` | `FiberSnapshot` full campaign tree |
| `sphere_warden_golden.json` | `WardenStatus` gap-0 state |

The **Tier-2 live-snapshot script** (`scripts/capture-fixtures.sh`) re-captures
all fixtures from the live services — ensures no test-fixture drift from
upstream schema changes.

---

## See also

- [[notes/Bridge Client & Polling Engine]] — how struct instances are created from wire bytes
- [[notes/P0 P1 Security Audit (2026-04-22)]] — §P1.G5 dead-struct audit
- [[Dashboard Modules]] — which modules consume which structs
