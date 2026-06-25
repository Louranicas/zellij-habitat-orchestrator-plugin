//! `sphere_warden` — the SENSE organ of the agentic-factory coordination fabric.
//!
//! Surfaces the gap between live Zellij panes and registered PV2 Kuramoto spheres —
//! the D7 field-under-population diagnosis (a 2-sphere field can't phase-stagger
//! anything). It is fed by `bin/zj-sphere-warden`, a READ-ONLY helper polled as a
//! [`CommandSource`]; the result arrives as `BridgeData { tag: "sphere_warden" }`.
//!
//! # Observe-only (deliberate)
//! This first cut is a pure SENSOR: it DIAGNOSES the coverage gap and surfaces the
//! arming-key readiness, but never registers a sphere. Auto-registration needs a
//! ratified sphere-id convention (live spheres use `domain:session:pane`; Zellij
//! exposes only generic `terminal_N` ids) and anti-burst discipline (the pswarm
//! registration-burst SIGABRT scar). Actuation is a documented follow-up gated on
//! Luke ratifying the convention — so the witness/sensor never writes from Rust, and
//! the helper itself issues no `register`.

use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{CommandSource, DataSource, HabitatModule};
use habitat_core::render::*;
use serde::{Deserialize, Serialize};

/// Absolute path to the read-only sensor helper (`run_command` execs argv directly).
const WARDEN_HELPER: &str = "/home/louranicas/claude-code-workspace/bin/zj-sphere-warden";
/// Sensor poll cadence — coverage drifts slowly; awareness, not telemetry.
const WARDEN_POLL_SECS: f64 = 30.0;
/// Command-source/`BridgeData` tag the warden status arrives under.
const WARDEN_TAG: &str = "sphere_warden";
/// No fresh status for this long → header stale tag (3× the 30s cadence).
const STALE_THRESHOLD_SECS: f64 = 90.0;

/// The read-only field-coverage status produced by `bin/zj-sphere-warden`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct WardenStatus {
    #[serde(default)]
    pub v: u32,
    #[serde(default)]
    pub ts: u64,
    #[serde(default)]
    pub armed: bool,
    #[serde(default)]
    pub pv2_up: bool,
    #[serde(default)]
    pub spheres: u32,
    #[serde(default)]
    pub panes: u32,
    #[serde(default)]
    pub gap: u32,
    #[serde(default)]
    pub actuation: String,
    /// Zellij session name (for the source-verified `cc:<session>:<pane>` convention).
    #[serde(default)]
    pub session: String,
    /// The source-verified closure command template (operator guidance; empty when gap 0).
    /// Rendered, never executed — the sensor only surfaces the path to closure.
    #[serde(default)]
    pub closure: String,
    #[serde(default)]
    pub errors: Vec<String>,
}

/// The `sphere_warden` module state (observe-only — holds only the last status).
pub struct SphereWarden {
    status: Option<WardenStatus>,
    ticks_since_data: u64,
    poll_secs: f64,
}

impl SphereWarden {
    /// Construct an empty warden (no status until the first sensor poll returns).
    #[must_use]
    pub fn new() -> Self {
        Self {
            status: None,
            ticks_since_data: 0,
            poll_secs: 5.0,
        }
    }

    /// Apply a parsed status value (the `BridgeData` feed). Returns true if it
    /// deserialised into a [`WardenStatus`].
    fn apply_value(&mut self, value: &serde_json::Value) -> bool {
        match serde_json::from_value::<WardenStatus>(value.clone()) {
            Ok(s) => {
                self.status = Some(s);
                self.ticks_since_data = 0;
                true
            }
            Err(_) => false,
        }
    }

    fn stale_seconds(&self) -> f64 {
        // cast_precision_loss accepted crate-wide (lib.rs); display-only estimate.
        self.ticks_since_data as f64 * self.poll_secs.max(1.0)
    }
}

