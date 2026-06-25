use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use habitat_core::responses::{OracBridges, OracThermal};
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq)]
enum ServiceState {
    Unknown,
    Up,
    Down,
}

pub struct BridgeHealth {
    bridges: OracBridges,
    thermal: OracThermal,
    service_status: HashMap<u16, ServiceState>,
    orac_url: String,
    nerve_url: String,
    poll_secs: f64,
    nerve_up: bool,
}

const SERVICES: &[(u16, &str)] = &[
    (8082, "V3"),
    (8083, "Nerve"),
    (8085, "TL"),
    (8092, "SX"),
    (8111, "V8"),
    (8120, "VMS"),
    (8125, "POVM"),
    (8130, "RM"),
    (8132, "PV2"),
    (8133, "ORAC"),
    (8140, "Inj"),
    (8142, "WFE"),
    (8144, "Arch"),
    (8180, "ME"),
    (8200, "LCM"),
    (10002, "PSw"),
];

impl BridgeHealth {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bridges: OracBridges::default(),
            thermal: OracThermal::default(),
            service_status: HashMap::new(),
            orac_url: "http://127.0.0.1:8133".into(),
            nerve_url: "http://127.0.0.1:8083".into(),
            poll_secs: 5.0,
            nerve_up: false,
        }
    }
}

