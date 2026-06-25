use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use habitat_core::responses::{SessionStats, TokenState};

pub struct SessionTimer {
    stats: SessionStats,
    tokens: TokenState,
    orac_url: String,
    poll_secs: f64,
}

impl SessionTimer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            stats: SessionStats::default(),
            tokens: TokenState::default(),
            orac_url: "http://127.0.0.1:8133".into(),
            poll_secs: 10.0,
        }
    }

    fn format_duration(secs: u64) -> String {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let seconds = secs % 60;
        if hours > 0 {
            format!("{hours}h{mins}m")
        } else if mins > 0 {
            format!("{mins}m{seconds}s")
        } else {
            format!("{seconds}s")
        }
    }
}

impl Default for SessionTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for SessionTimer {
    fn id(&self) -> &'static str {
        "session_timer"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.orac_url.clone_from(&config.orac_url);
        self.poll_secs = config.governance_poll;
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::BridgeData { tag, data, .. } => {
                match tag.as_str() {
                    "orac_session" => {
                        if let Ok(s) = serde_json::from_value(data.clone()) {
                            self.stats = s;
                            return true;
                        }
                    }
                    "orac_tokens" => {
                        if let Ok(t) = serde_json::from_value(data.clone()) {
                            self.tokens = t;
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
            " {B}{CYN}SESSION{R}  {D}uptime · tokens · tool activity{R}",
        )));
        lines.push(RenderLine::separator(w));

        let uptime = Self::format_duration(self.stats.session_elapsed_secs);
        lines.push(RenderLine::new(format!(
            " {D}Uptime{R} {B}{uptime}{R}  {D}Tools{R} {B}{}{R}  {D}Last{R} {B}{}{R}",
            fmt_num(self.stats.tool_count),
            if self.stats.last_tool_name.is_empty() {
                "—"
            } else {
                &self.stats.last_tool_name
            },
        )));

        let gate_icon = if self.stats.last_gate_pass { GRN } else { YEL };
        lines.push(RenderLine::new(format!(
            " {gate_icon}gate{R} {D}last{R}={B}{}{R}",
            if self.stats.last_gate_pass {
                "PASS"
            } else {
                "FAIL"
            },
        )));

        let budget_color = match self.tokens.budget_status.as_str() {
            "Ok" => GRN,
            "Warning" => YEL,
            _ => RED,
        };
        lines.push(RenderLine::new(format!(
            " {D}Budget{R} {budget_color}{}{R} {B}{:.1}{R} remaining  {D}util{R}={B}{:.2}%{R}",
            self.tokens.budget_status,
            self.tokens.budget_remaining,
            self.tokens.utilization * 100.0,
        )));

        lines.push(RenderLine::new(format!(
            " {D}IO{R} in={B}{}{R} out={B}{}{R}  {D}panes{R}={B}{}{R}",
            fmt_num(self.tokens.total_input),
            fmt_num(self.tokens.total_output),
            self.tokens.total_panes,
        )));

        lines
    }

    fn serialize_state(&self) -> Option<String> {
        serde_json::to_string(&(&self.stats, &self.tokens)).ok()
    }

