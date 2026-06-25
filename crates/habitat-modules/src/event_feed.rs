use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use habitat_core::responses::{BusEvents, EmergenceState};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const MAX_EVENTS: usize = 100;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FeedEntry {
    pub kind: EntryKind,
    pub timestamp: u64,
    pub event_type: String,
    pub description: String,
    pub confidence: f64,
    pub severity: String,
    pub ttl: u32,
    pub detected_at_tick: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum EntryKind {
    Emergence,
    BusEvent,
    Cascade,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct OracFieldResponse {
    #[serde(default)]
    emergence: OracEmergenceField,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct OracEmergenceField {
    #[serde(default)]
    recent: Vec<RecentEmergenceEvent>,
    #[serde(default)]
    total_detected: u64,
    #[serde(default)]
    by_type: std::collections::HashMap<String, u64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct RecentEmergenceEvent {
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    description: String,
    #[serde(default)]
    detected_at_tick: u64,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    ttl: u32,
    #[serde(default, rename = "type")]
    event_type: String,
}

pub struct EventFeed {
    entries: VecDeque<FeedEntry>,
    seen_ticks: VecDeque<u64>,
    emergence_summary: EmergenceState,
    last_bus_tick: u64,
    orac_url: String,
    pv2_url: String,
    poll_secs: f64,
    scroll_offset: usize,
}

impl EventFeed {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_EVENTS),
            seen_ticks: VecDeque::with_capacity(MAX_EVENTS),
            emergence_summary: EmergenceState::default(),
            last_bus_tick: 0,
            orac_url: "http://127.0.0.1:8133".into(),
            pv2_url: "http://127.0.0.1:8132".into(),
            poll_secs: 5.0,
            scroll_offset: 0,
        }
    }

    fn add_entry(&mut self, entry: FeedEntry) {
        let key = entry.detected_at_tick;
        if key > 0 && self.seen_ticks.contains(&key) {
            return;
        }
        if key > 0 {
            self.seen_ticks.push_back(key);
            while self.seen_ticks.len() > MAX_EVENTS {
                self.seen_ticks.pop_front();
            }
        }
        self.entries.push_front(entry);
        while self.entries.len() > MAX_EVENTS {
            self.entries.pop_back();
        }
    }

    fn confidence_color(c: f64) -> &'static str {
        if c > 0.9 {
            GRN
        } else if c > 0.7 {
            YEL
        } else {
            RED
        }
    }

    fn severity_icon(sev: &str) -> &'static str {
        match sev {
            "high" | "critical" => ICON_CROSS,
            "medium" | "warning" => "\u{26a0}",
            _ => ICON_CHECK,
        }
    }
}