impl Default for BridgeHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for BridgeHealth {
    fn id(&self) -> &'static str {
        "bridge_health"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.orac_url.clone_from(&config.orac_url);
        self.nerve_url.clone_from(&config.nerve_url);
        self.poll_secs = config.health_poll;
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::BridgeData { tag, data, .. } => {
                match tag.as_str() {
                    "orac_bridges" => {
                        if let Ok(b) = serde_json::from_value(data.clone()) {
                            self.bridges = b;
                            return true;
                        }
                    }
                    "orac_thermal" => {
                        if let Ok(t) = serde_json::from_value(data.clone()) {
                            self.thermal = t;
                            return true;
                        }
                    }
                    t if t.starts_with("svc_") => {
                        if let Some(port_str) = t.strip_prefix("svc_") {
                            if let Ok(port) = port_str.parse::<u16>() {
                                self.service_status.insert(port, ServiceState::Up);
                                return true;
                            }
                        }
                    }
                    "nerve_health" => {
                        self.nerve_up = true;
                        self.service_status.insert(8083, ServiceState::Up);
                        return true;
                    }
                    _ => {}
                }
                false
            }
            HabitatEvent::BridgeError { tag, .. } => {
                if let Some(port_str) = tag.strip_prefix("svc_") {
                    if let Ok(port) = port_str.parse::<u16>() {
                        self.service_status.insert(port, ServiceState::Down);
                        return true;
                    }
                }
                if tag == "nerve_health" {
                    self.nerve_up = false;
                    self.service_status.insert(8083, ServiceState::Down);
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
            " {B}{CYN}BRIDGES{R}  {GRN}{}{R}/{D}{}{R}/{RED}{}{R} {D}(closed/half/open){R}",
            self.bridges.breakers_closed,
            self.bridges.breakers_half_open,
            self.bridges.breakers_open,
        )));
        lines.push(RenderLine::separator(w));

        let (temp_color, temp_label) = thermal_band(self.thermal.temperature, self.thermal.target);
        lines.push(RenderLine::new(format!(
            " {D}Thermal{R} {temp_color}[{temp_label}]{R} T={B}{:.3}{R} target={D}{:.1}{R} PID={D}{:.3}{R}",
            self.thermal.temperature,
            self.thermal.target,
            self.thermal.pid_output,
        )));

        let mut svc_line = String::from(" ");
        for &(port, name) in SERVICES {
            let state = self
                .service_status
                .get(&port)
                .copied()
                .unwrap_or(ServiceState::Unknown);
            let icon = match state {
                ServiceState::Up => format!("{GRN}{ICON_UP}{R}"),
                ServiceState::Down => format!("{RED}{ICON_CROSS}{R}"),
                ServiceState::Unknown => format!("{D}?{R}"),
            };
            let _ = write!(svc_line, "{icon}{D}{name}{R} ");
        }
        lines.push(RenderLine::new(svc_line));

        let healthy = self
            .service_status
            .values()
            .filter(|v| **v == ServiceState::Up)
            .count();
        let known = self
            .service_status
            .values()
            .filter(|v| **v != ServiceState::Unknown)
            .count();
        let total = SERVICES.len();
        let (hc, hl) = if known < total / 2 {
            (D, "PROBING")
        } else if healthy == known && known == total {
            (GRN, "ALL UP")
        } else if healthy >= total - 2 {
            (YEL, "DEGRADED")
        } else {
            (RED, "CRITICAL")
        };
        lines.push(RenderLine::new(format!(
            " {hc}{hl}{R} {B}{healthy}{R}/{total} up ({known} probed)  {D}IPC{R}={B}{}{R}",
            self.bridges.ipc_state,
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
        let mut sources = vec![
            DataSource {
                url: format!("{}/bridges", self.orac_url),
                interval_secs: self.poll_secs,
                tag: "orac_bridges".into(),
                module_id: self.id().into(),
            },
            DataSource {
                url: format!("{}/thermal", self.orac_url),
                interval_secs: self.poll_secs,
                tag: "orac_thermal".into(),
                module_id: self.id().into(),
            },
            DataSource {
                url: format!("{}/health", self.nerve_url),
                interval_secs: self.poll_secs,
                tag: "nerve_health".into(),
                module_id: self.id().into(),
            },
        ];

        let health_paths: &[(u16, &str)] = &[
            (8082, "/health"),
            (8083, "/health"),
            (8085, "/health"),
            (8092, "/health"),
            (8111, "/health"),
            (8120, "/health"),
            (8125, "/health"),
            (8130, "/health"),
            (8132, "/health"),
            (8133, "/health"),
            (8140, "/health"),
            (8142, "/health"),
            (8144, "/health"),
            (8180, "/api/health"),
            (8200, "/health"),
            (10002, "/health"),
        ];
        for &(port, path) in health_paths {
            sources.push(DataSource {
                url: format!("http://127.0.0.1:{port}{path}"),
                interval_secs: self.poll_secs * 6.0,
                tag: format!("svc_{port}"),
                module_id: self.id().into(),
            });
        }
        sources
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn make_config_with_nerve(nerve: &str) -> ModuleConfig {
        let mut m = BTreeMap::new();
        m.insert("orac_url".into(), "http://orac.test:8133".into());
        m.insert("nerve_url".into(), nerve.into());
        m.insert("health_poll".into(), "5.0".into());
        ModuleConfig::from_btree(&m).0
    }

    #[test]
    fn new_starts_with_empty_service_status_and_nerve_down() {
        let b = BridgeHealth::new();
        assert!(b.service_status.is_empty());
        assert!(!b.nerve_up);
    }

    #[test]
    fn init_threads_nerve_url_and_health_poll_config() {
        let cfg = make_config_with_nerve("http://nerve.test:8083");
        let mut b = BridgeHealth::new();
        b.init(&cfg);
        assert_eq!(b.nerve_url, "http://nerve.test:8083");
        assert_eq!(b.orac_url, "http://orac.test:8133");
        assert!((b.poll_secs - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_orac_bridges_populates_breaker_counters_and_returns_true() {
        let mut b = BridgeHealth::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "bridge_health".into(),
            tag: "orac_bridges".into(),
            data: json!({
                "breakers_closed": 5,
                "breakers_half_open": 1,
                "breakers_open": 2,
                "ipc_state": "subscribed"
            }),
        };
        assert!(b.handle_event(&ev));
        assert_eq!(b.bridges.breakers_closed, 5);
        assert_eq!(b.bridges.breakers_half_open, 1);
        assert_eq!(b.bridges.breakers_open, 2);
        assert_eq!(b.bridges.ipc_state, "subscribed");
    }

    #[test]
    fn handle_event_orac_thermal_populates_thermal_and_returns_true() {
        let mut b = BridgeHealth::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "bridge_health".into(),
            tag: "orac_thermal".into(),
            data: json!({"temperature": 0.55, "target": 0.5, "pid_output": 0.05}),
        };
        assert!(b.handle_event(&ev));
        assert!((b.thermal.temperature - 0.55).abs() < f64::EPSILON);
        assert!((b.thermal.target - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn handle_event_svc_port_marks_service_up_via_success() {
        let mut b = BridgeHealth::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "bridge_health".into(),
            tag: "svc_8092".into(),
            data: json!({}),
        };
        assert!(b.handle_event(&ev));
        assert_eq!(b.service_status.get(&8092), Some(&ServiceState::Up));
    }

    #[test]
    fn handle_event_bridge_error_on_svc_port_marks_service_down() {
        let mut b = BridgeHealth::new();
        b.service_status.insert(8092, ServiceState::Up);
        let ev = HabitatEvent::BridgeError {
            module_id: "bridge_health".into(),
            tag: "svc_8092".into(),
        };
        assert!(b.handle_event(&ev));
        assert_eq!(b.service_status.get(&8092), Some(&ServiceState::Down));
    }

    #[test]
    fn handle_event_nerve_health_toggles_nerve_up_flag() {
        let mut b = BridgeHealth::new();
        let ok = HabitatEvent::BridgeData {
            module_id: "bridge_health".into(),
            tag: "nerve_health".into(),
            data: json!("ok"),
        };
        assert!(b.handle_event(&ok));
        assert!(b.nerve_up);
        assert_eq!(b.service_status.get(&8083), Some(&ServiceState::Up));

        let err = HabitatEvent::BridgeError {
            module_id: "bridge_health".into(),
            tag: "nerve_health".into(),
        };
        assert!(b.handle_event(&err));
        assert!(!b.nerve_up);
        assert_eq!(b.service_status.get(&8083), Some(&ServiceState::Down));
    }

    #[test]
    fn handle_event_malformed_svc_tag_does_not_panic_and_returns_false() {
        // "svc_not-a-number" must not panic or corrupt state — parse-fail → ignore.
        let mut b = BridgeHealth::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "bridge_health".into(),
            tag: "svc_notanumber".into(),
            data: json!({}),
        };
        assert!(!b.handle_event(&ev));
        assert!(b.service_status.is_empty());
    }

    #[test]
    fn handle_event_unknown_tag_returns_false_and_does_not_update_state() {
        let mut b = BridgeHealth::new();
        let ev = HabitatEvent::BridgeData {
            module_id: "bridge_health".into(),
            tag: "something_else".into(),
            data: json!({"x": 1}),
        };
        assert!(!b.handle_event(&ev));
        assert_eq!(b.bridges.breakers_closed, 0);
    }

    #[test]
    fn data_sources_include_16_per_service_probes_plus_3_orac_probes() {
        // 16 services (V3 Nerve TL SX V8 VMS POVM RM PV2 ORAC Inj WFE Arch ME LCM PSw)
        // — Arch (:8144 The Architect / DRAUGHTWRIGHT control plane) added S1008073, the 16th service.
        // + 3 ORAC-specific probes (bridges, thermal, nerve_health) = 19.
        // LCM @ :8200 added 2026-05-25 per loop-engine-v2 M1 /health advancement
        // (Wave-18; earlier Wave-16 commit lost to a linter race that landed WFE
        // at the LCM slot — see loop-engine-v2/ai_docs/M1_HTTP_HEALTH.md honest
        // residual).
        let mut b = BridgeHealth::new();
        b.init(&make_config_with_nerve("http://127.0.0.1:8083"));
        let ds = b.data_sources();
        assert_eq!(ds.len(), 19);
    }

    #[test]
    fn data_sources_service_probes_use_health_poll_times_six() {
        // Per-service probes poll at 6× health_poll (30s when health_poll=5s).
        let cfg = make_config_with_nerve("http://127.0.0.1:8083");
        let mut b = BridgeHealth::new();
        b.init(&cfg);
        let ds = b.data_sources();
        let svc_sources: Vec<_> = ds.iter().filter(|s| s.tag.starts_with("svc_")).collect();
        for s in svc_sources {
            assert!(
                (s.interval_secs - 30.0).abs() < f64::EPSILON,
                "expected 30s per-service interval, got {}",
                s.interval_secs
            );
        }
    }

    #[test]
    fn data_sources_synthex_v2_uses_health_and_me_uses_api_health() {
        let cfg = make_config_with_nerve("http://127.0.0.1:8083");
        let mut b = BridgeHealth::new();
        b.init(&cfg);
        let ds = b.data_sources();
        let synthex = ds
            .iter()
            .find(|s| s.tag == "svc_8092")
            .expect("synthex v2 source");
        let me = ds.iter().find(|s| s.tag == "svc_8180").expect("me source");
        assert!(
            synthex.url.ends_with(":8092/health"),
            "synthex v2 uses /health"
        );
        assert!(
            me.url.ends_with(":8180/api/health"),
            "me v2 uses /api/health"
        );
    }

    #[test]
    fn subscriptions_include_bridge_response_only() {
        let b = BridgeHealth::new();
        assert_eq!(b.subscriptions(), vec![EventCategory::BridgeResponse]);
    }

    #[test]
    fn render_empty_state_shows_probing_status_header() {
        // Zero probed services → PROBING (dim), not CRITICAL (red).
        let b = BridgeHealth::new();
        let lines = b.render(20, 80);
        let joined: String = lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("PROBING"));
    }

    #[test]
    fn id_and_version_match_module_metadata() {
        let b = BridgeHealth::new();
        assert_eq!(b.id(), "bridge_health");
        assert_eq!(b.version(), "0.1.0");
    }
}
