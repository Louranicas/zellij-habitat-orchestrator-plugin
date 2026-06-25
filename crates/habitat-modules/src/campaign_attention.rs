//! `campaign_attention` — the AWARENESS layer of the agentic-factory coordination fabric.
//!
//! Where `fiber_cockpit` is the full browsable WITNESS, this module is the ambient
//! ALERT view: it watches the same coordination medium and surfaces only what
//! *changed* — a campaign fiber grew, a lease is near expiry, an arming key flipped.
//! Quiet by default (one line); loud only when a signal needs a human glance.
//!
//! # Data path (shared pipe feed)
//! Consumes the SAME `fiber-data` pipe `fiber_cockpit` does — fed by
//! `bin/fiber-cockpit-snapshot` via `bin/fiber-cockpit`. One feeder, two witnesses
//! (the stigmergy ideal: the medium serves all readers). No curl `DataSource`, no
//! core-trait change, no host-timer splice. (The plan's self-polling host-splice +
//! config-driven watch set is a documented follow-up, held like `fiber_cockpit`'s
//! `command_sources()` upgrade.)
//!
//! # Change detection
//! Per campaign a digest `(node_count, status-multiset, armed)` is compared against an
//! acked baseline; a delta raises a `NEW` flag, cleared by the `a` key or the
//! `attention-ack` pipe. Lease warnings are stateless — they auto-clear on
//! renew/release/expiry (mirroring the kv-lease lifecycle), no ack needed.
//!
//! # Boundary (integration doc §6: witness, not actor)
//! Zero substrate writes — render-only awareness over a read-only feed. Enforced by
//! the external grep gate (no kv/lease/fiber write verbs in this module).

use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use std::collections::BTreeMap;

// Reuse the snapshot wire types — one medium document, two witnesses (DRY within crate).
use crate::fiber_cockpit::FiberSnapshot;

/// Lease ttl below this (seconds) renders RED.
const TTL_CRITICAL_SECS: i64 = 30;
/// Lease ttl below this (seconds) renders YELLOW (the "warn" window).
const TTL_WARN_SECS: i64 = 120;
/// Cap on campaigns shown (awareness, not telemetry).
const MAX_WATCHED: usize = 4;
/// No fresh feed for this many seconds → header stale tag (3× a 30s ambient cadence).
const STALE_THRESHOLD_SECS: f64 = 90.0;
/// The shared command-source/`BridgeData` tag `fiber_cockpit` publishes the snapshot under.
/// When both witnesses run in one instance, one feeder serves both (stigmergy ideal).
const SNAPSHOT_TAG: &str = "fiber_snapshot";

/// The `campaign_attention` module state.
pub struct CampaignAttention {
    snapshot: FiberSnapshot,
    /// Per-campaign acked digest. A campaign whose live digest differs is NEW.
    acked: BTreeMap<String, String>,
    /// Optional explicit watch filter (empty = show all, capped). Set via pipe.
    watch: Vec<String>,
    /// Optional owner prefix for the MINE highlight (set via `attention-mine` pipe).
    owner_prefix: Option<String>,
    /// Ticks since the last `fiber-data` feed — drives the stale tag.
    ticks_since_data: u64,
    /// Seconds per tick, from config `health_poll` (stale-seconds estimate).
    poll_secs: f64,
}