impl Default for EventFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for EventFeed {
    fn id(&self) -> &'static str {
        "event_feed"
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
                    "orac_field_emergence" => {
                        if let Ok(resp) = serde_json::from_value::<OracFieldResponse>(data.clone())
                        {
                            for ev in resp.emergence.recent {
                                if ev.ttl < 60 {
                                    continue;
                                }
                                self.add_entry(FeedEntry {
                                    kind: EntryKind::Emergence,
                                    timestamp: ev.detected_at_tick,
                                    event_type: ev.event_type,
                                    description: ev.description,
                                    confidence: ev.confidence,
                                    severity: ev.severity,
                                    ttl: ev.ttl,
                                    detected_at_tick: ev.detected_at_tick,
                                });
                            }
                            self.emergence_summary.total_detected = resp.emergence.total_detected;
                            self.emergence_summary.by_type = resp.emergence.by_type;
                            return true;
                        }
                    }
                    "pv2_bus_events" => {
                        if let Ok(bus) = serde_json::from_value::<BusEvents>(data.clone()) {
                            let mut added = false;
                            for ev in bus.events {
                                if ev.tick <= self.last_bus_tick {
                                    continue;
                                }
                                if !ev.event_type.starts_with("sphere.")
                                    && !ev.event_type.starts_with("field.chimera")
                                    && !ev.event_type.starts_with("emergence.")
                                {
                                    continue;
                                }
                                self.last_bus_tick = ev.tick;
                                self.add_entry(FeedEntry {
                                    kind: EntryKind::BusEvent,
                                    timestamp: ev.tick,
                                    event_type: ev.event_type,
                                    description: format!("{:.120}", ev.data.to_string()),
                                    confidence: 1.0,
                                    severity: "info".into(),
                                    ttl: 0,
                                    detected_at_tick: ev.tick,
                                });
                                added = true;
                            }
                            return added;
                        }
                    }
                    _ => {}
                }
                false
            }
            HabitatEvent::KeyPress { key } => match key {
                'j' | 'J' => {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                    true
                }
                'k' | 'K' => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    true
                }
                'g' | 'G' => {
                    self.scroll_offset = 0;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn render(&self, rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);
        let mut lines = Vec::new();

        lines.push(RenderLine::new(format!(
            " {B}{CYN}EVENTS{R}  {D}{} total · {} shown{R}",
            fmt_num(self.emergence_summary.total_detected),
            self.entries.len(),
        )));
        lines.push(RenderLine::separator(w));

        let mut top_types: Vec<(&String, &u64)> = self.emergence_summary.by_type.iter().collect();
        top_types.sort_by(|a, b| b.1.cmp(a.1));
        if !top_types.is_empty() {
            let summary: Vec<String> = top_types
                .iter()
                .take(3)
                .map(|(k, v)| format!("{D}{}{R}={B}{}{R}", k, fmt_num(**v)))
                .collect();
            lines.push(RenderLine::new(format!(" {}", summary.join("  "))));
        }

        let visible = rows.saturating_sub(lines.len() + 2).max(4);
        let start = self.scroll_offset.min(self.entries.len().saturating_sub(1));

        for entry in self.entries.iter().skip(start).take(visible) {
            let conf_color = Self::confidence_color(entry.confidence);
            let sev_icon = Self::severity_icon(&entry.severity);
            let kind_tag = match entry.kind {
                EntryKind::Emergence => format!("{MAG}EMG{R}"),
                EntryKind::BusEvent => format!("{CYN}BUS{R}"),
                EntryKind::Cascade => format!("{YEL}CAS{R}"),
            };

            let conf_str = if entry.confidence > 0.0 {
                format!("{conf_color}[{:.0}%]{R}", entry.confidence * 100.0)
            } else {
                format!("{D}[  ]{R}")
            };

            let desc = truncate(&entry.description, w.saturating_sub(35));
            lines.push(RenderLine::new(format!(
                " {sev_icon} {kind_tag} {conf_str} {D}t{}{R} {B}{:<18}{R} {}",
                fmt_num(entry.detected_at_tick),
                truncate(&entry.event_type, 18),
                desc,
            )));
        }

        if self.entries.is_empty() {
            lines.push(RenderLine::new(format!(
                " {D}no events yet — waiting for emergence detections...{R}"
            )));
        }

        lines.push(RenderLine::new(format!(
            " {D}[j/k] scroll  [g] top  showing {}-{} of {}{R}",
            start + 1,
            (start + visible).min(self.entries.len()),
            self.entries.len(),
        )));

        lines
    }

    fn serialize_state(&self) -> Option<String> {
        serde_json::to_string(&self.entries.iter().collect::<Vec<_>>()).ok()
    }

    fn restore_state(&mut self, state: &str) {
        if let Ok(v) = serde_json::from_str::<Vec<FeedEntry>>(state) {
            self.entries = VecDeque::from(v);
        }
    }

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::BridgeResponse, EventCategory::KeyPress]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        vec![
            DataSource {
                url: format!("{}/field", self.orac_url),
                interval_secs: self.poll_secs,
                tag: "orac_field_emergence".into(),
                module_id: self.id().into(),
            },
            DataSource {
                url: format!("{}/bus/events", self.pv2_url),
                interval_secs: self.poll_secs * 2.0,
                tag: "pv2_bus_events".into(),
                module_id: self.id().into(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn emergence_payload(ticks_ttls: &[(u64, u32)]) -> serde_json::Value {
        json!({
            "emergence": {
                "total_detected": ticks_ttls.len(),
                "by_type": {},
                "recent": ticks_ttls.iter().map(|(tick, ttl)| json!({
                    "type": "chimera_formation",
                    "confidence": 0.99,
                    "description": "test",
                    "detected_at_tick": tick,
                    "severity": "medium",
                    "ttl": ttl
                })).collect::<Vec<_>>()
            }
        })
    }

    #[test]
    fn add_entry_dedupes_by_detected_tick() {
        let mut f = EventFeed::new();
        let mk = |tick: u64| FeedEntry {
            kind: EntryKind::Emergence,
            timestamp: tick,
            event_type: "x".into(),
            description: String::new(),
            confidence: 1.0,
            severity: String::new(),
            ttl: 600,
            detected_at_tick: tick,
        };
        f.add_entry(mk(42));
        f.add_entry(mk(42));
        assert_eq!(f.entries.len(), 1, "same-tick entry must be deduped");

        f.add_entry(mk(43));
        assert_eq!(f.entries.len(), 2);
    }

    #[test]
    fn handle_event_orac_field_skips_events_with_ttl_under_60() {
        // NA S5: low-TTL events are transient; plugin must not surface them as
        // persistent journal items. 30s TTL = skipped. 120s TTL = kept.
        let mut f = EventFeed::new();
        let payload = emergence_payload(&[(100, 30), (101, 120)]);
        let ev = HabitatEvent::BridgeData {
            module_id: "event_feed".into(),
            tag: "orac_field_emergence".into(),
            data: payload,
        };
        let handled = f.handle_event(&ev);
        assert!(handled);
        assert_eq!(
            f.entries.len(),
            1,
            "low-TTL event must not land in the feed"
        );
        assert_eq!(f.entries.front().unwrap().detected_at_tick, 101);
    }

    #[test]
    fn handle_event_pv2_bus_filters_to_sphere_field_emergence_prefixes() {
        // bus/events is a firehose — the feed filters to sphere.*, field.chimera*,
        // emergence.* and drops the rest (e.g. `bridge.health_probe`). Without this
        // filter, every housekeeping tick lands in the user-facing journal.
        let mut f = EventFeed::new();
        let payload = json!({"events":[
            {"event_type":"sphere.joined","tick":1,"timestamp":0.0,"data":{}},
            {"event_type":"bridge.health_probe","tick":2,"timestamp":0.0,"data":{}},
            {"event_type":"field.chimera_formation","tick":3,"timestamp":0.0,"data":{}},
        ]});
        let ev = HabitatEvent::BridgeData {
            module_id: "event_feed".into(),
            tag: "pv2_bus_events".into(),
            data: payload,
        };
        f.handle_event(&ev);
        assert_eq!(f.entries.len(), 2);
        let kinds: Vec<String> = f.entries.iter().map(|e| e.event_type.clone()).collect();
        assert!(kinds.contains(&"sphere.joined".into()));
        assert!(kinds.contains(&"field.chimera_formation".into()));
        assert!(
            !kinds.contains(&"bridge.health_probe".into()),
            "housekeeping events must NOT pollute the journal"
        );
    }

    #[test]
    fn handle_event_pv2_bus_tracks_monotonic_tick_to_avoid_replay() {
        let mut f = EventFeed::new();
        // First pass: events at tick 1, 2, 3.
        let first = json!({"events":[
            {"event_type":"sphere.a","tick":1,"timestamp":0.0,"data":{}},
            {"event_type":"sphere.b","tick":2,"timestamp":0.0,"data":{}},
            {"event_type":"sphere.c","tick":3,"timestamp":0.0,"data":{}},
        ]});
        f.handle_event(&HabitatEvent::BridgeData {
            module_id: "event_feed".into(),
            tag: "pv2_bus_events".into(),
            data: first,
        });
        assert_eq!(f.entries.len(), 3);

        // Second pass: same-tick events (polling overlap) must not duplicate.
        let second = json!({"events":[
            {"event_type":"sphere.a","tick":1,"timestamp":0.0,"data":{}},
            {"event_type":"sphere.d","tick":4,"timestamp":0.0,"data":{}},
        ]});
        f.handle_event(&HabitatEvent::BridgeData {
            module_id: "event_feed".into(),
            tag: "pv2_bus_events".into(),
            data: second,
        });
        // Entry at tick 4 lands; entry at tick 1 is replay-filtered by last_bus_tick.
        assert_eq!(f.entries.len(), 4);
    }

    #[test]
    fn handle_event_j_key_scrolls_down_not_past_saturation() {
        let mut f = EventFeed::new();
        assert_eq!(f.scroll_offset, 0);
        f.handle_event(&HabitatEvent::KeyPress { key: 'j' });
        assert_eq!(f.scroll_offset, 1);
        f.handle_event(&HabitatEvent::KeyPress { key: 'j' });
        assert_eq!(f.scroll_offset, 2);
    }

    #[test]
    fn handle_event_k_key_scrolls_up_saturating_at_zero() {
        let mut f = EventFeed::new();
        // k at origin must not wrap to usize::MAX (saturating_sub).
        f.handle_event(&HabitatEvent::KeyPress { key: 'k' });
        assert_eq!(f.scroll_offset, 0);
    }

    #[test]
    fn handle_event_g_key_jumps_to_top() {
        let mut f = EventFeed::new();
        f.scroll_offset = 42;
        f.handle_event(&HabitatEvent::KeyPress { key: 'g' });
        assert_eq!(f.scroll_offset, 0);
    }

    #[test]
    fn confidence_color_tiers_match_documented_thresholds() {
        // >0.9 green, >0.7 yellow, else red — drives the emergence_journal colouring.
        assert_eq!(EventFeed::confidence_color(0.95), GRN);
        assert_eq!(EventFeed::confidence_color(0.9), YEL); // strictly > 0.9 is green
        assert_eq!(EventFeed::confidence_color(0.75), YEL);
        assert_eq!(EventFeed::confidence_color(0.5), RED);
    }

    #[test]
    fn severity_icon_maps_high_and_critical_to_cross() {
        assert_eq!(EventFeed::severity_icon("high"), ICON_CROSS);
        assert_eq!(EventFeed::severity_icon("critical"), ICON_CROSS);
    }

    #[test]
    fn severity_icon_defaults_to_check_for_unknown() {
        assert_eq!(EventFeed::severity_icon("info"), ICON_CHECK);
        assert_eq!(EventFeed::severity_icon("unknown_tier"), ICON_CHECK);
    }

    #[test]
    fn data_sources_use_configured_urls_and_doubled_interval_for_bus() {
        let mut f = EventFeed::new();
        let mut m = std::collections::BTreeMap::new();
        m.insert("orac_url".into(), "http://orac.test:1".into());
        m.insert("pv2_url".into(), "http://pv2.test:2".into());
        m.insert("health_poll".into(), "3.0".into());
        let (cfg, _warnings) = ModuleConfig::from_btree(&m);
        f.init(&cfg);
        let ds = f.data_sources();
        assert_eq!(ds.len(), 2);
        assert!(ds[0].url.contains("orac.test:1/field"));
        assert!((ds[0].interval_secs - 3.0).abs() < f64::EPSILON);
        assert!(ds[1].url.contains("pv2.test:2/bus/events"));
        assert!(
            (ds[1].interval_secs - 6.0).abs() < f64::EPSILON,
            "bus events poll at 2x health_poll per module design"
        );
    }
}
