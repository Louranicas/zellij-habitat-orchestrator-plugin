//! `fiber_cockpit` — the WITNESS module of the agentic-factory coordination fabric.
//!
//! Renders the coordination MEDIUM (S1007594 torus-stack): hopf campaign fibers,
//! the kv-lease table, and factory arming-key states — so an operator can see, at a
//! glance, which loops ran under which campaign and what is currently held/armed.
//!
//! # Data path (pipe-fed first cut)
//! The host helper `bin/fiber-cockpit-snapshot` aggregates the medium into one JSON
//! document and feeds it in via `zellij pipe -n fiber-data` (driven by the
//! `bin/fiber-cockpit` launch wrapper). This module subscribes to [`EventCategory::PipeCommand`]
//! and replaces its snapshot on each `fiber-data` message — no curl `DataSource`, no
//! core-trait change. (The self-polling `CommandSource` upgrade is a documented
//! follow-up gated behind the crate's "ask before core-trait change" doctrine.)
//!
//! # Boundary (integration doc §6: witness, not actor)
//! This module performs ZERO writes to any substrate. It owns no KV key, claims no
//! lease, links no fiber. It only renders what the read-only helper observed. The
//! arming-key state is render-only. A mechanical grep gate (plan §9.8) enforces this.
//!
//! # Failure posture
//! Every wire field is `#[serde(default)]` (drift-tolerant); a malformed payload is
//! ignored (last snapshot retained); a WASM panic would kill the pane, so parse paths
//! never `unwrap`. Stale beats blank.

use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{CommandSource, DataSource, HabitatModule};
use habitat_core::render::*;
use serde::{Deserialize, Serialize};

/// Below this many seconds without a fresh feed, the header shows a stale tag.
/// The self-poll fires every ~30s; 150s is ~5 missed polls.
const STALE_THRESHOLD_SECS: f64 = 150.0;
/// Absolute path to the read-only aggregator (`run_command` execs argv directly — no
/// shell, no `$PATH`/`~` expansion, so the path must be absolute).
const SNAPSHOT_HELPER: &str = "/home/louranicas/claude-code-workspace/bin/fiber-cockpit-snapshot";
/// Self-poll cadence for the snapshot command source.
/// 30s (was 5s) — overlap guard lives in the helper's flock, but slower poll
/// keeps subprocess cardinality bounded across many Zellij servers (S1008517).
const SNAPSHOT_POLL_SECS: f64 = 30.0;
/// `BridgeData`/command-source tag the snapshot arrives under (SHARED with `campaign_attention`).
const SNAPSHOT_TAG: &str = "fiber_snapshot";

/// One node on a campaign fiber — a loop's receipt anchor at its scale.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FiberNode {
    #[serde(default)]
    pub r#loop: String,
    #[serde(default)]
    pub scale: String,
    #[serde(default)]
    pub anchor: String,
    #[serde(default)]
    pub parent: String,
    #[serde(default)]
    pub status: String,
}

/// One hopf campaign and its fiber tree.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CampaignDoc {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub nodes: Vec<FiberNode>,
    #[serde(default)]
    pub truncated: bool,
}

/// One held/expired lease row (free slots are omitted by the helper).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LeaseRow {
    #[serde(default)]
    pub resource: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub ttl_remaining: i64,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub expired: bool,
}

/// One factory arming-key state (render-only — the cockpit never sets it).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ArmRow {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: String,
}

/// The full medium snapshot, as produced by `bin/fiber-cockpit-snapshot`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FiberSnapshot {
    #[serde(default)]
    pub v: u32,
    #[serde(default)]
    pub ts: u64,
    #[serde(default)]
    pub campaigns: Vec<CampaignDoc>,
    #[serde(default)]
    pub leases: Vec<LeaseRow>,
    #[serde(default)]
    pub arming: Vec<ArmRow>,
    #[serde(default)]
    pub errors: Vec<String>,
}

/// The `fiber_cockpit` module state.
pub struct FiberCockpit {
    snapshot: FiberSnapshot,
    /// Selected campaign index (clamped to the campaign list on every snapshot).
    selected: usize,
    /// Browse (campaign list) vs Expanded (selected campaign's fiber tree).
    expanded: bool,
    /// Ticks since the last `fiber-data` pipe — drives the stale tag.
    ticks_since_data: u64,
    /// Seconds-per-tick, from config `health_poll` (for stale-seconds estimate).
    poll_secs: f64,
}