impl CampaignAttention {
    /// Construct an empty awareness module (no snapshot until the first feed).
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot: FiberSnapshot::default(),
            acked: BTreeMap::new(),
            watch: Vec::new(),
            owner_prefix: None,
            ticks_since_data: 0,
            poll_secs: 5.0,
        }
    }

    /// Digest of a campaign's salient state: `(node_count | sorted-status-multiset | armed)`.
    /// A change in any component flips the campaign to NEW.
    fn campaign_digest(&self, name: &str) -> String {
        let nodes = self
            .snapshot
            .campaigns
            .iter()
            .find(|c| c.name == name)
            .map_or(0, |c| c.nodes.len());
        let mut statuses: Vec<&str> = self
            .snapshot
            .campaigns
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.nodes.iter().map(|n| n.status.as_str()).collect())
            .unwrap_or_default();
        statuses.sort_unstable();
        let armed = self.is_armed(name);
        // Pipe-delimited; component strings cannot contain '|' from the wire shape.
        format!("{nodes}|{}|{armed}", statuses.join(","))
    }

    /// Is a campaign's arming key set to exactly "armed"? (render-only signal.)
    fn is_armed(&self, name: &str) -> bool {
        self.snapshot
            .arming
            .iter()
            .any(|a| a.value == "armed" && (a.key == name || a.key.ends_with(name)))
    }

    /// The campaigns to display: explicit watch set if non-empty (filtered to those
    /// actually present), else every campaign in the snapshot — capped either way.
    fn visible_campaigns(&self) -> Vec<String> {
        let all: Vec<String> = if self.watch.is_empty() {
            self.snapshot
                .campaigns
                .iter()
                .map(|c| c.name.clone())
                .collect()
        } else {
            self.watch
                .iter()
                .filter(|w| self.snapshot.campaigns.iter().any(|c| &c.name == *w))
                .cloned()
                .collect()
        };
        all.into_iter().take(MAX_WATCHED).collect()
    }

    /// True if a campaign's live digest differs from its acked baseline.
    fn is_new(&self, name: &str) -> bool {
        match self.acked.get(name) {
            Some(d) => *d != self.campaign_digest(name),
            None => false, // first sighting is the BASELINE, not an alert
        }
    }

    /// Establish/refresh the baseline for every visible campaign (no alerts on first feed).
    fn baseline_all(&mut self) {
        for name in self.visible_campaigns() {
            let d = self.campaign_digest(&name);
            self.acked.entry(name).or_insert(d);
        }
        // Drop acked entries for campaigns that have left the snapshot (no unbounded growth).
        let live: Vec<String> = self
            .snapshot
            .campaigns
            .iter()
            .map(|c| c.name.clone())
            .collect();
        self.acked.retain(|k, _| live.contains(k));
    }

    /// Acknowledge: set the acked digest to the live digest for one campaign or all.
    fn ack(&mut self, campaign: Option<&str>) -> bool {
        let targets: Vec<String> = match campaign {
            Some(c) if !c.is_empty() => vec![c.to_string()],
            _ => self.visible_campaigns(),
        };
        let mut changed = false;
        for name in targets {
            let d = self.campaign_digest(&name);
            if self.acked.get(&name) != Some(&d) {
                self.acked.insert(name, d);
                changed = true;
            }
        }
        changed
    }

    /// Apply a fresh `fiber-data` payload (pipe path); returns true if it parsed.
    fn apply_payload(&mut self, payload: &str) -> bool {
        match serde_json::from_str::<FiberSnapshot>(payload) {
            Ok(snap) => {
                self.set_snapshot(snap);
                true
            }
            Err(_) => false,
        }
    }

    /// Apply a fresh snapshot from an already-parsed value (shared command-source
    /// `BridgeData` path); returns true if it deserialised.
    fn apply_value(&mut self, value: &serde_json::Value) -> bool {
        match serde_json::from_value::<FiberSnapshot>(value.clone()) {
            Ok(snap) => {
                self.set_snapshot(snap);
                true
            }
            Err(_) => false,
        }
    }

    /// Install a fresh snapshot + refresh baselines (one path for pipe + `BridgeData`).
    fn set_snapshot(&mut self, snap: FiberSnapshot) {
        self.snapshot = snap;
        self.ticks_since_data = 0;
        self.baseline_all();
    }

    /// Leases inside the warn window (`expires-now < TTL_WARN`), worst-first.
    fn warning_leases(&self) -> Vec<&crate::fiber_cockpit::LeaseRow> {
        let mut v: Vec<&crate::fiber_cockpit::LeaseRow> = self
            .snapshot
            .leases
            .iter()
            .filter(|l| !l.expired && l.ttl_remaining >= 0 && l.ttl_remaining < TTL_WARN_SECS)
            .collect();
        v.sort_by_key(|l| l.ttl_remaining);
        v
    }

    /// Count campaigns currently flagged NEW (unacked delta).
    fn new_count(&self) -> usize {
        self.visible_campaigns()
            .iter()
            .filter(|c| self.is_new(c))
            .count()
    }

    /// Count armed campaigns among the visible set.
    fn armed_count(&self) -> usize {
        self.visible_campaigns()
            .iter()
            .filter(|c| self.is_armed(c))
            .count()
    }

    fn stale_seconds(&self) -> f64 {
        // cast_precision_loss accepted crate-wide (lib.rs); display-only estimate.
        self.ticks_since_data as f64 * self.poll_secs.max(1.0)
    }

    fn ttl_color(ttl: i64) -> &'static str {
        if ttl < TTL_CRITICAL_SECS {
            RED
        } else if ttl < TTL_WARN_SECS {
            YEL
        } else {
            GRN
        }
    }
}

