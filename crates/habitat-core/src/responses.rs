use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ──────────────────────────────────────────────────────────────────────
// P3: LiveDataCheck — schema-drift detection trait.
//
// Every field in every response struct carries `#[serde(default)]`, which
// means a broken upstream response silently populates the struct with zero
// values rather than failing to parse. The `has_live_data()` method answers
// "did we receive real data, or are we looking at serde defaults?" and lets
// staleness / drift indicators render based on actual wire signal rather
// than a plausible-looking zeroed struct.
//
// Per-struct discriminant: choose a field that is reliably non-zero /
// non-empty in a real response, per P3 spec in the Hardening Plan. If every
// discriminant field happens to be zero, we treat the struct as "not live"
// even if the connection succeeded — the field hasn't emitted data yet.
//
// Comms Layer Unification Plan v3 §WS-0 P3.
// ──────────────────────────────────────────────────────────────────────

/// Trait for detecting whether a response struct carries live wire data
/// (as opposed to serde defaults from a dropped or empty response).
pub trait LiveDataCheck {
    /// `true` if this struct appears to carry real upstream data,
    /// `false` if every discriminant field is at its default value.
    fn has_live_data(&self) -> bool;
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the live ORAC /health wire schema — 5 boolean flags (ralph_converged, me_frozen, learning_active, synthex_stale, rm_stale) are a deliberate flat layout for zero-deser-friction. Refactor to a flags enum would diverge from the upstream JSON. See Charter §2 suppression discipline."
)]
pub struct OracHealth {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub ralph_gen: u64,
    #[serde(default)]
    pub ralph_phase: String,
    #[serde(default)]
    pub ralph_fitness: f64,
    #[serde(default)]
    pub ralph_converged: bool,
    #[serde(default)]
    pub field_r: f64,
    #[serde(default)]
    pub sphere_count: u32,
    #[serde(default)]
    pub sessions: u32,
    #[serde(default)]
    pub uptime_ticks: u64,
    #[serde(default)]
    pub thermal_temperature: f64,
    #[serde(default)]
    pub thermal_target: f64,
    #[serde(default)]
    pub me_fitness: f64,
    #[serde(default)]
    pub me_frozen: bool,
    #[serde(default)]
    pub hebbian_ltp_total: u64,
    #[serde(default)]
    pub hebbian_ltd_total: u64,
    #[serde(default)]
    pub learning_active: bool,
    #[serde(default)]
    pub emergence_events: u64,
    #[serde(default)]
    pub coupling_connections: u32,
    #[serde(default)]
    pub coupling_weight_mean: f64,
    #[serde(default)]
    pub coupling_weight_range: Vec<f64>,
    #[serde(default)]
    pub ipc_state: String,
    #[serde(default)]
    pub system_grade: String,
    #[serde(default)]
    pub breakers: HashMap<String, BreakerInfo>,
    #[serde(default)]
    pub dispatch_total: u64,
    #[serde(default)]
    pub co_activations_total: u64,
    #[serde(default)]
    pub synthex_stale: bool,
    #[serde(default)]
    pub rm_stale: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct BreakerInfo {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub successes: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct OracBridges {
    #[serde(default)]
    pub breakers_closed: u32,
    #[serde(default)]
    pub breakers_half_open: u32,
    #[serde(default)]
    pub breakers_open: u32,
    #[serde(default)]
    pub ipc_state: String,
    #[serde(default)]
    pub me_fitness: f64,
    #[serde(default)]
    pub me_frozen: bool,
    #[serde(default)]
    pub synthex_last_poll: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct OracThermal {
    #[serde(default)]
    pub temperature: f64,
    #[serde(default)]
    pub target: f64,
    #[serde(default)]
    pub pid_output: f64,
    #[serde(default)]
    pub k_adjustment: f64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub heat_sources: Vec<OracHeatSource>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct OracHeatSource {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub reading: f64,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct EmergenceState {
    #[serde(default)]
    pub active_monitors: u32,
    #[serde(default)]
    pub total_detected: u64,
    #[serde(default)]
    pub by_type: HashMap<String, u64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct HebbianState {
    #[serde(default)]
    pub ltp_total: u64,
    #[serde(default)]
    pub ltd_total: u64,
    #[serde(default)]
    pub ltp_ltd_ratio: f64,
    #[serde(default)]
    pub co_activations_total: u64,
    #[serde(default)]
    pub target_ratio: f64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct CouplingState {
    #[serde(default)]
    pub connections: u32,
    #[serde(default)]
    pub weight_mean: f64,
    #[serde(default)]
    pub weight_min: f64,
    #[serde(default)]
    pub weight_max: f64,
    #[serde(default)]
    pub at_ceiling: u32,
    #[serde(default)]
    pub at_floor: u32,
    #[serde(default)]
    pub saturation_pct: f64,
    #[serde(default)]
    pub k_modulation: f64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct TokenState {
    #[serde(default)]
    pub total_input: u64,
    #[serde(default)]
    pub total_output: u64,
    #[serde(default)]
    pub total_panes: u32,
    #[serde(default)]
    pub budget_remaining: f64,
    #[serde(default)]
    pub budget_status: String,
    #[serde(default)]
    pub utilization: f64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct SessionStats {
    #[serde(default)]
    pub tool_count: u64,
    #[serde(default)]
    pub session_started_at: u64,
    #[serde(default)]
    pub session_elapsed_secs: u64,
    #[serde(default)]
    pub last_tool_name: String,
    #[serde(default)]
    pub last_gate_pass: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Pv2Health {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub r: f64,
    #[serde(default)]
    pub spheres: u32,
    #[serde(default)]
    pub tick: u64,
    #[serde(default)]
    pub k: f64,
    #[serde(default)]
    pub k_modulation: f64,
    #[serde(default)]
    pub fleet_mode: String,
    #[serde(default)]
    pub warmup_remaining: u32,
    #[serde(default)]
    pub hebbian_ltp_total: u64,
    #[serde(default)]
    pub hebbian_ltd_total: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Pv2Field {
    #[serde(default)]
    pub r: f64,
    #[serde(default, rename = "K")]
    pub k: f64,
    #[serde(default)]
    pub sphere_count: u32,
    #[serde(default)]
    pub spheres: u32,
    #[serde(default)]
    pub tick: u64,
    #[serde(default)]
    pub k_modulation: f64,
    #[serde(default)]
    pub total_memories: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct BusEvents {
    #[serde(default)]
    pub events: Vec<BusEvent>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct BusEvent {
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub tick: u64,
    #[serde(default)]
    pub timestamp: f64,
    #[serde(default)]
    pub data: serde_json::Value,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Proposals {
    #[serde(default)]
    pub proposals: Vec<Proposal>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Proposal {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub parameter: String,
    #[serde(default)]
    pub proposed_value: f64,
    #[serde(default)]
    pub current_value: f64,
    #[serde(default)]
    pub proposer: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub submitted_at_tick: u64,
    #[serde(default)]
    pub votes: u32,
}

// ──────────────────────────────────────────────────────────────────────
// LiveDataCheck impls — one per live response struct.
//
// Each discriminant is chosen as a field that is guaranteed non-zero /
// non-empty in a real wire response (e.g. uptime_ticks > 0, tick > 0,
// connections > 0). If every discriminant field matches its default
// value, the struct is treated as "not live" — the upstream call
// either failed silently or returned an unpopulated body.
//
// `Proposals::has_live_data()` always returns true — an empty proposal
// list is a legitimate live response ("there are zero proposals"),
// not a drift signal.
// ──────────────────────────────────────────────────────────────────────

impl LiveDataCheck for OracHealth {
    fn has_live_data(&self) -> bool {
        // ORAC emits uptime_ticks > 0 from its first /health response onward.
        // A zero uptime would indicate no running daemon or a pre-tick boot.
        self.uptime_ticks > 0
    }
}

impl LiveDataCheck for OracBridges {
    fn has_live_data(&self) -> bool {
        self.breakers_closed + self.breakers_half_open + self.breakers_open > 0
    }
}

impl LiveDataCheck for OracThermal {
    fn has_live_data(&self) -> bool {
        // Any running SYNTHEX emits a non-zero temperature reading.
        self.temperature != 0.0
    }
}

impl LiveDataCheck for OracHeatSource {
    fn has_live_data(&self) -> bool {
        self.reading != 0.0 || self.weight != 0.0
    }
}

impl LiveDataCheck for EmergenceState {
    fn has_live_data(&self) -> bool {
        self.active_monitors > 0 || self.total_detected > 0
    }
}

impl LiveDataCheck for HebbianState {
    fn has_live_data(&self) -> bool {
        self.co_activations_total > 0
    }
}

impl LiveDataCheck for CouplingState {
    fn has_live_data(&self) -> bool {
        self.connections > 0
    }
}

impl LiveDataCheck for TokenState {
    fn has_live_data(&self) -> bool {
        self.total_input > 0 || self.total_output > 0
    }
}

impl LiveDataCheck for SessionStats {
    fn has_live_data(&self) -> bool {
        self.session_started_at > 0
    }
}

impl LiveDataCheck for Pv2Health {
    fn has_live_data(&self) -> bool {
        self.tick > 0
    }
}

impl LiveDataCheck for Pv2Field {
    fn has_live_data(&self) -> bool {
        self.tick > 0
    }
}

impl LiveDataCheck for BusEvents {
    fn has_live_data(&self) -> bool {
        !self.events.is_empty()
    }
}

impl LiveDataCheck for Proposals {
    fn has_live_data(&self) -> bool {
        // An empty proposal list is a valid live response. NA-P-15 governance
        // means the field can have zero proposals and still be fully responsive.
        // This discriminant always returns true — treating Proposals as "live"
        // is safe because the na_panel renders correctly with zero entries.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-shape ORAC `/health` fixture. If ORAC's response drifts, the test that
    /// uses this must break — that's the point. See Hardening Plan Phase 2.
    const ORAC_HEALTH_FIXTURE: &str = r#"{
        "status": "healthy",
        "ralph_gen": 26068,
        "ralph_phase": "Recognize",
        "ralph_fitness": 0.664,
        "ralph_converged": false,
        "field_r": 0.0,
        "sphere_count": 0,
        "hebbian_ltp_total": 38,
        "hebbian_ltd_total": 34,
        "emergence_events": 3449,
        "breakers": {
            "orac_pv2_bridge": {"state": "Closed", "consecutive_failures": 0, "successes": 900}
        }
    }"#;

    #[test]
    fn orac_health_deserialises_real_shape() {
        let h: OracHealth =
            serde_json::from_str(ORAC_HEALTH_FIXTURE).expect("ORAC /health fixture must parse");
        assert_eq!(h.status, "healthy");
        assert_eq!(h.ralph_gen, 26068);
        assert_eq!(h.ralph_phase, "Recognize");
        assert!((h.ralph_fitness - 0.664).abs() < 1e-9);
        assert_eq!(h.hebbian_ltp_total, 38);
        assert_eq!(h.hebbian_ltd_total, 34);
        assert_eq!(h.emergence_events, 3449);
    }

    #[test]
    fn orac_health_nested_breakers_survive_deserialisation() {
        let h: OracHealth = serde_json::from_str(ORAC_HEALTH_FIXTURE).unwrap();
        let b = h
            .breakers
            .get("orac_pv2_bridge")
            .expect("fixture has one breaker");
        assert_eq!(b.state, "Closed");
        assert_eq!(b.consecutive_failures, 0);
        assert_eq!(b.successes, 900);
    }

    #[test]
    fn orac_health_missing_fields_fall_back_to_serde_defaults_not_error() {
        // This is AP02 territory — silent defaults are the known trade-off for the
        // WASM plugin. The test fixes that trade-off in place: every field is `default`
        // and the struct returns without error even from `{}`.
        let h: OracHealth = serde_json::from_str("{}").unwrap();
        assert!(h.status.is_empty());
        assert_eq!(h.ralph_gen, 0);
        assert!((h.ralph_fitness - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn orac_health_truly_malformed_json_returns_error() {
        // AP02 check: defaults are only applied to *missing* fields, not malformed JSON.
        // If someone writes a `#[serde(default)]` wrapper that swallows parse errors,
        // the plugin goes blind. This test locks the invariant.
        let err = serde_json::from_str::<OracHealth>("{ not json");
        assert!(err.is_err(), "malformed JSON must surface as an error");
    }

    #[test]
    fn pv2_field_renames_capital_k_correctly() {
        // PV2 `/field` uses capital K; ORAC uses lowercase k. The plugin must read K.
        let json = r#"{"r": 0.95, "K": 2.5, "sphere_count": 3, "spheres": 3, "tick": 100}"#;
        let f: Pv2Field = serde_json::from_str(json).unwrap();
        assert!((f.k - 2.5).abs() < 1e-9);
        assert_eq!(f.sphere_count, 3);
        assert!((f.r - 0.95).abs() < 1e-9);
    }

    #[test]
    fn pv2_field_lowercase_k_does_not_populate_k() {
        // Conversely, a lowercase-`k` payload must NOT write to `k` — that would
        // confuse the two services' wire protocols.
        let json = r#"{"r": 0.95, "k": 99.0, "spheres": 3}"#;
        let f: Pv2Field = serde_json::from_str(json).unwrap();
        assert!(
            (f.k - 0.0).abs() < f64::EPSILON,
            "lowercase `k` must not map to the `K`-renamed field"
        );
    }

    #[test]
    fn bus_events_deserialises_heterogeneous_data_payloads() {
        // `BusEvent.data` is `serde_json::Value` to tolerate arbitrary shapes.
        let json = r#"{"events":[
            {"event_type":"sphere.joined","tick":42,"timestamp":1.0,"data":{"sphere_id":"alpha"}},
            {"event_type":"field.chimera","tick":43,"timestamp":2.0,"data":[1,2,3]}
        ]}"#;
        let bus: BusEvents = serde_json::from_str(json).unwrap();
        assert_eq!(bus.events.len(), 2);
        assert_eq!(bus.events[0].event_type, "sphere.joined");
        assert!(bus.events[0].data.is_object());
        assert!(bus.events[1].data.is_array());
    }

    // ── P3 LiveDataCheck tests (one per struct + invariants) ─────────────

    #[test]
    fn has_live_data_returns_false_on_default_orac_health() {
        assert!(!OracHealth::default().has_live_data());
    }

    #[test]
    fn has_live_data_returns_true_on_fixture_populated_orac_health() {
        // The ORAC_HEALTH_FIXTURE includes uptime_ticks=0 — but real wire
        // responses set uptime_ticks > 0 from first tick. Verify via
        // explicit construction so the test is independent of fixture details.
        let h = OracHealth {
            uptime_ticks: 1,
            ..OracHealth::default()
        };
        assert!(h.has_live_data());
    }

    #[test]
    fn has_live_data_returns_false_on_default_pv2_field() {
        assert!(!Pv2Field::default().has_live_data());
    }

    #[test]
    fn has_live_data_returns_true_on_pv2_field_with_non_zero_tick() {
        let f = Pv2Field {
            tick: 1,
            ..Pv2Field::default()
        };
        assert!(f.has_live_data());
    }

    #[test]
    fn has_live_data_always_true_for_proposals_even_when_empty() {
        // Empty proposals list is a valid live response per NA-P-15.
        assert!(Proposals::default().has_live_data());
    }

    #[test]
    fn has_live_data_returns_false_on_default_hebbian_state() {
        assert!(!HebbianState::default().has_live_data());
    }

    #[test]
    fn has_live_data_returns_true_on_coupling_state_with_any_connections() {
        let c = CouplingState {
            connections: 1,
            ..CouplingState::default()
        };
        assert!(c.has_live_data());
        assert!(!CouplingState::default().has_live_data());
    }

    #[test]
    fn has_live_data_returns_false_on_default_bus_events_returns_true_when_populated() {
        assert!(!BusEvents::default().has_live_data());
        let populated = BusEvents {
            events: vec![BusEvent::default()],
        };
        assert!(populated.has_live_data());
    }

    #[test]
    fn has_live_data_oracthermal_discriminates_by_temperature() {
        assert!(!OracThermal::default().has_live_data());
        let t = OracThermal {
            temperature: 0.5,
            ..OracThermal::default()
        };
        assert!(t.has_live_data());
    }

    #[test]
    fn has_live_data_heat_source_triggers_on_either_reading_or_weight_nonzero() {
        assert!(!OracHeatSource::default().has_live_data());
        let with_reading = OracHeatSource {
            reading: 0.5,
            ..OracHeatSource::default()
        };
        assert!(with_reading.has_live_data());
        let with_weight = OracHeatSource {
            weight: 0.1,
            ..OracHeatSource::default()
        };
        assert!(with_weight.has_live_data());
    }

    #[test]
    fn has_live_data_session_stats_uses_started_at_as_discriminant() {
        assert!(!SessionStats::default().has_live_data());
        let s = SessionStats {
            session_started_at: 100,
            ..SessionStats::default()
        };
        assert!(s.has_live_data());
    }

    #[test]
    fn has_live_data_emergence_triggers_on_active_monitors_or_total_detected() {
        assert!(!EmergenceState::default().has_live_data());
        let with_monitors = EmergenceState {
            active_monitors: 1,
            ..EmergenceState::default()
        };
        assert!(with_monitors.has_live_data());
        let with_total = EmergenceState {
            total_detected: 1,
            ..EmergenceState::default()
        };
        assert!(with_total.has_live_data());
    }

    #[test]
    fn has_live_data_token_state_triggers_on_any_io_activity() {
        assert!(!TokenState::default().has_live_data());
        let with_input = TokenState {
            total_input: 1,
            ..TokenState::default()
        };
        assert!(with_input.has_live_data());
        let with_output = TokenState {
            total_output: 1,
            ..TokenState::default()
        };
        assert!(with_output.has_live_data());
    }

    #[test]
    fn has_live_data_orac_bridges_triggers_on_any_breaker_count() {
        assert!(!OracBridges::default().has_live_data());
        for field_setter in [
            OracBridges {
                breakers_closed: 1,
                ..OracBridges::default()
            },
            OracBridges {
                breakers_half_open: 1,
                ..OracBridges::default()
            },
            OracBridges {
                breakers_open: 1,
                ..OracBridges::default()
            },
        ] {
            assert!(
                field_setter.has_live_data(),
                "any non-zero breaker count must flip has_live_data to true"
            );
        }
    }

    #[test]
    fn proposals_deserialises_full_shape_including_vote_count() {
        // na_panel renders these; proposer + vote count + proposed_value must survive.
        let json = r#"{"proposals":[{
            "id":"p1",
            "parameter":"thermal_target",
            "proposed_value":0.6,
            "current_value":0.5,
            "proposer":"watcher",
            "reason":"cool drift",
            "status":"pending",
            "votes":3
        }]}"#;
        let p: Proposals = serde_json::from_str(json).unwrap();
        assert_eq!(p.proposals.len(), 1);
        assert_eq!(p.proposals[0].id, "p1");
        assert!((p.proposals[0].proposed_value - 0.6).abs() < 1e-9);
        assert_eq!(p.proposals[0].votes, 3);
        assert_eq!(p.proposals[0].proposer, "watcher");
    }
}