impl FiberCockpit {
    /// Construct an empty cockpit (no snapshot until the first `fiber-data` pipe).
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot: FiberSnapshot::default(),
            selected: 0,
            expanded: false,
            ticks_since_data: 0,
            poll_secs: SNAPSHOT_POLL_SECS,
        }
    }

    /// Clamp `selected` into the current campaign list (head on empty).
    fn clamp_selection(&mut self) {
        let n = self.snapshot.campaigns.len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// Replace the snapshot from a parsed `fiber-data` payload. Returns true if the
    /// payload parsed (and was therefore applied); false leaves prior state intact.
    fn apply_payload(&mut self, payload: &str) -> bool {
        match serde_json::from_str::<FiberSnapshot>(payload) {
            Ok(snap) => {
                self.set_snapshot(snap);
                true
            }
            Err(_) => false,
        }
    }

    /// Replace the snapshot from an already-parsed JSON value (the command-source
    /// `BridgeData` path). Returns true if it deserialised into a `FiberSnapshot`.
    fn apply_value(&mut self, value: &serde_json::Value) -> bool {
        match serde_json::from_value::<FiberSnapshot>(value.clone()) {
            Ok(snap) => {
                self.set_snapshot(snap);
                true
            }
            Err(_) => false,
        }
    }

    /// Install a fresh snapshot + reset the view/stale bookkeeping (one code path
    /// for both the pipe and command-source feeds).
    fn set_snapshot(&mut self, snap: FiberSnapshot) {
        self.snapshot = snap;
        self.clamp_selection();
        self.ticks_since_data = 0;
    }

    /// Select a campaign by name if present; returns true on a hit.
    fn select_by_name(&mut self, name: &str) -> bool {
        if let Some(idx) = self.snapshot.campaigns.iter().position(|c| c.name == name) {
            self.selected = idx;
            true
        } else {
            false
        }
    }

    /// Scale badge with a per-scale colour (macro=magenta, meso=cyan, micro=dim).
    fn scale_badge(scale: &str) -> String {
        match scale {
            "macro" => format!("{MAG}MAC{R}"),
            "meso" => format!("{CYN}MES{R}"),
            "micro" => format!("{D}MIC{R}"),
            "raw" => format!("{RED}RAW{R}"),
            _ => format!("{D}???{R}"),
        }
    }

    /// Status glyph (ok=green check, degraded=yellow warn, else red cross).
    fn status_glyph(status: &str) -> String {
        match status {
            "ok" => format!("{GRN}{ICON_CHECK}{R}"),
            "degraded" => format!("{YEL}\u{26a0}{R}"),
            "" => format!("{D}-{R}"),
            _ => format!("{RED}{ICON_CROSS}{R}"),
        }
    }

    /// Estimated seconds since the last data feed (for the stale tag).
    ///
    /// `cast_precision_loss` is accepted crate-wide (see `lib.rs`) — this is a
    /// display-only estimate and tick counts never approach 2^53.
    fn stale_seconds(&self) -> f64 {
        // poll_secs is floored so a misconfig can't zero the estimate.
        let per = self.poll_secs.max(1.0);
        self.ticks_since_data as f64 * per
    }
}