impl Default for CampaignAttention {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for CampaignAttention {
    fn id(&self) -> &'static str {
        "campaign_attention"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.poll_secs = config.health_poll;
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::PipeCommand { name, payload } => match name.as_str() {
                // Shared feed with fiber_cockpit — one medium document, two witnesses.
                "fiber-data" => self.apply_payload(payload),
                "attention-ack" => {
                    let c = payload.trim();
                    self.ack(if c.is_empty() { None } else { Some(c) })
                }
                "attention-watch" => {
                    let c = payload.trim();
                    if c.is_empty() || self.watch.iter().any(|w| w == c) {
                        false
                    } else {
                        if self.watch.len() >= MAX_WATCHED {
                            self.watch.remove(0); // evict oldest (bounded)
                        }
                        self.watch.push(c.to_string());
                        true
                    }
                }
                "attention-unwatch" => {
                    let c = payload.trim();
                    let before = self.watch.len();
                    self.watch.retain(|w| w != c);
                    self.watch.len() != before
                }
                "attention-mine" => {
                    let p = payload.trim();
                    self.owner_prefix = if p.is_empty() {
                        None
                    } else {
                        Some(p.to_string())
                    };
                    true
                }
                _ => false,
            },
            HabitatEvent::KeyPress { key } => match key {
                'a' | 'A' => self.ack(None),
                _ => false,
            },
            HabitatEvent::Tick { .. } => {
                self.ticks_since_data = self.ticks_since_data.saturating_add(1);
                false
            }
            // Shared snapshot feed (fiber_cockpit's command source): same medium, two witnesses.
            HabitatEvent::BridgeData { tag, data, .. } if tag == SNAPSHOT_TAG => {
                self.apply_value(data)
            }
            HabitatEvent::BridgeData { .. } | HabitatEvent::BridgeError { .. } => false,
        }
    }

    fn render(&self, rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);
        let visible = self.visible_campaigns();
        let new_n = self.new_count();
        let warn = self.warning_leases();
        let armed_n = self.armed_count();

        // ── QUIET: nothing needs attention → one line, no separator ─────────
        if new_n == 0 && warn.is_empty() {
            let stale = stale_tag(self.stale_seconds(), STALE_THRESHOLD_SECS).unwrap_or_default();
            return vec![RenderLine::new(format!(
                " {CYN}\u{2691} attention{R}  {D}quiet · {} campaigns · {} leases{R}  {D}armed:{}{R}{}",
                visible.len(),
                self.snapshot.leases.len(),
                armed_n,
                if stale.is_empty() { String::new() } else { format!("  {stale}") },
            ))];
        }

        // ── ACTIVE: at least one signal → header + rows (≤8 total) ──────────
        let mut lines = Vec::new();
        let stale = stale_tag(self.stale_seconds(), STALE_THRESHOLD_SECS).unwrap_or_default();
        lines.push(RenderLine::new(format!(
            " {B}{CYN}\u{2691} ATTENTION{R}  {} fiber \u{0394} · {} lease \u{26a0} · armed:{}{}",
            new_n,
            warn.len(),
            armed_n,
            if stale.is_empty() {
                String::new()
            } else {
                format!("   {stale}")
            },
        )));
        lines.push(RenderLine::separator(w));

        let budget = rows.min(8).saturating_sub(2).max(1);

        // NEW campaign rows first (the things a human must acknowledge).
        for name in visible.iter().filter(|c| self.is_new(c)) {
            if lines.len() >= budget + 2 {
                break;
            }
            let c = self.snapshot.campaigns.iter().find(|c| &c.name == name);
            let nodes = c.map_or(0, |c| c.nodes.len());
            let armed = if self.is_armed(name) {
                format!(" {GRN}armed{R}")
            } else {
                String::new()
            };
            lines.push(RenderLine::new(format!(
                " {GRN}\u{25cf}{R} {B}{}{R} {D}{} loops{R}{}  {MAG}{B}NEW{R}",
                truncate(name, w.saturating_sub(24)),
                nodes,
                armed,
            )));
        }

        // Lease-warning rows (stateless — no ack).
        for l in &warn {
            if lines.len() >= budget + 2 {
                break;
            }
            let col = Self::ttl_color(l.ttl_remaining);
            let mine = match &self.owner_prefix {
                Some(p) if l.owner.starts_with(p.as_str()) => format!(" {CYN}MINE{R}"),
                _ => String::new(),
            };
            lines.push(RenderLine::new(format!(
                " {YEL}\u{23f3}{R} {} {col}ttl {}s{R} {D}{}{R}{}",
                truncate(&l.resource, w.saturating_sub(28)),
                l.ttl_remaining.max(0),
                truncate(&l.owner, 20),
                mine,
            )));
        }

        lines.push(RenderLine::new(format!(" {D}a=ack{R}")));
        lines
    }

    fn serialize_state(&self) -> Option<String> {
        // Persist the acked baselines + watch set so alerts don't re-fire on re-init.
        serde_json::to_string(&(&self.acked, &self.watch, &self.owner_prefix)).ok()
    }

    fn restore_state(&mut self, state: &str) {
        type Persisted = (BTreeMap<String, String>, Vec<String>, Option<String>);
        if let Ok((acked, watch, prefix)) = serde_json::from_str::<Persisted>(state) {
            self.acked = acked;
            self.watch = watch;
            self.owner_prefix = prefix;
        }
    }

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![
            EventCategory::BridgeResponse, // shared `fiber_snapshot` feed from fiber_cockpit
            EventCategory::PipeCommand,    // manual `fiber-data` + attention-* control pipes
            EventCategory::KeyPress,
            EventCategory::Tick,
        ]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        Vec::new() // consumes fiber_cockpit's shared snapshot; declares no source of its own.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../tests/fixtures/fiber_snapshot_golden.json");

    fn loaded() -> CampaignAttention {
        let mut ca = CampaignAttention::new();
        assert!(ca.apply_payload(GOLDEN), "golden fixture must parse");
        ca
    }
    fn pipe(name: &str, payload: &str) -> HabitatEvent {
        HabitatEvent::PipeCommand {
            name: name.into(),
            payload: payload.into(),
        }
    }
    fn keyp(k: char) -> HabitatEvent {
        HabitatEvent::KeyPress { key: k }
    }
    fn snap_with(campaigns: &str, leases: &str, arming: &str) -> String {
        format!(r#"{{"v":1,"campaigns":[{campaigns}],"leases":[{leases}],"arming":[{arming}]}}"#)
    }

    // ── construction ─────────────────────────────────────────────────────
    #[test]
    fn new_is_empty_quiet() {
        let ca = CampaignAttention::new();
        assert!(ca.snapshot.campaigns.is_empty());
        assert_eq!(ca.new_count(), 0);
        assert!(ca.warning_leases().is_empty());
    }

    #[test]
    fn default_equals_new() {
        assert!(CampaignAttention::default().acked.is_empty());
    }

    #[test]
    fn id_and_version() {
        let ca = CampaignAttention::new();
        assert_eq!(ca.id(), "campaign_attention");
        assert_eq!(ca.version(), "0.1.0");
    }

    #[test]
    fn subscriptions_pipe_key_tick_and_bridge() {
        let s = CampaignAttention::new().subscriptions();
        assert!(s.contains(&EventCategory::PipeCommand));
        assert!(s.contains(&EventCategory::KeyPress));
        assert!(s.contains(&EventCategory::Tick));
        // BridgeResponse since it consumes fiber_cockpit's shared snapshot feed.
        assert!(s.contains(&EventCategory::BridgeResponse));
    }

    #[test]
    fn data_sources_empty_pipe_fed() {
        assert!(CampaignAttention::new().data_sources().is_empty());
    }

    // ── feed / parse ─────────────────────────────────────────────────────
    #[test]
    fn golden_parses_and_baselines() {
        let ca = loaded();
        assert!(!ca.snapshot.campaigns.is_empty());
        // First feed is the BASELINE — nothing is NEW.
        assert_eq!(ca.new_count(), 0, "first snapshot must not alert");
    }

    #[test]
    fn shared_fiber_data_pipe_feeds_module() {
        let mut ca = CampaignAttention::new();
        assert!(ca.handle_event(&pipe("fiber-data", GOLDEN)));
        assert!(!ca.snapshot.campaigns.is_empty());
    }

    #[test]
    fn shared_bridge_data_snapshot_feeds_module() {
        // The command-source feed (fiber_cockpit's snapshot) arrives as BridgeData.
        let mut ca = CampaignAttention::new();
        let data: serde_json::Value = serde_json::from_str(GOLDEN).unwrap();
        let ev = HabitatEvent::BridgeData {
            module_id: "fiber_cockpit".into(),
            tag: "fiber_snapshot".into(),
            data,
        };
        assert!(ca.handle_event(&ev));
        assert!(!ca.snapshot.campaigns.is_empty());
    }

    #[test]
    fn subscriptions_include_bridge_response() {
        assert!(CampaignAttention::new()
            .subscriptions()
            .contains(&EventCategory::BridgeResponse));
    }

    #[test]
    fn bridge_data_other_tag_ignored() {
        let mut ca = loaded();
        let ev = HabitatEvent::BridgeData {
            module_id: "x".into(),
            tag: "not_ours".into(),
            data: serde_json::json!({"campaigns": []}),
        };
        assert!(!ca.handle_event(&ev));
    }

    #[test]
    fn malformed_payload_ignored_keeps_state() {
        let mut ca = loaded();
        let before = ca.snapshot.campaigns.len();
        assert!(!ca.apply_payload("{ broken"));
        assert_eq!(ca.snapshot.campaigns.len(), before);
    }

    #[test]
    fn drift_tolerant_unknown_fields() {
        let mut ca = CampaignAttention::new();
        assert!(ca.apply_payload(r#"{"v":3,"unknown":true,"campaigns":[]}"#));
    }

    #[test]
    fn feed_resets_stale_counter() {
        let mut ca = loaded();
        ca.ticks_since_data = 50;
        ca.apply_payload(GOLDEN);
        assert_eq!(ca.ticks_since_data, 0);
    }

    // ── change detection ─────────────────────────────────────────────────
    #[test]
    fn growing_a_campaign_raises_new() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        ));
        assert_eq!(ca.new_count(), 0, "baseline");
        // second feed: same campaign gained a node → digest delta → NEW
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"},{"loop":"b","scale":"meso","status":"ok"}]}"#,
            "", ""));
        assert_eq!(ca.new_count(), 1, "node growth must raise NEW");
        assert!(ca.is_new("c"));
    }

    #[test]
    fn status_change_raises_new() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        ));
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"degraded"}]}"#,
            "",
            "",
        ));
        assert!(ca.is_new("c"), "a status flip must raise NEW");
    }

    #[test]
    fn identical_feed_does_not_raise_new() {
        let mut ca = CampaignAttention::new();
        let s = snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        );
        ca.apply_payload(&s);
        ca.apply_payload(&s);
        assert_eq!(ca.new_count(), 0, "an unchanged campaign must stay quiet");
    }

    #[test]
    fn arming_flip_raises_new() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(r#"{"name":"factory-x","nodes":[]}"#, "", ""));
        assert!(!ca.is_armed("factory-x"));
        ca.apply_payload(&snap_with(
            r#"{"name":"factory-x","nodes":[]}"#,
            "",
            r#"{"key":"factory.authorize.factory-x","value":"armed"}"#,
        ));
        assert!(ca.is_armed("factory-x"));
        assert!(ca.is_new("factory-x"), "arming flip must raise NEW");
    }

    #[test]
    fn first_sighting_is_baseline_not_alert() {
        // A campaign appearing in a later feed is a BASELINE on first sight, never NEW.
        let mut ca = loaded();
        ca.apply_payload(&snap_with(r#"{"name":"brand-new","nodes":[]}"#, "", ""));
        assert!(
            !ca.is_new("brand-new"),
            "a never-before-seen campaign is a baseline"
        );
    }

    // ── ack ──────────────────────────────────────────────────────────────
    #[test]
    fn ack_all_clears_new() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        ));
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"degraded"}]}"#,
            "",
            "",
        ));
        assert_eq!(ca.new_count(), 1);
        assert!(ca.ack(None));
        assert_eq!(ca.new_count(), 0, "ack clears the NEW flag");
    }

    #[test]
    fn ack_key_a_clears_new() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        ));
        ca.apply_payload(&snap_with(r#"{"name":"c","nodes":[]}"#, "", ""));
        assert!(ca.is_new("c"));
        assert!(ca.handle_event(&keyp('a')));
        assert!(!ca.is_new("c"));
    }

    #[test]
    fn ack_uppercase_a_also_works() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(r#"{"name":"c","nodes":[]}"#, "", ""));
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"x","scale":"micro","status":"ok"}]}"#,
            "",
            "",
        ));
        assert!(ca.handle_event(&keyp('A')));
        assert!(!ca.is_new("c"));
    }

    #[test]
    fn ack_pipe_single_campaign() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c1","nodes":[]},{"name":"c2","nodes":[]}"#,
            "",
            "",
        ));
        ca.apply_payload(&snap_with(
            r#"{"name":"c1","nodes":[{"loop":"a","scale":"macro","status":"ok"}]},{"name":"c2","nodes":[{"loop":"b","scale":"macro","status":"ok"}]}"#, "", ""));
        assert_eq!(ca.new_count(), 2);
        assert!(ca.handle_event(&pipe("attention-ack", "c1")));
        assert!(!ca.is_new("c1"));
        assert!(
            ca.is_new("c2"),
            "single-campaign ack must not clear the other"
        );
    }

    #[test]
    fn ack_when_nothing_new_is_noop() {
        let mut ca = loaded();
        assert!(!ca.ack(None), "ack with no delta returns false");
    }

    #[test]
    fn ack_empty_pipe_payload_acks_all() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(r#"{"name":"c","nodes":[]}"#, "", ""));
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        ));
        assert!(ca.handle_event(&pipe("attention-ack", "")));
        assert!(!ca.is_new("c"));
    }

    // ── watch set ────────────────────────────────────────────────────────
    #[test]
    fn watch_filters_visible() {
        let mut ca = loaded();
        let first = ca.snapshot.campaigns[0].name.clone();
        assert!(ca.handle_event(&pipe("attention-watch", &first)));
        let vis = ca.visible_campaigns();
        assert_eq!(vis, vec![first]);
    }

    #[test]
    fn watch_dedupes() {
        let mut ca = loaded();
        let n = ca.snapshot.campaigns[0].name.clone();
        assert!(ca.handle_event(&pipe("attention-watch", &n)));
        assert!(
            !ca.handle_event(&pipe("attention-watch", &n)),
            "duplicate watch is a no-op"
        );
    }

    #[test]
    fn watch_evicts_oldest_beyond_cap() {
        let mut ca = CampaignAttention::new();
        for i in 0..MAX_WATCHED + 2 {
            ca.handle_event(&pipe("attention-watch", &format!("c{i}")));
        }
        assert_eq!(ca.watch.len(), MAX_WATCHED, "watch set is bounded");
        assert!(!ca.watch.contains(&"c0".to_string()), "oldest evicted");
    }

    #[test]
    fn unwatch_removes() {
        let mut ca = loaded();
        let n = ca.snapshot.campaigns[0].name.clone();
        ca.handle_event(&pipe("attention-watch", &n));
        assert!(ca.handle_event(&pipe("attention-unwatch", &n)));
        assert!(ca.watch.is_empty());
    }

    #[test]
    fn unwatch_absent_is_noop() {
        let mut ca = loaded();
        assert!(!ca.handle_event(&pipe("attention-unwatch", "never-watched")));
    }

    #[test]
    fn empty_watch_shows_all_capped() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"a","nodes":[]},{"name":"b","nodes":[]},{"name":"c","nodes":[]},{"name":"d","nodes":[]},{"name":"e","nodes":[]}"#,
            "", ""));
        assert_eq!(
            ca.visible_campaigns().len(),
            MAX_WATCHED,
            "capped at MAX_WATCHED"
        );
    }

    // ── lease warnings ───────────────────────────────────────────────────
    #[test]
    fn lease_within_warn_window_surfaces() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[]}"#,
            r#"{"resource":"r","owner":"o","ttl_remaining":60,"note":"","expired":false}"#,
            "",
        ));
        assert_eq!(ca.warning_leases().len(), 1);
    }

    #[test]
    fn lease_outside_warn_window_is_ignored() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[]}"#,
            r#"{"resource":"r","owner":"o","ttl_remaining":600,"note":"","expired":false}"#,
            "",
        ));
        assert!(
            ca.warning_leases().is_empty(),
            "a lease far from expiry is not a warning"
        );
    }

    #[test]
    fn expired_lease_is_not_a_warning() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[]}"#,
            r#"{"resource":"r","owner":"o","ttl_remaining":0,"note":"","expired":true}"#,
            "",
        ));
        assert!(
            ca.warning_leases().is_empty(),
            "expired = free, not a live warning"
        );
    }

    #[test]
    fn warning_leases_sorted_worst_first() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[]}"#,
            r#"{"resource":"a","owner":"o","ttl_remaining":100,"note":"","expired":false},{"resource":"b","owner":"o","ttl_remaining":20,"note":"","expired":false}"#,
            ""));
        let w = ca.warning_leases();
        assert_eq!(w[0].resource, "b", "lowest ttl first");
    }

    #[test]
    fn ttl_color_tiers() {
        assert_eq!(CampaignAttention::ttl_color(10), RED);
        assert_eq!(CampaignAttention::ttl_color(60), YEL);
        assert_eq!(CampaignAttention::ttl_color(300), GRN);
    }

    // ── armed ────────────────────────────────────────────────────────────
    #[test]
    fn is_armed_matches_suffix_key() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"camp","nodes":[]}"#,
            "",
            r#"{"key":"factory.authorize.camp","value":"armed"}"#,
        ));
        assert!(ca.is_armed("camp"));
    }

    #[test]
    fn is_armed_false_for_non_armed_value() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"camp","nodes":[]}"#,
            "",
            r#"{"key":"factory.authorize.camp","value":"pending"}"#,
        ));
        assert!(!ca.is_armed("camp"));
    }

    #[test]
    fn armed_count_counts_visible() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"camp","nodes":[]}"#,
            "",
            r#"{"key":"factory.authorize.camp","value":"armed"}"#,
        ));
        assert_eq!(ca.armed_count(), 1);
    }

    // ── mine highlight ───────────────────────────────────────────────────
    #[test]
    fn attention_mine_sets_prefix() {
        let mut ca = CampaignAttention::new();
        assert!(ca.handle_event(&pipe("attention-mine", "zj:main")));
        assert_eq!(ca.owner_prefix.as_deref(), Some("zj:main"));
    }

    #[test]
    fn attention_mine_empty_clears_prefix() {
        let mut ca = CampaignAttention::new();
        ca.handle_event(&pipe("attention-mine", "zj:main"));
        assert!(ca.handle_event(&pipe("attention-mine", "")));
        assert!(ca.owner_prefix.is_none());
    }

    #[test]
    fn mine_highlight_renders_when_owner_matches() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[]}"#,
            r#"{"resource":"r","owner":"zj:main:7","ttl_remaining":50,"note":"","expired":false}"#,
            "",
        ));
        ca.owner_prefix = Some("zj:main".into());
        let joined = ca
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("MINE"));
    }

    // ── routing / misc events ────────────────────────────────────────────
    #[test]
    fn unknown_pipe_ignored() {
        let mut ca = loaded();
        assert!(!ca.handle_event(&pipe("some-other", "x")));
    }

    #[test]
    fn unknown_key_ignored() {
        let mut ca = loaded();
        assert!(!ca.handle_event(&keyp('z')));
    }

    #[test]
    fn tick_ages_stale_clock_without_render() {
        let mut ca = loaded();
        assert!(!ca.handle_event(&HabitatEvent::Tick { tick: 1 }));
        assert_eq!(ca.ticks_since_data, 1);
    }

    #[test]
    fn tick_saturates() {
        let mut ca = CampaignAttention::new();
        ca.ticks_since_data = u64::MAX;
        ca.handle_event(&HabitatEvent::Tick { tick: 9 });
        assert_eq!(ca.ticks_since_data, u64::MAX);
    }

    #[test]
    fn bridge_events_ignored() {
        let mut ca = loaded();
        assert!(!ca.handle_event(&HabitatEvent::BridgeError {
            module_id: "x".into(),
            tag: "y".into()
        }));
    }

    // ── digest internals ─────────────────────────────────────────────────
    #[test]
    fn digest_is_order_independent_for_statuses() {
        // status multiset is sorted, so node ordering must not change the digest.
        let mut a = CampaignAttention::new();
        a.apply_payload(&snap_with(r#"{"name":"c","nodes":[{"loop":"x","scale":"macro","status":"ok"},{"loop":"y","scale":"meso","status":"degraded"}]}"#, "", ""));
        let mut b = CampaignAttention::new();
        b.apply_payload(&snap_with(r#"{"name":"c","nodes":[{"loop":"y","scale":"meso","status":"degraded"},{"loop":"x","scale":"macro","status":"ok"}]}"#, "", ""));
        assert_eq!(a.campaign_digest("c"), b.campaign_digest("c"));
    }

    #[test]
    fn digest_absent_campaign_is_zero_shaped() {
        let ca = CampaignAttention::new();
        assert_eq!(ca.campaign_digest("ghost"), "0||false");
    }

    #[test]
    fn acked_map_drops_departed_campaigns() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(r#"{"name":"gone","nodes":[]}"#, "", ""));
        assert!(ca.acked.contains_key("gone"));
        ca.apply_payload(&snap_with(r#"{"name":"other","nodes":[]}"#, "", ""));
        assert!(
            !ca.acked.contains_key("gone"),
            "departed campaign pruned from acked map"
        );
    }

    // ── render ───────────────────────────────────────────────────────────
    #[test]
    fn render_quiet_is_single_line() {
        let ca = loaded();
        let lines = ca.render(20, 90);
        assert_eq!(lines.len(), 1, "quiet state is exactly one line");
        assert!(lines[0].content.contains("quiet"));
    }

    #[test]
    fn render_empty_is_quiet() {
        let lines = CampaignAttention::new().render(20, 90);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].content.contains("attention"));
    }

    #[test]
    fn render_active_shows_new_and_ack_hint() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(r#"{"name":"c","nodes":[]}"#, "", ""));
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        ));
        let joined = ca
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("ATTENTION"), "active header");
        assert!(joined.contains("NEW"));
        assert!(joined.contains("a=ack"));
    }

    #[test]
    fn render_active_shows_lease_warning() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[]}"#,
            r#"{"resource":"git.index.workspace","owner":"o","ttl_remaining":25,"note":"","expired":false}"#, ""));
        let joined = ca
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("ATTENTION"));
        assert!(joined.contains("git.index.workspace"));
    }

    #[test]
    fn render_respects_eight_line_budget() {
        let mut ca = CampaignAttention::new();
        // many NEW campaigns + many warning leases
        let camps = (0..6)
            .map(|i| format!(r#"{{"name":"c{i}","nodes":[]}}"#))
            .collect::<Vec<_>>()
            .join(",");
        ca.apply_payload(&snap_with(&camps, "", ""));
        let camps2 = (0..6)
            .map(|i| {
                format!(
                    r#"{{"name":"c{i}","nodes":[{{"loop":"x","scale":"macro","status":"ok"}}]}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        ca.apply_payload(&snap_with(&camps2, "", ""));
        let lines = ca.render(40, 100);
        assert!(
            lines.len() <= 8,
            "ambient module caps at 8 lines, got {}",
            lines.len()
        );
    }

    #[test]
    fn render_never_panics_across_dims() {
        let mut ca = loaded();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[]}"#,
            r#"{"resource":"r","owner":"o","ttl_remaining":5,"note":"","expired":false}"#,
            "",
        ));
        for (r, c) in [(0, 0), (1, 1), (2, 5), (8, 200), (40, 300)] {
            let _ = ca.render(r, c);
        }
    }

    #[test]
    fn render_armed_count_in_quiet_header() {
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"camp","nodes":[]}"#,
            "",
            r#"{"key":"factory.authorize.camp","value":"armed"}"#,
        ));
        // armed but no delta and no lease-warn → still quiet, header shows armed:1
        let joined = ca
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("armed:1"));
    }

    // ── serialize / restore ──────────────────────────────────────────────
    #[test]
    fn serialize_restore_round_trips_acked_and_watch() {
        let mut ca = loaded();
        ca.ack(None);
        ca.watch.push("plugin-plans-s1007594".into());
        ca.owner_prefix = Some("zj:x".into());
        let state = ca.serialize_state().expect("serialize");
        let mut fresh = CampaignAttention::new();
        fresh.restore_state(&state);
        assert_eq!(fresh.watch, vec!["plugin-plans-s1007594".to_string()]);
        assert_eq!(fresh.owner_prefix.as_deref(), Some("zj:x"));
        assert!(!fresh.acked.is_empty());
    }

    #[test]
    fn restore_ignores_garbage() {
        let mut ca = loaded();
        ca.watch.push("keep".into());
        ca.restore_state("not json");
        assert_eq!(ca.watch, vec!["keep".to_string()]);
    }

    #[test]
    fn restored_baseline_suppresses_replay_alert() {
        // After restore, an identical snapshot must not re-fire NEW (the point of persisting acked).
        let mut ca = CampaignAttention::new();
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"ok"}]}"#,
            "",
            "",
        ));
        ca.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"degraded"}]}"#,
            "",
            "",
        ));
        ca.ack(None);
        let state = ca.serialize_state().unwrap();
        let mut fresh = CampaignAttention::new();
        fresh.restore_state(&state);
        fresh.apply_payload(&snap_with(
            r#"{"name":"c","nodes":[{"loop":"a","scale":"macro","status":"degraded"}]}"#,
            "",
            "",
        ));
        assert!(
            !fresh.is_new("c"),
            "restored ack baseline suppresses the replay alert"
        );
    }
}