impl Default for SphereWarden {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for SphereWarden {
    fn id(&self) -> &'static str {
        "sphere_warden"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.poll_secs = config.health_poll;
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::BridgeData { tag, data, .. } if tag == WARDEN_TAG => {
                self.apply_value(data)
            }
            HabitatEvent::Tick { .. } => {
                self.ticks_since_data = self.ticks_since_data.saturating_add(1);
                false
            }
            _ => false,
        }
    }

    fn render(&self, _rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);
        let Some(s) = &self.status else {
            return vec![RenderLine::new(format!(
                " {CYN}\u{25c9} warden{R}  {D}sensing… (waiting for first poll){R}"
            ))];
        };

        let stale = stale_tag(self.stale_seconds(), STALE_THRESHOLD_SECS).unwrap_or_default();
        let pv2 = if s.pv2_up {
            format!("{GRN}PV2 up{R}")
        } else {
            format!("{RED}PV2 down{R}")
        };
        // gap>0 is the field-under-population signal worth a glance.
        let gap = if s.gap > 0 {
            format!("{YEL}gap {}{R}", s.gap)
        } else {
            format!("{GRN}gap 0{R}")
        };
        let arm = if s.armed {
            format!("{YEL}armed{R}")
        } else {
            format!("{D}observe-only{R}")
        };
        let err = if s.errors.is_empty() {
            String::new()
        } else {
            format!("  {RED}!{}{R}", s.errors.len())
        };

        let mut lines = vec![RenderLine::new(format!(
            " {B}{CYN}\u{25c9} WARDEN{R}  {} {D}·{R} panes {} / spheres {} {D}·{R} {} {D}·{R} {}{}{}",
            pv2,
            s.panes,
            s.spheres,
            gap,
            arm,
            err,
            if stale.is_empty() { String::new() } else { format!("  {stale}") },
        ))];

        // One advisory line when the field is under-populated (observe-only — the
        // closure command is SHOWN, never run: actuation stays arming + convention gated).
        if s.gap > 0 {
            let closure = if s.closure.is_empty() {
                "register via pane-vortex-ctl".to_string()
            } else {
                truncate(&s.closure, w.saturating_sub(28)).to_string()
            };
            lines.push(RenderLine::new(format!(
                " {D}{} uncovered · closure: {} {D}(gated){R}",
                s.gap, closure,
            )));
        }
        lines
    }

    fn serialize_state(&self) -> Option<String> {
        None // observe-only sensor: the status re-feeds on next poll; nothing to persist.
    }

    fn restore_state(&mut self, _state: &str) {}

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::BridgeResponse, EventCategory::Tick]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        Vec::new()
    }

    fn command_sources(&self) -> Vec<CommandSource> {
        vec![CommandSource {
            argv: vec![WARDEN_HELPER.to_string()],
            interval_secs: WARDEN_POLL_SECS,
            tag: WARDEN_TAG.to_string(),
            module_id: self.id().to_string(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../tests/fixtures/sphere_warden_golden.json");

    fn loaded() -> SphereWarden {
        let mut w = SphereWarden::new();
        let v: serde_json::Value = serde_json::from_str(GOLDEN).expect("golden json");
        assert!(w.apply_value(&v), "golden fixture must parse");
        w
    }
    fn bridge(tag: &str, v: serde_json::Value) -> HabitatEvent {
        HabitatEvent::BridgeData {
            module_id: "sphere_warden".into(),
            tag: tag.into(),
            data: v,
        }
    }

    // ── construction ─────────────────────────────────────────────────────
    #[test]
    fn new_is_empty() {
        assert!(SphereWarden::new().status.is_none());
    }
    #[test]
    fn default_equals_new() {
        assert!(SphereWarden::default().status.is_none());
    }
    #[test]
    fn id_and_version() {
        let w = SphereWarden::new();
        assert_eq!(w.id(), "sphere_warden");
        assert_eq!(w.version(), "0.1.0");
    }
    #[test]
    fn subscriptions_bridge_and_tick_only() {
        let s = SphereWarden::new().subscriptions();
        assert!(s.contains(&EventCategory::BridgeResponse));
        assert!(s.contains(&EventCategory::Tick));
        assert!(!s.contains(&EventCategory::KeyPress));
        assert!(!s.contains(&EventCategory::PipeCommand));
    }
    #[test]
    fn data_sources_empty() {
        assert!(SphereWarden::new().data_sources().is_empty());
    }

    // ── command source ───────────────────────────────────────────────────
    #[test]
    fn command_sources_declares_warden_helper() {
        let cs = SphereWarden::new().command_sources();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].tag, "sphere_warden");
        assert_eq!(cs[0].module_id, "sphere_warden");
        assert!(
            cs[0].argv[0].starts_with('/'),
            "argv[0] absolute (no shell)"
        );
        assert!(cs[0].argv[0].ends_with("zj-sphere-warden"));
        assert!((cs[0].interval_secs - 30.0).abs() < f64::EPSILON);
    }

    // ── feed / parse ─────────────────────────────────────────────────────
    #[test]
    fn golden_fixture_parses() {
        let w = loaded();
        let s = w.status.as_ref().unwrap();
        assert!(s.pv2_up);
        assert_eq!(s.actuation, "observe-only");
    }
    #[test]
    fn bridge_data_on_tag_applies() {
        let mut w = SphereWarden::new();
        let v: serde_json::Value = serde_json::from_str(GOLDEN).unwrap();
        assert!(w.handle_event(&bridge("sphere_warden", v)));
        assert!(w.status.is_some());
    }
    #[test]
    fn bridge_data_other_tag_ignored() {
        let mut w = loaded();
        assert!(!w.handle_event(&bridge("not_ours", serde_json::json!({"gap": 9}))));
    }
    #[test]
    fn malformed_value_ignored_keeps_prior() {
        let mut w = loaded();
        assert!(!w.apply_value(&serde_json::json!("not an object")));
        assert!(w.status.is_some(), "bad value keeps prior status");
    }
    #[test]
    fn drift_tolerant_unknown_fields() {
        let mut w = SphereWarden::new();
        assert!(w.apply_value(&serde_json::json!({"v":2,"future":true,"gap":3})));
        assert_eq!(w.status.as_ref().unwrap().gap, 3);
    }
    #[test]
    fn apply_resets_stale_counter() {
        let mut w = loaded();
        w.ticks_since_data = 40;
        let v: serde_json::Value = serde_json::from_str(GOLDEN).unwrap();
        w.apply_value(&v);
        assert_eq!(w.ticks_since_data, 0);
    }

    // ── tick / stale ─────────────────────────────────────────────────────
    #[test]
    fn tick_ages_without_render() {
        let mut w = loaded();
        assert!(!w.handle_event(&HabitatEvent::Tick { tick: 1 }));
        assert_eq!(w.ticks_since_data, 1);
    }
    #[test]
    fn tick_saturates() {
        let mut w = SphereWarden::new();
        w.ticks_since_data = u64::MAX;
        w.handle_event(&HabitatEvent::Tick { tick: 1 });
        assert_eq!(w.ticks_since_data, u64::MAX);
    }
    #[test]
    fn stale_seconds_uses_poll_floor() {
        let mut w = SphereWarden::new();
        w.poll_secs = 0.0;
        w.ticks_since_data = 7;
        assert!((w.stale_seconds() - 7.0).abs() < f64::EPSILON);
    }
    #[test]
    fn key_and_pipe_events_ignored() {
        let mut w = loaded();
        assert!(!w.handle_event(&HabitatEvent::KeyPress { key: 'a' }));
        assert!(!w.handle_event(&HabitatEvent::PipeCommand {
            name: "x".into(),
            payload: String::new()
        }));
    }
    #[test]
    fn bridge_error_ignored() {
        let mut w = loaded();
        assert!(!w.handle_event(&HabitatEvent::BridgeError {
            module_id: "x".into(),
            tag: "sphere_warden".into()
        }));
    }

    // ── render ───────────────────────────────────────────────────────────
    #[test]
    fn render_empty_shows_sensing_banner() {
        let lines = SphereWarden::new().render(20, 90);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].content.contains("warden"));
        assert!(lines[0].content.contains("sensing"));
    }
    #[test]
    fn render_shows_coverage_and_observe_only() {
        let joined = loaded()
            .render(20, 100)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("WARDEN"));
        assert!(joined.contains("panes"));
        assert!(joined.contains("spheres"));
        assert!(joined.contains("observe-only"));
    }
    #[test]
    fn render_gap_zero_is_single_line() {
        // golden has gap 0 → no advisory line.
        let lines = loaded().render(20, 100);
        assert_eq!(lines.len(), 1, "no gap → just the header");
    }
    #[test]
    fn render_gap_positive_adds_advisory_line() {
        let mut w = SphereWarden::new();
        w.apply_value(&serde_json::json!({"v":1,"pv2_up":true,"spheres":2,"panes":9,"gap":7,"actuation":"observe-only"}));
        let lines = w.render(20, 100);
        assert!(lines.len() >= 2, "gap>0 surfaces an advisory line");
        let joined = lines.iter().map(|l| l.content.as_str()).collect::<String>();
        assert!(joined.contains("uncovered"));
        assert!(
            joined.contains("gated"),
            "actuation is gated, not performed"
        );
    }

    #[test]
    fn render_closure_command_when_present() {
        // The source-verified closure path is SHOWN to the operator (never executed).
        let mut w = SphereWarden::new();
        w.apply_value(&serde_json::json!({
            "v":1,"pv2_up":true,"spheres":5,"panes":35,"gap":30,
            "session":"quiet-echidna",
            "closure":"pane-vortex-ctl register cc:quiet-echidna:<paneN> cc-pane"
        }));
        let joined = w
            .render(20, 120)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("closure:"));
        assert!(
            joined.contains("cc:quiet-echidna"),
            "renders the verified id convention"
        );
    }

    #[test]
    fn session_and_closure_parse_from_value() {
        let mut w = SphereWarden::new();
        w.apply_value(&serde_json::json!({"v":1,"session":"foo","closure":"x","gap":1}));
        let s = w.status.as_ref().unwrap();
        assert_eq!(s.session, "foo");
        assert_eq!(s.closure, "x");
    }

    #[test]
    fn render_gap_zero_omits_closure_line() {
        let mut w = SphereWarden::new();
        w.apply_value(&serde_json::json!({"v":1,"pv2_up":true,"spheres":7,"panes":7,"gap":0}));
        assert_eq!(w.render(20, 100).len(), 1, "no gap → no closure advisory");
    }
    #[test]
    fn render_armed_state_shown() {
        let mut w = SphereWarden::new();
        w.apply_value(
            &serde_json::json!({"v":1,"pv2_up":true,"armed":true,"spheres":3,"panes":3,"gap":0}),
        );
        let joined = w
            .render(20, 100)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("armed"));
    }
    #[test]
    fn render_pv2_down_shown() {
        let mut w = SphereWarden::new();
        w.apply_value(&serde_json::json!({"v":1,"pv2_up":false,"spheres":0,"panes":0,"gap":0}));
        let joined = w
            .render(20, 100)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("PV2 down"));
    }
    #[test]
    fn render_error_tag_when_helper_errored() {
        let mut w = SphereWarden::new();
        w.apply_value(&serde_json::json!({"v":1,"pv2_up":false,"errors":["pv2-unreachable"]}));
        let joined = w
            .render(20, 100)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("!1"));
    }
    #[test]
    fn render_never_panics_across_dims() {
        let w = loaded();
        for (r, c) in [(0, 0), (1, 1), (2, 5), (8, 200)] {
            let _ = w.render(r, c);
        }
    }

    // ── serialize (stateless sensor) ─────────────────────────────────────
    #[test]
    fn serialize_state_is_none() {
        assert!(loaded().serialize_state().is_none());
    }
    #[test]
    fn restore_state_is_noop() {
        let mut w = SphereWarden::new();
        w.restore_state("anything");
        assert!(w.status.is_none());
    }

    // ── boundary: this module never names a write verb ───────────────────
    #[test]
    fn observe_only_actuation_label() {
        // The fixture and any healthy poll must report observe-only — the sensor
        // never registers. (The helper is grep-gated separately for `register`.)
        assert_eq!(loaded().status.unwrap().actuation, "observe-only");
    }
}