impl Default for FiberCockpit {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for FiberCockpit {
    fn id(&self) -> &'static str {
        "fiber_cockpit"
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
                "fiber-data" => self.apply_payload(payload),
                "fiber-refresh" => {
                    // A named-campaign refresh ping selects it; the actual data
                    // re-feed is host-side (the wrapper loop). Empty payload = no-op.
                    if payload.is_empty() {
                        false
                    } else {
                        self.select_by_name(payload.trim())
                    }
                }
                _ => false,
            },
            HabitatEvent::KeyPress { key } => match key {
                'j' | 'J' => {
                    let n = self.snapshot.campaigns.len();
                    if n > 0 && self.selected + 1 < n {
                        self.selected += 1;
                        return true;
                    }
                    false
                }
                'k' | 'K' => {
                    if self.selected > 0 {
                        self.selected -= 1;
                        return true;
                    }
                    false
                }
                // 'l' and Enter('\n') both expand (N1/F12: Enter dies in a stale host,
                // so 'l' is the always-available expand key).
                'l' | '\n' => {
                    if self.expanded {
                        false
                    } else {
                        self.expanded = true;
                        true
                    }
                }
                'h' if self.expanded => {
                    self.expanded = false;
                    true
                }
                'g' | 'G' => {
                    let changed = self.selected != 0 || self.expanded;
                    self.selected = 0;
                    self.expanded = false;
                    changed
                }
                _ => false,
            },
            HabitatEvent::Tick { .. } => {
                self.ticks_since_data = self.ticks_since_data.saturating_add(1);
                false // tick alone never forces a re-render here
            }
            // Self-poll feed: the snapshot command source returns here as BridgeData.
            HabitatEvent::BridgeData { tag, data, .. } if tag == SNAPSHOT_TAG => {
                self.apply_value(data)
            }
            HabitatEvent::BridgeData { .. } | HabitatEvent::BridgeError { .. } => false,
        }
    }

    fn render(&self, rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);
        let mut lines = Vec::new();

        // ── header (with stale tag + error count) ──────────────────────────
        let stale = stale_tag(self.stale_seconds(), STALE_THRESHOLD_SECS).unwrap_or_default();
        let err_tag = if self.snapshot.errors.is_empty() {
            String::new()
        } else {
            format!("  {RED}!{}{R}", self.snapshot.errors.len())
        };
        lines.push(RenderLine::new(format!(
            " {B}{CYN}FIBER{R} {D}{} campaigns · {} leases{R}{}{}",
            self.snapshot.campaigns.len(),
            self.snapshot.leases.len(),
            err_tag,
            if stale.is_empty() {
                String::new()
            } else {
                format!("  {stale}")
            },
        )));
        lines.push(RenderLine::separator(w));

        if self.snapshot.campaigns.is_empty() {
            lines.push(RenderLine::new(format!(
                " {D}no fiber data yet — waiting for fiber-data pipe (bin/fiber-cockpit){R}"
            )));
        } else if self.expanded {
            // ── expanded: selected campaign's fiber tree ───────────────────
            let c = &self.snapshot.campaigns[self.selected.min(self.snapshot.campaigns.len() - 1)];
            lines.push(RenderLine::new(format!(
                " {B}{}{R} {D}{}{R}",
                truncate(&c.name, w.saturating_sub(4)),
                truncate(&c.root, 40),
            )));
            let body = rows.saturating_sub(lines.len() + 1).max(2);
            for node in c.nodes.iter().take(body) {
                let indent = if node.parent.is_empty() { "" } else { "  " };
                lines.push(RenderLine::new(format!(
                    " {indent}{} {} {B}{}{R} {D}{}{R}",
                    Self::status_glyph(&node.status),
                    Self::scale_badge(&node.scale),
                    truncate(&node.r#loop, 24),
                    truncate(&node.anchor, w.saturating_sub(40)),
                )));
            }
            if c.truncated {
                lines.push(RenderLine::new(format!(
                    " {YEL}+more — campaign {} is prunable via the hopf-anchor CLI{R}",
                    truncate(&c.name, 24),
                )));
            }
            lines.push(RenderLine::new(format!(" {D}[h] back  [g] top{R}")));
        } else {
            // ── browse: campaign list ──────────────────────────────────────
            let body = rows.saturating_sub(lines.len() + 1).max(2);
            for (i, c) in self.snapshot.campaigns.iter().take(body).enumerate() {
                let marker = if i == self.selected {
                    format!("{CYN}>{R}")
                } else {
                    " ".into()
                };
                lines.push(RenderLine::new(format!(
                    " {marker} {B}{:<28}{R} {D}{} loops{R}{}",
                    truncate(&c.name, 28),
                    c.nodes.len(),
                    if c.truncated {
                        format!(" {YEL}+{R}")
                    } else {
                        String::new()
                    },
                )));
            }

            // ── lease table (held + expired) ───────────────────────────────
            if !self.snapshot.leases.is_empty() {
                lines.push(RenderLine::new(format!(" {D}── leases ──{R}")));
                for l in self.snapshot.leases.iter().take(4) {
                    let (col, ttl) = if l.expired {
                        (D, "EXPIRED".to_string())
                    } else {
                        (GRN, format!("{}s", l.ttl_remaining.max(0)))
                    };
                    lines.push(RenderLine::new(format!(
                        " {col}{} {R}{D}{}{R} {col}{}{R}",
                        truncate(&l.resource, 28),
                        truncate(&l.owner, 22),
                        ttl,
                    )));
                }
            }

            // ── arming keys (render-only) ──────────────────────────────────
            for a in &self.snapshot.arming {
                let armed = a.value == "armed";
                let col = if armed { GRN } else { D };
                lines.push(RenderLine::new(format!(
                    " {col}ARM {} = {}{R}",
                    truncate(&a.key, 40),
                    if armed { "armed" } else { "—" },
                )));
            }

            lines.push(RenderLine::new(format!(
                " {D}[j/k] select  [l] expand  [g] top{R}"
            )));
        }

        lines
    }

    fn serialize_state(&self) -> Option<String> {
        // Persist only the lightweight view cursor; the snapshot re-feeds on resume.
        serde_json::to_string(&(self.selected, self.expanded)).ok()
    }

    fn restore_state(&mut self, state: &str) {
        if let Ok((sel, exp)) = serde_json::from_str::<(usize, bool)>(state) {
            self.selected = sel;
            self.expanded = exp;
        }
    }

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![
            EventCategory::BridgeResponse, // command-source snapshot feed
            EventCategory::PipeCommand,    // manual `fiber-data` pipe + `fiber-refresh`
            EventCategory::KeyPress,
            EventCategory::Tick,
        ]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        Vec::new() // no curl sources — the snapshot is a CommandSource (below)
    }

    fn command_sources(&self) -> Vec<CommandSource> {
        // Self-poll the read-only aggregator; result arrives as BridgeData{SNAPSHOT_TAG}.
        // Shared with campaign_attention when both run in one instance (one feeder,
        // two witnesses). The pipe path (`fiber-data`) remains for manual/operator feeds.
        vec![CommandSource {
            argv: vec![SNAPSHOT_HELPER.to_string()],
            interval_secs: SNAPSHOT_POLL_SECS,
            tag: SNAPSHOT_TAG.to_string(),
            module_id: self.id().to_string(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../tests/fixtures/fiber_snapshot_golden.json");

    fn loaded() -> FiberCockpit {
        let mut fc = FiberCockpit::new();
        assert!(fc.apply_payload(GOLDEN), "golden fixture must parse");
        fc
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

    // ── construction / defaults ──────────────────────────────────────────
    #[test]
    fn new_starts_empty_browse_mode() {
        let fc = FiberCockpit::new();
        assert_eq!(fc.selected, 0);
        assert!(!fc.expanded);
        assert!(fc.snapshot.campaigns.is_empty());
    }

    #[test]
    fn default_equals_new() {
        let a = FiberCockpit::default();
        assert_eq!(a.selected, 0);
        assert!(!a.expanded);
    }

    #[test]
    fn id_and_version_are_stable() {
        let fc = FiberCockpit::new();
        assert_eq!(fc.id(), "fiber_cockpit");
        assert_eq!(fc.version(), "0.1.0");
    }

    #[test]
    fn subscriptions_cover_pipe_key_tick_and_bridge() {
        let subs = FiberCockpit::new().subscriptions();
        assert!(subs.contains(&EventCategory::PipeCommand));
        assert!(subs.contains(&EventCategory::KeyPress));
        assert!(subs.contains(&EventCategory::Tick));
        // BridgeResponse since the self-poll snapshot returns as BridgeData.
        assert!(subs.contains(&EventCategory::BridgeResponse));
    }

    #[test]
    fn data_sources_is_empty_pipe_fed() {
        // The witness must NOT register curl pollers.
        assert!(FiberCockpit::new().data_sources().is_empty());
    }

    #[test]
    fn command_sources_declares_the_snapshot_helper() {
        let cs = FiberCockpit::new().command_sources();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].tag, "fiber_snapshot");
        assert_eq!(cs[0].module_id, "fiber_cockpit");
        assert!(
            cs[0].argv[0].ends_with("fiber-cockpit-snapshot"),
            "absolute helper path"
        );
        assert!(
            cs[0].argv[0].starts_with('/'),
            "argv[0] must be absolute (no shell expansion)"
        );
        assert!(
            (cs[0].interval_secs - SNAPSHOT_POLL_SECS).abs() < f64::EPSILON,
            "poll interval must match SNAPSHOT_POLL_SECS"
        );
    }

    #[test]
    fn subscriptions_include_bridge_response_for_self_poll() {
        assert!(FiberCockpit::new()
            .subscriptions()
            .contains(&EventCategory::BridgeResponse));
    }

    #[test]
    fn bridge_data_on_snapshot_tag_applies() {
        let mut fc = FiberCockpit::new();
        let data: serde_json::Value = serde_json::from_str(GOLDEN).unwrap();
        let ev = HabitatEvent::BridgeData {
            module_id: "fiber_cockpit".into(),
            tag: "fiber_snapshot".into(),
            data,
        };
        assert!(
            fc.handle_event(&ev),
            "snapshot-tagged BridgeData drives the self-poll feed"
        );
        assert!(!fc.snapshot.campaigns.is_empty());
    }

    #[test]
    fn bridge_data_on_other_tag_ignored() {
        let mut fc = loaded();
        let ev = HabitatEvent::BridgeData {
            module_id: "x".into(),
            tag: "something_else".into(),
            data: serde_json::json!({"campaigns": []}),
        };
        assert!(
            !fc.handle_event(&ev),
            "non-snapshot tags are not this module's feed"
        );
    }

    #[test]
    fn bridge_data_malformed_snapshot_ignored() {
        let mut fc = loaded();
        let before = fc.snapshot.campaigns.len();
        let ev = HabitatEvent::BridgeData {
            module_id: "fiber_cockpit".into(),
            tag: "fiber_snapshot".into(),
            data: serde_json::json!("not a snapshot object"),
        };
        assert!(!fc.handle_event(&ev));
        assert_eq!(
            fc.snapshot.campaigns.len(),
            before,
            "bad value keeps prior state"
        );
    }

    // ── golden fixture parse ─────────────────────────────────────────────
    #[test]
    fn golden_fixture_parses() {
        let fc = loaded();
        assert!(!fc.snapshot.campaigns.is_empty());
        assert_eq!(fc.snapshot.v, 1);
    }

    #[test]
    fn golden_fixture_has_expected_campaigns() {
        let fc = loaded();
        let names: Vec<&str> = fc
            .snapshot
            .campaigns
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(
            names.contains(&"plugin-plans-s1007594"),
            "the plugin-plans campaign should be in the live golden capture"
        );
    }

    #[test]
    fn golden_nodes_carry_scale_and_status() {
        let fc = loaded();
        let any_node = fc
            .snapshot
            .campaigns
            .iter()
            .flat_map(|c| &c.nodes)
            .next()
            .expect("golden has at least one fiber node");
        assert!(!any_node.scale.is_empty());
    }

    // ── apply_payload ────────────────────────────────────────────────────
    #[test]
    fn apply_payload_replaces_not_appends() {
        let mut fc = loaded();
        let before = fc.snapshot.campaigns.len();
        assert!(fc.apply_payload(r#"{"v":1,"campaigns":[{"name":"solo","nodes":[]}]}"#));
        assert_eq!(fc.snapshot.campaigns.len(), 1);
        assert_ne!(before, 1, "fixture had more than one campaign");
        assert_eq!(fc.snapshot.campaigns[0].name, "solo");
    }

    #[test]
    fn apply_payload_malformed_is_ignored_and_keeps_prior() {
        let mut fc = loaded();
        let before = fc.snapshot.campaigns.len();
        assert!(!fc.apply_payload("{not json"));
        assert_eq!(
            fc.snapshot.campaigns.len(),
            before,
            "bad payload must not wipe state"
        );
    }

    #[test]
    fn apply_payload_empty_object_parses_to_empty_snapshot() {
        let mut fc = loaded();
        assert!(fc.apply_payload("{}"));
        assert!(fc.snapshot.campaigns.is_empty());
        assert_eq!(fc.snapshot.v, 0);
    }

    #[test]
    fn apply_payload_resets_stale_counter() {
        let mut fc = loaded();
        fc.ticks_since_data = 99;
        assert!(fc.apply_payload(GOLDEN));
        assert_eq!(fc.ticks_since_data, 0);
    }

    #[test]
    fn apply_payload_drift_tolerant_unknown_fields() {
        // A future helper adds a field the module doesn't know — must still parse.
        let mut fc = FiberCockpit::new();
        assert!(fc.apply_payload(r#"{"v":2,"future_field":42,"campaigns":[]}"#));
        assert_eq!(fc.snapshot.v, 2);
    }

    #[test]
    fn apply_payload_clamps_selection_when_list_shrinks() {
        let mut fc = loaded();
        fc.selected = fc.snapshot.campaigns.len() - 1; // last
        assert!(fc.apply_payload(r#"{"campaigns":[{"name":"only","nodes":[]}]}"#));
        assert_eq!(fc.selected, 0, "selection clamps into the shrunk list");
    }

    #[test]
    fn apply_payload_via_pipe_event() {
        let mut fc = FiberCockpit::new();
        assert!(fc.handle_event(&pipe("fiber-data", GOLDEN)));
        assert!(!fc.snapshot.campaigns.is_empty());
    }

    // ── pipe routing ─────────────────────────────────────────────────────
    #[test]
    fn unknown_pipe_name_is_ignored() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&pipe("some-other-cmd", "whatever")));
    }

    #[test]
    fn fiber_refresh_with_known_name_selects_campaign() {
        let mut fc = loaded();
        let target = fc.snapshot.campaigns.last().unwrap().name.clone();
        assert!(fc.handle_event(&pipe("fiber-refresh", &target)));
        assert_eq!(fc.snapshot.campaigns[fc.selected].name, target);
    }

    #[test]
    fn fiber_refresh_with_unknown_name_is_noop() {
        let mut fc = loaded();
        let before = fc.selected;
        assert!(!fc.handle_event(&pipe("fiber-refresh", "no-such-campaign")));
        assert_eq!(fc.selected, before);
    }

    #[test]
    fn fiber_refresh_empty_payload_is_noop() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&pipe("fiber-refresh", "")));
    }

    #[test]
    fn fiber_refresh_trims_whitespace_in_name() {
        let mut fc = loaded();
        let target = fc.snapshot.campaigns[0].name.clone();
        assert!(fc.handle_event(&pipe("fiber-refresh", &format!("  {target}  "))));
    }

    // ── key navigation ───────────────────────────────────────────────────
    #[test]
    fn j_advances_selection_within_bounds() {
        let mut fc = loaded();
        assert!(
            fc.snapshot.campaigns.len() >= 2,
            "fixture needs >=2 campaigns"
        );
        assert!(fc.handle_event(&keyp('j')));
        assert_eq!(fc.selected, 1);
    }

    #[test]
    fn j_at_last_campaign_is_noop() {
        let mut fc = loaded();
        fc.selected = fc.snapshot.campaigns.len() - 1;
        assert!(
            !fc.handle_event(&keyp('j')),
            "cannot advance past the last campaign"
        );
    }

    #[test]
    fn k_retreats_selection() {
        let mut fc = loaded();
        fc.selected = 1;
        assert!(fc.handle_event(&keyp('k')));
        assert_eq!(fc.selected, 0);
    }

    #[test]
    fn k_at_head_saturates_at_zero() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&keyp('k')));
        assert_eq!(fc.selected, 0);
    }

    #[test]
    fn uppercase_j_k_also_navigate() {
        let mut fc = loaded();
        assert!(fc.handle_event(&keyp('J')));
        assert_eq!(fc.selected, 1);
        assert!(fc.handle_event(&keyp('K')));
        assert_eq!(fc.selected, 0);
    }

    #[test]
    fn l_expands_and_is_idempotent() {
        let mut fc = loaded();
        assert!(fc.handle_event(&keyp('l')));
        assert!(fc.expanded);
        assert!(!fc.handle_event(&keyp('l')), "second expand is a no-op");
    }

    #[test]
    fn enter_newline_also_expands() {
        // N1/F12: Enter must work even though the host only forwards Char — the host
        // diff maps Enter→'\n'; this proves the module honours it.
        let mut fc = loaded();
        assert!(fc.handle_event(&keyp('\n')));
        assert!(fc.expanded);
    }

    #[test]
    fn h_collapses_from_expanded() {
        let mut fc = loaded();
        fc.expanded = true;
        assert!(fc.handle_event(&keyp('h')));
        assert!(!fc.expanded);
    }

    #[test]
    fn h_in_browse_is_noop() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&keyp('h')));
    }

    #[test]
    fn g_resets_to_head_and_browse() {
        let mut fc = loaded();
        fc.selected = 2;
        fc.expanded = true;
        assert!(fc.handle_event(&keyp('g')));
        assert_eq!(fc.selected, 0);
        assert!(!fc.expanded);
    }

    #[test]
    fn g_when_already_at_head_browse_is_noop() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&keyp('g')));
    }

    #[test]
    fn unknown_key_is_ignored() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&keyp('z')));
    }

    #[test]
    fn navigation_on_empty_snapshot_never_panics() {
        let mut fc = FiberCockpit::new();
        assert!(!fc.handle_event(&keyp('j')));
        assert!(!fc.handle_event(&keyp('k')));
        assert!(!fc.handle_event(&keyp('g')));
        // expand on empty is allowed (toggles flag) but render must still be safe
        fc.handle_event(&keyp('l'));
        let _ = fc.render(20, 80);
    }

    // ── tick / stale ─────────────────────────────────────────────────────
    #[test]
    fn tick_increments_stale_counter_without_render() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&HabitatEvent::Tick { tick: 1 }));
        assert_eq!(fc.ticks_since_data, 1);
    }

    #[test]
    fn tick_counter_saturates() {
        let mut fc = FiberCockpit::new();
        fc.ticks_since_data = u64::MAX;
        fc.handle_event(&HabitatEvent::Tick { tick: 1 });
        assert_eq!(fc.ticks_since_data, u64::MAX, "saturating add, no overflow");
    }

    #[test]
    fn stale_seconds_uses_poll_floor() {
        let mut fc = FiberCockpit::new();
        fc.poll_secs = 0.0; // misconfig
        fc.ticks_since_data = 10;
        assert!(
            (fc.stale_seconds() - 10.0).abs() < f64::EPSILON,
            "floor of 1.0 applied"
        );
    }

    #[test]
    fn bridge_events_are_ignored() {
        let mut fc = loaded();
        assert!(!fc.handle_event(&HabitatEvent::BridgeData {
            module_id: "x".into(),
            tag: "y".into(),
            data: serde_json::Value::Null,
        }));
        assert!(!fc.handle_event(&HabitatEvent::BridgeError {
            module_id: "x".into(),
            tag: "y".into(),
        }));
    }

    // ── selection clamp ──────────────────────────────────────────────────
    #[test]
    fn clamp_selection_on_empty_is_zero() {
        let mut fc = FiberCockpit::new();
        fc.selected = 7;
        fc.clamp_selection();
        assert_eq!(fc.selected, 0);
    }

    #[test]
    fn clamp_selection_caps_at_last_index() {
        let mut fc = loaded();
        fc.selected = 999;
        fc.clamp_selection();
        assert_eq!(fc.selected, fc.snapshot.campaigns.len() - 1);
    }

    // ── select_by_name ───────────────────────────────────────────────────
    #[test]
    fn select_by_name_hit_returns_true() {
        let mut fc = loaded();
        let name = fc.snapshot.campaigns[0].name.clone();
        assert!(fc.select_by_name(&name));
        assert_eq!(fc.selected, 0);
    }

    #[test]
    fn select_by_name_miss_returns_false() {
        let mut fc = loaded();
        assert!(!fc.select_by_name("ghost-campaign"));
    }

    // ── badges / glyphs ──────────────────────────────────────────────────
    #[test]
    fn scale_badge_colours_each_scale() {
        assert!(FiberCockpit::scale_badge("macro").contains(MAG));
        assert!(FiberCockpit::scale_badge("meso").contains(CYN));
        assert!(FiberCockpit::scale_badge("micro").contains(D));
        assert!(FiberCockpit::scale_badge("raw").contains(RED));
        assert!(FiberCockpit::scale_badge("weird").contains("???"));
    }

    #[test]
    fn status_glyph_maps_states() {
        assert!(FiberCockpit::status_glyph("ok").contains(ICON_CHECK));
        assert!(FiberCockpit::status_glyph("degraded").contains(YEL));
        assert!(FiberCockpit::status_glyph("failed").contains(ICON_CROSS));
        assert!(FiberCockpit::status_glyph("").contains('-'));
    }

    // ── render ───────────────────────────────────────────────────────────
    #[test]
    fn render_empty_shows_waiting_banner() {
        let fc = FiberCockpit::new();
        let lines = fc.render(20, 80);
        let joined = lines.iter().map(|l| l.content.as_str()).collect::<String>();
        assert!(
            joined.contains("waiting"),
            "empty cockpit shows a waiting hint"
        );
    }

    #[test]
    fn render_browse_lists_campaigns_with_marker() {
        let fc = loaded();
        let lines = fc.render(30, 100);
        let joined = lines.iter().map(|l| l.content.as_str()).collect::<String>();
        assert!(joined.contains("FIBER"), "header present");
        assert!(joined.contains(&fc.snapshot.campaigns[0].name));
        assert!(joined.contains('>'), "selection marker present in browse");
    }

    #[test]
    fn render_expanded_shows_fiber_tree() {
        let mut fc = loaded();
        // pick a campaign that has nodes
        let idx = fc
            .snapshot
            .campaigns
            .iter()
            .position(|c| !c.nodes.is_empty())
            .unwrap();
        fc.selected = idx;
        fc.expanded = true;
        let lines = fc.render(40, 110);
        let joined = lines.iter().map(|l| l.content.as_str()).collect::<String>();
        let loop_name = &fc.snapshot.campaigns[idx].nodes[0].r#loop;
        assert!(
            joined.contains(loop_name.as_str()),
            "expanded view shows fiber node loops"
        );
        assert!(
            joined.contains("back"),
            "expanded view shows the [h] back hint"
        );
    }

    #[test]
    fn render_shows_lease_table_when_present() {
        let mut fc = FiberCockpit::new();
        fc.apply_payload(r#"{"campaigns":[{"name":"c","nodes":[]}],
            "leases":[{"resource":"git.index.workspace","owner":"pane-1","ttl_remaining":120,"note":"x","expired":false}]}"#);
        let joined = fc
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("leases"));
        assert!(joined.contains("git.index.workspace"));
    }

    #[test]
    fn render_marks_expired_lease() {
        let mut fc = FiberCockpit::new();
        fc.apply_payload(
            r#"{"campaigns":[{"name":"c","nodes":[]}],
            "leases":[{"resource":"r","owner":"o","ttl_remaining":0,"note":"","expired":true}]}"#,
        );
        let joined = fc
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("EXPIRED"));
    }

    #[test]
    fn render_arming_armed_vs_unarmed() {
        let mut fc = FiberCockpit::new();
        fc.apply_payload(
            r#"{"campaigns":[{"name":"c","nodes":[]}],
            "arming":[{"key":"factory.authorize.foo","value":"armed"}]}"#,
        );
        let joined = fc
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("ARM"));
        assert!(joined.contains("armed"));
    }

    #[test]
    fn render_shows_error_tag_when_helper_errored() {
        let mut fc = FiberCockpit::new();
        fc.apply_payload(
            r#"{"campaigns":[{"name":"c","nodes":[]}],"errors":["atuin-unreachable"]}"#,
        );
        let joined = fc
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(joined.contains("!1"), "error count tag in header");
    }

    #[test]
    fn render_truncated_campaign_shows_prune_hint() {
        let mut fc = FiberCockpit::new();
        fc.apply_payload(r#"{"campaigns":[{"name":"big","nodes":[{"loop":"a","scale":"micro"}],"truncated":true}]}"#);
        fc.expanded = true;
        let joined = fc
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(
            joined.contains("more"),
            "truncation surfaces a +more/prune hint"
        );
    }

    #[test]
    fn render_never_panics_across_tiny_and_wide_dims() {
        let fc = loaded();
        for (r, c) in [(0, 0), (1, 1), (2, 10), (5, 200), (60, 300)] {
            let _ = fc.render(r, c);
        }
    }

    #[test]
    fn render_handles_raw_node_gracefully() {
        let mut fc = FiberCockpit::new();
        fc.apply_payload(r#"{"campaigns":[{"name":"c","nodes":[{"loop":"unparseable","scale":"raw","status":"raw"}]}]}"#);
        fc.expanded = true;
        let joined = fc
            .render(20, 90)
            .iter()
            .map(|l| l.content.as_str())
            .collect::<String>();
        assert!(
            joined.contains("RAW"),
            "raw node renders with a RAW badge, no crash"
        );
    }

    // ── serialize / restore ──────────────────────────────────────────────
    #[test]
    fn serialize_restore_round_trips_cursor() {
        let mut fc = loaded();
        fc.selected = 1;
        fc.expanded = true;
        let state = fc.serialize_state().expect("serialize");
        let mut fresh = FiberCockpit::new();
        fresh.restore_state(&state);
        assert_eq!(fresh.selected, 1);
        assert!(fresh.expanded);
    }

    #[test]
    fn restore_state_ignores_garbage() {
        let mut fc = loaded();
        fc.selected = 2;
        fc.restore_state("not a tuple");
        assert_eq!(fc.selected, 2, "garbage restore leaves cursor intact");
    }

    #[test]
    fn restore_does_not_carry_snapshot() {
        // State is cursor-only; snapshot must come from a fresh feed (no stale data revival).
        let fc = loaded();
        let state = fc.serialize_state().unwrap();
        let mut fresh = FiberCockpit::new();
        fresh.restore_state(&state);
        assert!(
            fresh.snapshot.campaigns.is_empty(),
            "restore must not resurrect snapshot data"
        );
    }
    // Witness-boundary (no write verbs in helper + module) is enforced by the
    // external grep gate in the plan §9.8 smoke, not an in-source test (a literal
    // forbidden-string list would match itself). See build step 6.
}