    fn restore_state(&mut self, state: &str) {
        if let Ok((stats, tokens)) = serde_json::from_str::<(SessionStats, TokenState)>(state) {
            self.stats = stats;
            self.tokens = tokens;
        }
    }

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::BridgeResponse]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        vec![
            DataSource {
                url: format!("{}/session-stats", self.orac_url),
                interval_secs: self.poll_secs,
                tag: "orac_session".into(),
                module_id: self.id().into(),
            },
            DataSource {
                url: format!("{}/tokens", self.orac_url),
                interval_secs: self.poll_secs,
                tag: "orac_tokens".into(),
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
        m.insert("orac_url".into(), "http://orac.test:8133".into());
        m.insert("governance_poll".into(), "12.0".into());
        ModuleConfig::from_btree(&m).0
    }

    #[test]
    fn new_starts_with_default_stats_and_ten_second_poll() {
        let t = SessionTimer::new();
        assert!((t.poll_secs - 10.0).abs() < f64::EPSILON);
        assert_eq!(t.stats.tool_count, 0);
    }

    #[test]
    fn init_binds_governance_poll_not_health_or_coherence() {
        let cfg = make_config();
        let mut t = SessionTimer::new();
        t.init(&cfg);
        assert!((t.poll_secs - 12.0).abs() < f64::EPSILON);
        assert_eq!(t.orac_url, "http://orac.test:8133");
    }

    #[test]
    fn format_duration_seconds_only_for_sub_minute() {
        assert_eq!(SessionTimer::format_duration(0), "0s");
        assert_eq!(SessionTimer::format_duration(1), "1s");
        assert_eq!(SessionTimer::format_duration(59), "59s");
    }

    #[test]
    fn format_duration_minutes_plus_seconds_below_one_hour() {
        assert_eq!(SessionTimer::format_duration(60), "1m0s");
        assert_eq!(SessionTimer::format_duration(125), "2m5s");
        assert_eq!(SessionTimer::format_duration(3599), "59m59s");
    }

    #[test]
    fn format_duration_hours_plus_minutes_above_one_hour_drops_seconds() {
        // Hours + minutes — seconds intentionally dropped for display compactness.
        assert_eq!(SessionTimer::format_duration(3600), "1h0m");
        assert_eq!(SessionTimer::format_duration(7260), "2h1m");
        assert_eq!(SessionTimer::format_duration(86_400), "24h0m");
    }

    #[test]
    fn handle_event_orac_session_updates_stats_and_returns_true() {
        let mut t = SessionTimer::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "session_timer".into(),
            tag: "orac_session".into(),
            data: json!({
                "tool_count": 42,
                "session_started_at": 1_700_000_000,
                "session_elapsed_secs": 120,
                "last_tool_name": "bacon",
                "last_gate_pass": true
            }),
        };
        assert!(t.handle_event(&ev));
        assert_eq!(t.stats.tool_count, 42);
        assert_eq!(t.stats.session_elapsed_secs, 120);
        assert_eq!(t.stats.last_tool_name, "bacon");
        assert!(t.stats.last_gate_pass);
    }

    #[test]
    fn handle_event_orac_tokens_updates_budget_and_utilization() {
        let mut t = SessionTimer::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "session_timer".into(),
            tag: "orac_tokens".into(),
            data: json!({
                "total_input": 100_000,
                "total_output": 50_000,
                "budget_remaining": 0.3,
                "budget_status": "Warning",
                "utilization": 0.7
            }),
        };
        assert!(t.handle_event(&ev));
        assert_eq!(t.tokens.total_input, 100_000);
        assert!((t.tokens.utilization - 0.7).abs() < f64::EPSILON);
        assert_eq!(t.tokens.budget_status, "Warning");
    }

    #[test]
    fn handle_event_unknown_tag_returns_false_without_mutation() {
        let mut t = SessionTimer::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "session_timer".into(),
            tag: "orac_blackboard".into(),
            data: json!({}),
        };
        assert!(!t.handle_event(&ev));
    }

    #[test]
    fn handle_event_tick_and_keypress_are_ignored() {
        let mut t = SessionTimer::new();
        assert!(!t.handle_event(&HabitatEvent::Tick { tick: 100 }));
        assert!(!t.handle_event(&HabitatEvent::KeyPress { key: 't' }));
    }

    #[test]
    fn render_shows_em_dash_when_no_tool_has_run_yet() {
        // Empty `last_tool_name` must render as an em-dash, not empty string.
        let t = SessionTimer::new();
        let lines = t.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("—"));
    }

    #[test]
    fn render_shows_pass_when_last_gate_passed() {
        let mut t = SessionTimer::new();
        t.stats.last_gate_pass = true;
        let lines = t.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("PASS"));
    }

    #[test]
    fn data_sources_expose_two_orac_endpoints_with_configured_url() {
        let cfg = make_config();
        let mut t = SessionTimer::new();
        t.init(&cfg);
        let ds = t.data_sources();
        assert_eq!(ds.len(), 2);
        assert!(ds[0].url.contains("orac.test:8133/session-stats"));
        assert!(ds[1].url.contains("orac.test:8133/tokens"));
    }

    #[test]
    fn serialize_restore_roundtrip_preserves_stats_and_tokens() {
        let mut t = SessionTimer::new();
        t.stats.tool_count = 7;
        t.tokens.budget_remaining = 0.42;
        let state = t.serialize_state().expect("serialize succeeds");

        let mut t2 = SessionTimer::new();
        t2.restore_state(&state);
        assert_eq!(t2.stats.tool_count, 7);
        assert!((t2.tokens.budget_remaining - 0.42).abs() < f64::EPSILON);
    }

    #[test]
    fn id_version_and_subscriptions_match_module_metadata() {
        let t = SessionTimer::new();
        assert_eq!(t.id(), "session_timer");
        assert_eq!(t.version(), "0.1.0");
        assert_eq!(t.subscriptions(), vec![EventCategory::BridgeResponse]);
    }
}
