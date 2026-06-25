use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use habitat_core::responses::Proposals;

pub struct NaPanel {
    proposals: Proposals,
    pv2_url: String,
    orac_url: String,
    poll_secs: f64,
    show_consent: bool,
    show_attribution: bool,
}

impl NaPanel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            proposals: Proposals::default(),
            pv2_url: "http://127.0.0.1:8132".into(),
            orac_url: "http://127.0.0.1:8133".into(),
            poll_secs: 10.0,
            show_consent: true,
            show_attribution: true,
        }
    }
}

impl Default for NaPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for NaPanel {
    fn id(&self) -> &'static str {
        "na_panel"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.pv2_url.clone_from(&config.pv2_url);
        self.orac_url.clone_from(&config.orac_url);
        self.poll_secs = config.governance_poll;
        self.show_consent = config.show_consent_states;
        self.show_attribution = config.show_attribution;
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::BridgeData { tag, data, .. } => {
                if tag == "pv2_proposals" {
                    if let Ok(p) = serde_json::from_value(data.clone()) {
                        self.proposals = p;
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn render(&self, _rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);
        let mut lines = Vec::new();

        let active_count = self
            .proposals
            .proposals
            .iter()
            .filter(|p| p.status != "Expired" && p.status != "Applied")
            .count();

        lines.push(RenderLine::new(format!(
            " {B}{MAG}NA PANEL{R}  {D}sovereignty · consent · governance{R}",
        )));
        lines.push(RenderLine::separator(w));

        lines.push(RenderLine::new(format!(
            " {D}NA-P-1 consent · NA-P-4 k_mod · NA-P-9 attribution · NA-P-13 forget · NA-P-15 governance{R}",
        )));

        if self.proposals.proposals.is_empty() {
            lines.push(RenderLine::new(format!(
                " {D}Governance: no proposals on file{R}",
            )));
        } else {
            lines.push(RenderLine::new(format!(
                " {D}Governance{R} {B}{}{R} total · {CYN}{}{R} active",
                self.proposals.proposals.len(),
                active_count,
            )));

            for prop in self.proposals.proposals.iter().take(5) {
                let status_color = match prop.status.as_str() {
                    "Applied" => GRN,
                    "Expired" => D,
                    _ => YEL,
                };
                lines.push(RenderLine::new(format!(
                    " {status_color}{:<8}{R} {B}{:<18}{R} {:.3}\u{2192}{:.3} {D}by{R} {:<16} {D}votes={B}{}{R}",
                    truncate(&prop.status, 8),
                    truncate(&prop.parameter, 18),
                    prop.current_value,
                    prop.proposed_value,
                    truncate(&prop.proposer, 16),
                    prop.votes,
                )));
            }

            if self.proposals.proposals.len() > 5 {
                lines.push(RenderLine::new(format!(
                    " {D}...{} more proposals{R}",
                    self.proposals.proposals.len() - 5,
                )));
            }
        }

        lines.push(RenderLine::new(format!(
            " {D}Sovereignty: each sphere controls its own data manifest{R}",
        )));
        lines.push(RenderLine::new(format!(
            " {D}Forget: use cmd_pipe 'forget <sphere_id>' to trigger cascade{R}",
        )));

        lines
    }

    fn serialize_state(&self) -> Option<String> {
        None
    }
    fn restore_state(&mut self, _state: &str) {}

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::BridgeResponse]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        vec![DataSource {
            url: format!("{}/field/proposals", self.pv2_url),
            interval_secs: self.poll_secs,
            tag: "pv2_proposals".into(),
            module_id: self.id().into(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn config_with_governance(secs: &str, show_consent: &str, show_attrib: &str) -> ModuleConfig {
        let mut m = BTreeMap::new();
        m.insert("pv2_url".into(), "http://pv2.test:8132".into());
        m.insert("governance_poll".into(), secs.into());
        m.insert("show_consent_states".into(), show_consent.into());
        m.insert("show_attribution".into(), show_attrib.into());
        ModuleConfig::from_btree(&m).0
    }

    #[test]
    fn new_defaults_show_consent_and_attribution_preserving_na_contract() {
        // NA-P-9 (attribution) + NA-P-1 (consent) default ON — silent fallback
        // must not strip sphere sovereignty from the render.
        let p = NaPanel::new();
        assert!(p.show_consent);
        assert!(p.show_attribution);
    }

    #[test]
    fn init_binds_governance_poll_not_health_or_coherence() {
        // Regression: na_panel reads governance_poll; if bound to health_poll the
        // NA governance panel would over-refresh on busy habitats.
        let cfg = config_with_governance("15.0", "true", "true");
        let mut p = NaPanel::new();
        p.init(&cfg);
        assert!((p.poll_secs - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn init_respects_show_consent_false_override() {
        let cfg = config_with_governance("10.0", "false", "true");
        let mut p = NaPanel::new();
        p.init(&cfg);
        assert!(!p.show_consent);
    }

    #[test]
    fn init_respects_show_attribution_false_override() {
        // NA-Z7 — user explicitly opts out of attribution. Must be honoured.
        let cfg = config_with_governance("10.0", "true", "false");
        let mut p = NaPanel::new();
        p.init(&cfg);
        assert!(!p.show_attribution);
    }

    #[test]
    fn handle_event_pv2_proposals_populates_list_and_returns_true() {
        let mut p = NaPanel::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "na_panel".into(),
            tag: "pv2_proposals".into(),
            data: json!({"proposals": [
                {"id": "p1", "parameter": "thermal_target", "proposed_value": 0.6, "current_value": 0.5,
                 "proposer": "watcher", "status": "pending", "votes": 3, "reason": "cool drift"}
            ]}),
        };
        assert!(p.handle_event(&ev));
        assert_eq!(p.proposals.proposals.len(), 1);
        assert_eq!(p.proposals.proposals[0].id, "p1");
    }

    #[test]
    fn handle_event_unknown_tag_returns_false_without_mutation() {
        let mut p = NaPanel::new();
        let before = p.proposals.proposals.len();
        let ev = HabitatEvent::BridgeData {
            module_id: "na_panel".into(),
            tag: "orac_health".into(),
            data: json!({}),
        };
        assert!(!p.handle_event(&ev));
        assert_eq!(p.proposals.proposals.len(), before);
    }

    #[test]
    fn handle_event_tick_and_keypress_are_ignored() {
        let mut p = NaPanel::new();
        assert!(!p.handle_event(&HabitatEvent::Tick { tick: 1 }));
        assert!(!p.handle_event(&HabitatEvent::KeyPress { key: 'x' }));
    }

    #[test]
    fn render_empty_proposals_shows_no_proposals_message() {
        let p = NaPanel::new();
        let lines = p.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("no proposals on file"));
    }

    #[test]
    fn render_shows_na_protocol_markers_on_every_render() {
        // NA-P-1/4/9/13/15 markers are the panel's sovereignty contract; must appear
        // even when proposal list is empty.
        let p = NaPanel::new();
        let lines = p.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("NA-P-1"));
        assert!(joined.contains("NA-P-13 forget"));
    }

    #[test]
    fn render_truncates_proposal_display_to_five_entries_with_more_indicator() {
        let mut p = NaPanel::new();
        let mut ps = Vec::new();
        for i in 0..7 {
            ps.push(json!({
                "id": format!("p{i}"),
                "parameter": "thermal_target",
                "proposed_value": 0.6,
                "current_value": 0.5,
                "proposer": "watcher",
                "status": "pending",
                "votes": 1,
                "reason": "test"
            }));
        }
        let ev = HabitatEvent::BridgeData {
            module_id: "na_panel".into(),
            tag: "pv2_proposals".into(),
            data: json!({"proposals": ps}),
        };
        assert!(p.handle_event(&ev));
        let lines = p.render(40, 100);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // 5 displayed + "...2 more proposals"
        assert!(joined.contains("2 more"));
    }

    #[test]
    fn data_sources_expose_single_field_proposals_endpoint() {
        let cfg = config_with_governance("10.0", "true", "true");
        let mut p = NaPanel::new();
        p.init(&cfg);
        let ds = p.data_sources();
        assert_eq!(ds.len(), 1);
        assert!(ds[0].url.ends_with("/field/proposals"));
        assert_eq!(ds[0].tag, "pv2_proposals");
    }

    #[test]
    fn id_version_and_subscriptions_match_module_metadata() {
        let p = NaPanel::new();
        assert_eq!(p.id(), "na_panel");
        assert_eq!(p.version(), "0.1.0");
        assert_eq!(p.subscriptions(), vec![EventCategory::BridgeResponse]);
    }
}
