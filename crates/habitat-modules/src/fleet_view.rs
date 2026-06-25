use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use habitat_core::responses::{OracHealth, Pv2Health};

pub struct FleetView {
    orac: OracHealth,
    pv2: Pv2Health,
    orac_url: String,
    pv2_url: String,
    poll_secs: f64,
}

impl FleetView {
    #[must_use]
    pub fn new() -> Self {
        Self {
            orac: OracHealth::default(),
            pv2: Pv2Health::default(),
            orac_url: "http://127.0.0.1:8133".into(),
            pv2_url: "http://127.0.0.1:8132".into(),
            poll_secs: 5.0,
        }
    }
}

impl Default for FleetView {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for FleetView {
    fn id(&self) -> &'static str {
        "fleet_view"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.orac_url.clone_from(&config.orac_url);
        self.pv2_url.clone_from(&config.pv2_url);
        self.poll_secs = config.health_poll;
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::BridgeData { tag, data, .. } => {
                match tag.as_str() {
                    "orac_health" => {
                        if let Ok(h) = serde_json::from_value(data.clone()) {
                            self.orac = h;
                            return true;
                        }
                    }
                    "pv2_health" => {
                        if let Ok(h) = serde_json::from_value(data.clone()) {
                            self.pv2 = h;
                            return true;
                        }
                    }
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    fn render(&self, _rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);
        let mut lines = Vec::new();

        lines.push(RenderLine::new(format!(
            " {B}{CYN}FLEET{R}  {D}tick {}{R}",
            fmt_num(self.pv2.tick),
        )));
        lines.push(RenderLine::separator(w));

        let pv_icon = if self.pv2.status == "healthy" {
            format!("{GRN}{ICON_UP}{R}")
        } else {
            format!("{RED}{ICON_DOWN}{R}")
        };
        let orac_icon = if self.orac.status == "healthy" {
            format!("{GRN}{ICON_UP}{R}")
        } else {
            format!("{RED}{ICON_DOWN}{R}")
        };

        lines.push(RenderLine::new(format!(
            " {pv_icon}{CYN}PV2{R} r={B}{:.3}{R} sph={B}{}{R} mode={D}{}{R}",
            self.pv2.r, self.pv2.spheres, self.pv2.fleet_mode,
        )));

        let phase_cycle = cycle_indicator(&self.orac.ralph_phase);
        lines.push(RenderLine::new(format!(
            " {orac_icon}{MAG}ORAC{R} {D}RALPH{R} [{phase_cycle}] gen={B}{}{R} fit={B}{:.3}{R}",
            fmt_num(self.orac.ralph_gen),
            self.orac.ralph_fitness,
        )));

        let (temp_color, temp_label) =
            thermal_band(self.orac.thermal_temperature, self.orac.thermal_target);
        lines.push(RenderLine::new(format!(
            " {D}Thermal{R} {temp_color}{temp_label}{R} T={B}{:.2}{R}/{:.2} {D}ME{R} fit={B}{:.3}{R}",
            self.orac.thermal_temperature,
            self.orac.thermal_target,
            self.orac.me_fitness,
        )));

        if self.pv2.spheres == 0 {
            lines.push(RenderLine::new(format!(
                " {D}STDP: idle (no active couplings){R}",
            )));
        } else if self.pv2.spheres == 1 {
            lines.push(RenderLine::new(format!(
                " {D}STDP: warming up (solo sphere){R}",
            )));
        } else {
            let ratio = if self.orac.hebbian_ltd_total > 0 {
                format!(
                    "{:.2}",
                    self.orac.hebbian_ltp_total as f64 / self.orac.hebbian_ltd_total as f64
                )
            } else {
                "inf".into()
            };
            lines.push(RenderLine::new(format!(
                " {D}STDP{R} LTP={B}{}{R} LTD={B}{}{R} ratio={B}{ratio}{R} cpl={B}{}{R}",
                self.orac.hebbian_ltp_total,
                self.orac.hebbian_ltd_total,
                self.orac.coupling_connections,
            )));
        }

        lines.push(RenderLine::new(format!(
            " {D}Emerge{R} {B}{}{R} events  {D}Dispatch{R} {B}{}{R}  {D}Grade{R} {B}{}{R}",
            fmt_num(self.orac.emergence_events),
            fmt_num(self.orac.dispatch_total),
            self.orac.system_grade,
        )));

        lines
    }

    fn serialize_state(&self) -> Option<String> {
        serde_json::to_string(&(&self.orac, &self.pv2)).ok()
    }

    fn restore_state(&mut self, state: &str) {
        if let Ok((orac, pv2)) = serde_json::from_str::<(OracHealth, Pv2Health)>(state) {
            self.orac = orac;
            self.pv2 = pv2;
        }
    }

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::BridgeResponse]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        vec![
            DataSource {
                url: format!("{}/health", self.orac_url),
                interval_secs: self.poll_secs,
                tag: "orac_health".into(),
                module_id: self.id().into(),
            },
            DataSource {
                url: format!("{}/health", self.pv2_url),
                interval_secs: self.poll_secs,
                tag: "pv2_health".into(),
                module_id: self.id().into(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn make_config() -> ModuleConfig {
        let mut m = BTreeMap::new();
        m.insert("orac_url".into(), "http://orac.test:9133".into());
        m.insert("pv2_url".into(), "http://pv2.test:9132".into());
        m.insert("health_poll".into(), "7.0".into());
        ModuleConfig::from_btree(&m).0
    }

    #[test]
    fn new_starts_with_canonical_localhost_urls() {
        let f = FleetView::new();
        assert_eq!(f.orac_url, "http://127.0.0.1:8133");
        assert_eq!(f.pv2_url, "http://127.0.0.1:8132");
        assert!((f.poll_secs - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn init_overwrites_urls_and_reads_health_poll_not_coherence_or_governance() {
        // Correct field binding: fleet_view reads health_poll, not the other two intervals.
        // A regression that binds to coherence_poll would cause fleet to over-poll.
        let mut m = BTreeMap::new();
        m.insert("orac_url".into(), "http://orac.test:9133".into());
        m.insert("pv2_url".into(), "http://pv2.test:9132".into());
        m.insert("coherence_poll".into(), "1.5".into());
        m.insert("health_poll".into(), "7.0".into());
        m.insert("governance_poll".into(), "30.0".into());
        let (cfg, _) = ModuleConfig::from_btree(&m);
        let mut f = FleetView::new();
        f.init(&cfg);
        assert_eq!(f.orac_url, "http://orac.test:9133");
        assert_eq!(f.pv2_url, "http://pv2.test:9132");
        assert!((f.poll_secs - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_orac_health_updates_state_and_returns_true() {
        let mut f = FleetView::new();
        let data = json!({
            "status": "healthy",
            "ralph_gen": 100,
            "ralph_phase": "Harvest",
            "emergence_events": 42
        });
        let event = HabitatEvent::BridgeData {
            module_id: "fleet_view".into(),
            tag: "orac_health".into(),
            data,
        };
        assert!(f.handle_event(&event));
        assert_eq!(f.orac.ralph_gen, 100);
        assert_eq!(f.orac.ralph_phase, "Harvest");
        assert_eq!(f.orac.emergence_events, 42);
    }

    #[test]
    fn handle_event_pv2_health_updates_state_and_returns_true() {
        let mut f = FleetView::new();
        let data = json!({"status": "healthy", "r": 0.95, "spheres": 3, "tick": 100});
        let event = HabitatEvent::BridgeData {
            module_id: "fleet_view".into(),
            tag: "pv2_health".into(),
            data,
        };
        assert!(f.handle_event(&event));
        assert!((f.pv2.r - 0.95).abs() < f64::EPSILON);
        assert_eq!(f.pv2.spheres, 3);
    }

    #[test]
    fn handle_event_unknown_tag_returns_false_without_mutation() {
        let mut f = FleetView::new();
        let pv2_before = f.pv2.clone();
        let event = HabitatEvent::BridgeData {
            module_id: "fleet_view".into(),
            tag: "orac_blackboard".into(),
            data: json!({"anything": true}),
        };
        assert!(!f.handle_event(&event));
        // State must be untouched on unknown tag.
        assert_eq!(f.pv2.tick, pv2_before.tick);
        assert!((f.pv2.r - pv2_before.r).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_tick_event_is_ignored() {
        let mut f = FleetView::new();
        assert!(!f.handle_event(&HabitatEvent::Tick { tick: 5 }));
    }

    #[test]
    fn handle_event_keypress_is_ignored() {
        let mut f = FleetView::new();
        assert!(!f.handle_event(&HabitatEvent::KeyPress { key: 'q' }));
    }

    #[test]
    fn handle_event_bridge_error_returns_false_without_mutation() {
        let mut f = FleetView::new();
        let event = HabitatEvent::BridgeError {
            module_id: "fleet_view".into(),
            tag: "orac_health".into(),
        };
        assert!(!f.handle_event(&event));
    }

    #[test]
    fn subscriptions_cover_bridge_response_only() {
        let f = FleetView::new();
        let subs = f.subscriptions();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], EventCategory::BridgeResponse);
    }

    #[test]
    fn data_sources_expose_two_health_endpoints_with_configured_urls() {
        let cfg = make_config();
        let mut f = FleetView::new();
        f.init(&cfg);
        let ds = f.data_sources();
        assert_eq!(ds.len(), 2);
        assert!(ds[0].url.contains("orac.test:9133/health"));
        assert!(ds[1].url.contains("pv2.test:9132/health"));
        assert_eq!(ds[0].tag, "orac_health");
        assert_eq!(ds[1].tag, "pv2_health");
    }

    #[test]
    fn data_sources_interval_matches_configured_health_poll() {
        let cfg = make_config();
        let mut f = FleetView::new();
        f.init(&cfg);
        let ds = f.data_sources();
        for src in &ds {
            assert!((src.interval_secs - 7.0).abs() < f64::EPSILON);
            assert_eq!(src.module_id, "fleet_view");
        }
    }

    #[test]
    fn serialize_restore_roundtrip_preserves_orac_and_pv2_state() {
        let mut f = FleetView::new();
        f.orac.ralph_gen = 42;
        f.orac.ralph_phase = "Learn".into();
        f.pv2.r = 0.73;
        f.pv2.spheres = 4;
        let state = f.serialize_state().expect("serialize succeeds");

        let mut f2 = FleetView::new();
        f2.restore_state(&state);
        assert_eq!(f2.orac.ralph_gen, 42);
        assert_eq!(f2.orac.ralph_phase, "Learn");
        assert!((f2.pv2.r - 0.73).abs() < f64::EPSILON);
        assert_eq!(f2.pv2.spheres, 4);
    }

    #[test]
    fn restore_state_gracefully_ignores_malformed_input() {
        // Restoring must not panic on garbage — defensive for state_cache corruption.
        let mut f = FleetView::new();
        let pre = f.orac.ralph_gen;
        f.restore_state("{ not json");
        assert_eq!(f.orac.ralph_gen, pre);
    }

    #[test]
    fn render_with_zero_spheres_reports_stdp_idle() {
        let f = FleetView::new();
        let lines = f.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("STDP: idle"));
    }

    #[test]
    fn render_with_single_sphere_reports_warming_up() {
        let mut f = FleetView::new();
        f.pv2.spheres = 1;
        let lines = f.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("warming up"));
    }

    #[test]
    fn render_with_multi_sphere_and_ltd_zero_shows_inf_ratio_not_nan() {
        // Division-by-zero guard: hebbian_ltd_total = 0 must render as "inf" not NaN or panic.
        let mut f = FleetView::new();
        f.pv2.spheres = 3;
        f.orac.hebbian_ltp_total = 100;
        f.orac.hebbian_ltd_total = 0;
        let lines = f.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("ratio="));
        assert!(joined.contains("inf"));
    }

    #[test]
    fn id_and_version_match_declared_module_metadata() {
        let f = FleetView::new();
        assert_eq!(f.id(), "fleet_view");
        assert_eq!(f.version(), "0.1.0");
    }
}
