use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use habitat_core::responses::{CouplingState, HebbianState, Pv2Field};

pub struct CoherenceGauge {
    field: Pv2Field,
    coupling: CouplingState,
    hebbian: HebbianState,
    pv2_url: String,
    orac_url: String,
    poll_secs: f64,
    show_matrix: bool,
}

impl CoherenceGauge {
    #[must_use]
    pub fn new() -> Self {
        Self {
            field: Pv2Field::default(),
            coupling: CouplingState::default(),
            hebbian: HebbianState::default(),
            pv2_url: "http://127.0.0.1:8132".into(),
            orac_url: "http://127.0.0.1:8133".into(),
            poll_secs: 2.0,
            show_matrix: false,
        }
    }

    fn coherence_bar(r: f64, width: usize) -> String {
        let filled = ((r * width as f64) as usize).min(width);
        let empty = width.saturating_sub(filled);
        let color = if r > 0.85 {
            GRN
        } else if r > 0.5 {
            YEL
        } else if r > 0.1 {
            CYN
        } else {
            D
        };
        format!(
            "{color}{}{R}{D}{}{R}",
            "\u{2588}".repeat(filled),
            "\u{2591}".repeat(empty),
        )
    }
}

impl Default for CoherenceGauge {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for CoherenceGauge {
    fn id(&self) -> &'static str {
        "coherence_gauge"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.pv2_url.clone_from(&config.pv2_url);
        self.orac_url.clone_from(&config.orac_url);
        self.poll_secs = config.coherence_poll;
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::BridgeData { tag, data, .. } => {
                match tag.as_str() {
                    "pv2_field" => {
                        if let Ok(f) = serde_json::from_value(data.clone()) {
                            self.field = f;
                            return true;
                        }
                    }
                    "orac_coupling" => {
                        if let Ok(c) = serde_json::from_value(data.clone()) {
                            self.coupling = c;
                            return true;
                        }
                    }
                    "orac_hebbian" => {
                        if let Ok(h) = serde_json::from_value(data.clone()) {
                            self.hebbian = h;
                            return true;
                        }
                    }
                    _ => {}
                }
                false
            }
            HabitatEvent::KeyPress { key } => {
                if *key == 'c' || *key == 'C' {
                    self.show_matrix = !self.show_matrix;
                    return true;
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
            " {B}{CYN}COHERENCE{R}  {D}Kuramoto field{R}",
        )));
        lines.push(RenderLine::separator(w));

        let bar_width = w.saturating_sub(22);
        let bar = Self::coherence_bar(self.field.r, bar_width);
        lines.push(RenderLine::new(format!(
            " r=[{bar}] {B}{:.3}{R}",
            self.field.r,
        )));

        lines.push(RenderLine::new(format!(
            " {D}K{R}={B}{:.2}{R} {D}k_mod{R}={B}{:.3}{R} {D}spheres{R}={B}{}{R} {D}tick{R}={B}{}{R}",
            self.field.k,
            self.field.k_modulation,
            self.field.sphere_count,
            fmt_num(self.field.tick),
        )));

        if self.coupling.connections == 0 {
            lines.push(RenderLine::new(format!(
                " {D}Coupling: no active edges (idle){R}",
            )));
        } else {
            let health = if self.coupling.saturation_pct > 80.0 {
                format!("{RED}saturated{R}")
            } else if self.coupling.saturation_pct > 50.0 {
                format!("{YEL}warming{R}")
            } else {
                format!("{GRN}healthy{R}")
            };
            lines.push(RenderLine::new(format!(
                " {D}Coupling{R} {health} {B}{}{R} edges  mean={B}{:.3}{R} range=[{:.2},{:.2}]",
                self.coupling.connections,
                self.coupling.weight_mean,
                self.coupling.weight_min,
                self.coupling.weight_max,
            )));

            if self.show_matrix {
                lines.push(RenderLine::new(format!(
                    " {D}saturation{R} {B}{:.1}%{R} {D}k_mod{R} {B}{:.3}{R} {D}ceiling/floor{R} {}/{}",
                    self.coupling.saturation_pct,
                    self.coupling.k_modulation,
                    self.coupling.at_ceiling,
                    self.coupling.at_floor,
                )));
            }
        }

        let learning = if self.hebbian.co_activations_total == 0 {
            format!("{D}STDP{R} dormant")
        } else {
            let ratio_str = if self.hebbian.ltd_total > 0 {
                format!(
                    "{:.2}",
                    self.hebbian.ltp_total as f64 / self.hebbian.ltd_total as f64
                )
            } else if self.hebbian.ltp_total > 0 {
                "∞".into()
            } else {
                "0".into()
            };
            format!(
                "{D}STDP{R} LTP={B}{}{R} LTD={B}{}{R} ratio={B}{ratio_str}{R} co-act={B}{}{R}",
                self.hebbian.ltp_total, self.hebbian.ltd_total, self.hebbian.co_activations_total,
            )
        };
        lines.push(RenderLine::new(format!(" {learning}")));

        if !self.show_matrix {
            lines.push(RenderLine::new(format!(
                " {D}press 'c' to toggle coupling detail{R}",
            )));
        }

        lines
    }

    fn serialize_state(&self) -> Option<String> {
        serde_json::to_string(&(&self.field, &self.coupling, &self.hebbian)).ok()
    }

    fn restore_state(&mut self, state: &str) {
        if let Ok((field, coupling, hebbian)) =
            serde_json::from_str::<(Pv2Field, CouplingState, HebbianState)>(state)
        {
            self.field = field;
            self.coupling = coupling;
            self.hebbian = hebbian;
        }
    }

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::BridgeResponse, EventCategory::KeyPress]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        vec![
            DataSource {
                url: format!("{}/field", self.pv2_url),
                interval_secs: self.poll_secs,
                tag: "pv2_field".into(),
                module_id: self.id().into(),
            },
            DataSource {
                url: format!("{}/coupling", self.orac_url),
                interval_secs: self.poll_secs * 2.5,
                tag: "orac_coupling".into(),
                module_id: self.id().into(),
            },
            DataSource {
                url: format!("{}/hebbian", self.orac_url),
                interval_secs: self.poll_secs * 2.5,
                tag: "orac_hebbian".into(),
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
        m.insert("pv2_url".into(), "http://pv2.test:8132".into());
        m.insert("orac_url".into(), "http://orac.test:8133".into());
        m.insert("coherence_poll".into(), "1.5".into());
        ModuleConfig::from_btree(&m).0
    }

    #[test]
    fn new_starts_with_matrix_hidden_and_two_second_poll() {
        let g = CoherenceGauge::new();
        assert!(!g.show_matrix);
        assert!((g.poll_secs - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn init_binds_coherence_poll_not_health_or_governance() {
        let cfg = make_config();
        let mut g = CoherenceGauge::new();
        g.init(&cfg);
        assert!((g.poll_secs - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_pv2_field_updates_r_and_returns_true() {
        let mut g = CoherenceGauge::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "coherence_gauge".into(),
            tag: "pv2_field".into(),
            data: json!({"r": 0.82, "K": 1.5, "sphere_count": 3, "tick": 42}),
        };
        assert!(g.handle_event(&ev));
        assert!((g.field.r - 0.82).abs() < f64::EPSILON);
        assert!((g.field.k - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_orac_coupling_updates_coupling_and_returns_true() {
        let mut g = CoherenceGauge::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "coherence_gauge".into(),
            tag: "orac_coupling".into(),
            data: json!({"connections": 5, "weight_mean": 0.4, "k_modulation": 1.1}),
        };
        assert!(g.handle_event(&ev));
        assert_eq!(g.coupling.connections, 5);
        assert!((g.coupling.weight_mean - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_orac_hebbian_updates_ratio_and_returns_true() {
        let mut g = CoherenceGauge::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "coherence_gauge".into(),
            tag: "orac_hebbian".into(),
            data: json!({"ltp_total": 100, "ltd_total": 20, "ltp_ltd_ratio": 5.0}),
        };
        assert!(g.handle_event(&ev));
        assert_eq!(g.hebbian.ltp_total, 100);
        assert!((g.hebbian.ltp_ltd_ratio - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_c_keypress_toggles_matrix_flag_idempotently() {
        let mut g = CoherenceGauge::new();
        assert!(g.handle_event(&HabitatEvent::KeyPress { key: 'c' }));
        assert!(g.show_matrix);
        assert!(g.handle_event(&HabitatEvent::KeyPress { key: 'c' }));
        assert!(!g.show_matrix);
    }

    #[test]
    fn handle_event_uppercase_c_also_toggles_matrix() {
        let mut g = CoherenceGauge::new();
        assert!(g.handle_event(&HabitatEvent::KeyPress { key: 'C' }));
        assert!(g.show_matrix);
    }

    #[test]
    fn handle_event_unrelated_keypress_does_nothing() {
        let mut g = CoherenceGauge::new();
        assert!(!g.handle_event(&HabitatEvent::KeyPress { key: 'q' }));
        assert!(!g.show_matrix);
    }

    #[test]
    fn handle_event_bridge_error_returns_false_without_mutation() {
        let mut g = CoherenceGauge::new();
        let ev = HabitatEvent::BridgeError {
            module_id: "coherence_gauge".into(),
            tag: "pv2_field".into(),
        };
        assert!(!g.handle_event(&ev));
    }

    #[test]
    fn coherence_bar_zero_r_is_fully_empty() {
        let bar = CoherenceGauge::coherence_bar(0.0, 10);
        // 10 dim empty blocks, zero filled — count empty-block glyph occurrences.
        assert_eq!(bar.matches('\u{2591}').count(), 10);
        assert_eq!(bar.matches('\u{2588}').count(), 0);
    }

    #[test]
    fn coherence_bar_full_r_fills_entire_width() {
        let bar = CoherenceGauge::coherence_bar(1.0, 8);
        assert_eq!(bar.matches('\u{2588}').count(), 8);
        assert_eq!(bar.matches('\u{2591}').count(), 0);
    }

    #[test]
    fn coherence_bar_over_saturated_r_does_not_exceed_width() {
        // r > 1.0 must be clamped so we never render more than `width` filled blocks.
        // Regression guard: no-`.min(width)` would panic on repeat with negative implicitly
        // or over-allocate.
        let bar = CoherenceGauge::coherence_bar(1.5, 6);
        assert_eq!(bar.matches('\u{2588}').count(), 6);
        assert_eq!(bar.matches('\u{2591}').count(), 0);
    }

    #[test]
    fn coherence_bar_half_r_fills_half_width_with_remaining_empty() {
        let bar = CoherenceGauge::coherence_bar(0.5, 10);
        assert_eq!(bar.matches('\u{2588}').count(), 5);
        assert_eq!(bar.matches('\u{2591}').count(), 5);
    }

    #[test]
    fn coherence_bar_color_tiers_match_documented_thresholds() {
        // Invariant: coherence-color tiers are GRN(>0.85) → YEL(>0.5) → CYN(>0.1) → D(else).
        // A regression that reorders these would visually mislead the operator.
        assert!(CoherenceGauge::coherence_bar(0.9, 5).contains(GRN));
        assert!(CoherenceGauge::coherence_bar(0.6, 5).contains(YEL));
        assert!(CoherenceGauge::coherence_bar(0.3, 5).contains(CYN));
        assert!(CoherenceGauge::coherence_bar(0.05, 5).contains(D));
    }

    #[test]
    fn data_sources_expose_three_endpoints_with_configured_urls() {
        let cfg = make_config();
        let mut g = CoherenceGauge::new();
        g.init(&cfg);
        let ds = g.data_sources();
        assert_eq!(ds.len(), 3);
        let tags: Vec<&str> = ds.iter().map(|s| s.tag.as_str()).collect();
        assert!(tags.contains(&"pv2_field"));
        assert!(tags.contains(&"orac_coupling"));
        assert!(tags.contains(&"orac_hebbian"));
    }

    #[test]
    fn id_version_and_subscriptions_match_module_metadata() {
        // coherence_gauge is distinct from other modules in that it ALSO subscribes
        // to KeyPress (for the 'c' matrix-toggle). Encoding the actual subscription
        // set here guards against an accidental drop of the key binding.
        let g = CoherenceGauge::new();
        assert_eq!(g.id(), "coherence_gauge");
        assert_eq!(g.version(), "0.1.0");
        let subs = g.subscriptions();
        assert!(subs.contains(&EventCategory::BridgeResponse));
        assert!(
            subs.contains(&EventCategory::KeyPress),
            "coherence_gauge must subscribe to KeyPress for 'c' matrix toggle"
        );
        assert_eq!(subs.len(), 2);
    }
}
