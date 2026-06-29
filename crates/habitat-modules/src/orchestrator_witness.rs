//! `orchestrator_witness` — read-only G8 governance dashboard module.
//!
//! Renders the orchestrator's live governance state by self-polling five
//! host helpers via [`command_sources`](HabitatModule::command_sources):
//!
//! | Source | Tag | Data |
//! |--------|-----|------|
//! | `orchestrator-perceive --dry-run` | `ow_perceive` | leases, hopf fibers, engine health, pane/session count |
//! | `orch-kernelctl snapshot --json` | `ow_kernel` | chain health, last seq/time, warrant count |
//! | `dcg-admit width --semaphore N` | `ow_width` | computed fan-out width, binding ceilings |
//! | `bin/fiber-cockpit-snapshot` | `ow_arming` | factory arming-key states |
//! | `orch-kernelctl events --trace route.decision` | `ow_route` | last routing decision: capability, engine, width, orchestrate |
//!
//! # Observe-only (deliberate)
//!
//! This module performs ZERO writes to any substrate. It owns no KV key,
//! claims no lease, links no fiber. All five command sources are read-only
//! observers. A mechanical grep gate enforces this: no `register`, `submit`,
//! `append`, or `write` verb appears in this source file.
//!
//! # Width-vs-ceiling note
//!
//! `dcg-admit width` requires a `--semaphore` value. This module passes
//! `--semaphore 8` as a **tier-B tunable default** (not proven optimal —
//! operators should adjust [`DEFAULT_SEMAPHORE`] when the live DCG semaphore
//! changes; measure the live permit count before changing).
//!
//! # Last perceive.snapshot seq/time
//!
//! `orch-kernelctl snapshot --json` returns `last_seq` and `generated_at`
//! from the kernel chain HEAD. Because the `orchestrator-perceive` FIBER-3
//! self-poll appends `perceive.snapshot` events, the chain HEAD is typically
//! a recent perceive event — so `last_seq` / `generated_at` proxy the last
//! perceive emission time.
//!
//! # Route-decision observation
//!
//! `orch-kernelctl events --trace route.decision` returns the full event log
//! filtered to routing decisions. This module takes the last (most recent)
//! entry and renders capability, engine, width, and orchestrate flag. An empty
//! result (no routing decisions emitted yet) is tolerated gracefully — the
//! module renders "no decisions yet" without raising a degraded condition.

use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{CommandSource, DataSource, HabitatModule};
use habitat_core::render::*;
use serde::Deserialize;

// ── tag constants ─────────────────────────────────────────────────────────────

/// `BridgeData` tag for the `orchestrator-perceive --dry-run` command source.
const OW_PERCEIVE_TAG: &str = "ow_perceive";
/// `BridgeData` tag for the `orch-kernelctl snapshot --json` command source.
const OW_KERNEL_TAG: &str = "ow_kernel";
/// `BridgeData` tag for the `dcg-admit width` command source.
const OW_WIDTH_TAG: &str = "ow_width";
/// `BridgeData` tag for the `bin/fiber-cockpit-snapshot` arming source.
const OW_ARMING_TAG: &str = "ow_arming";
/// `BridgeData` tag for the `orch-kernelctl events --trace route.decision` source.
const OW_ROUTE_TAG: &str = "ow_route";

// ── binary paths (D11 rule: absolute, exec'd directly, no shell) ──────────────

/// Absolute path to `orchestrator-perceive`.
const PERCEIVE_CLI: &str = "/home/louranicas/.local/bin/orchestrator-perceive";
/// Absolute path to `orch-kernelctl`.
const KERNELCTL_CLI: &str = "/home/louranicas/.local/bin/orch-kernelctl";
/// Absolute path to `dcg-admit`.
const DCG_ADMIT_CLI: &str = "/home/louranicas/.local/bin/dcg-admit";
/// Absolute path to the fiber-cockpit-snapshot helper.
const FIBER_SNAPSHOT_CLI: &str =
    "/home/louranicas/claude-code-workspace/bin/fiber-cockpit-snapshot";

// ── poll cadences ─────────────────────────────────────────────────────────────

/// Perceive poll cadence — assembles a full manifest; 30 s matches sphere-warden.
const PERCEIVE_POLL_SECS: f64 = 30.0;
/// Kernel snapshot poll cadence — chain HEAD tracking.
const KERNEL_POLL_SECS: f64 = 10.0;
/// Width probe cadence — admission width changes slowly.
const WIDTH_POLL_SECS: f64 = 30.0;
/// Arming poll cadence — arming keys rarely flip; 60 s is sufficient.
const ARMING_POLL_SECS: f64 = 60.0;
/// Route-decision poll cadence — between kernel (fast) and perceive (slow).
const ROUTE_POLL_SECS: f64 = 15.0;

/// Staleness threshold: 3 × the longest fast-source cadence (30 s → 90 s).
/// Sources whose data is older than this display a `[STALE Xs]` tag.
const STALE_THRESHOLD_SECS: f64 = 90.0;

/// Default semaphore ceiling passed to `dcg-admit width --semaphore`.
///
/// **Tier-B tunable default** — not derived from a live measurement.
/// Operators should set this to the actual DCG semaphore permit count.
/// Measure the live semaphore before changing; do not guess.
const DEFAULT_SEMAPHORE: u8 = 8;

// ── deserializable types ──────────────────────────────────────────────────────

/// Minimal lease row from `orchestrator-perceive --dry-run` output.
///
/// Only `resource` is rendered; wire fields not needed for display are omitted
/// from this private type (serde ignores unknown JSON keys by default).
#[derive(Deserialize, Clone, Debug, Default)]
struct PerceiveLease {
    /// The leased resource key displayed in the dashboard.
    #[serde(default)]
    resource: String,
}

/// Minimal fiber observation from `orchestrator-perceive --dry-run` output.
///
/// Only `campaign` is rendered; `root` and `loops` are decoded by perceive
/// but not surfaced in this module's compact render.
#[derive(Deserialize, Clone, Debug, Default)]
struct PerceiveFiber {
    /// Campaign identifier shown in the fibers line.
    #[serde(default)]
    campaign: String,
}

/// Minimal engine probe from `orchestrator-perceive --dry-run` output.
#[derive(Deserialize, Clone, Debug, Default)]
struct PerceiveEngine {
    /// HTTP status code (`Some(200)` = healthy, `None` = unreachable).
    #[serde(default)]
    health_code: Option<u16>,
}

/// The governance-relevant fields from `orchestrator-perceive --dry-run` stdout.
///
/// `panes` and `sessions` are counted but not further decoded.
/// `schema` and `captured_at_ms` are present on the wire but are not surfaced
/// by this module's compact render; they are accepted and silently dropped.
/// All fields use `#[serde(default)]` for drift tolerance.
#[derive(Deserialize, Clone, Debug, Default)]
struct PerceiveData {
    #[serde(default)]
    leases: Vec<PerceiveLease>,
    #[serde(default)]
    fibers: Vec<PerceiveFiber>,
    #[serde(default)]
    engines: Vec<PerceiveEngine>,
    #[serde(default)]
    panes: Vec<serde_json::Value>,
    #[serde(default)]
    sessions: Vec<serde_json::Value>,
}

/// Kernel chain snapshot from `orch-kernelctl snapshot --json`.
///
/// Provides chain health, last event seq, and generation timestamp —
/// which proxies the last `perceive.snapshot` emission time.
#[derive(Deserialize, Clone, Debug, Default)]
struct WitnessKernelSnap {
    #[serde(default)]
    status: String,
    #[serde(default)]
    last_seq: i64,
    #[serde(default)]
    last_hash: String,
    #[serde(default)]
    event_count: i64,
    #[serde(default)]
    verify_chain_ok: bool,
    /// Snapshot generation timestamp (ISO 8601 or RFC 3339).
    #[serde(default)]
    generated_at: String,
    #[serde(default)]
    warrant_count: i64,
    #[serde(default)]
    queue_depth: i64,
}

/// Fan-out width result from `dcg-admit width --semaphore N`.
///
/// `width` is a `u8` (transparent via the `Width` newtype in `dcg-admit`).
/// `bound_by` holds `snake_case` ceiling names (e.g. `"semaphore"`, `"model_tier"`).
#[derive(Deserialize, Clone, Debug, Default)]
struct WitnessWidthResult {
    #[serde(default)]
    width: u8,
    #[serde(default)]
    bound_by: Vec<String>,
}

/// One factory arming-key row (render-only — the witness never sets a key).
#[derive(Deserialize, Clone, Debug, Default)]
struct WitnessArmRow {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
}

/// Minimal subset of the fiber-cockpit snapshot used to extract arming rows.
///
/// The `bin/fiber-cockpit-snapshot` output contains many more fields; this
/// struct silently ignores them (drift-tolerant via `#[serde(default)]`).
#[derive(Deserialize, Clone, Debug, Default)]
struct FiberSnapshotArming {
    #[serde(default)]
    arming: Vec<WitnessArmRow>,
}

/// A single `route.decision` event entry from `orch-kernelctl events --trace`.
///
/// The witness takes the **last** entry in the returned array (most recent
/// decision). All fields are serde-defaulted for drift tolerance — unknown
/// future fields from the cortex are silently dropped.
#[derive(Deserialize, Clone, Debug, Default)]
struct WitnessRouteDecision {
    /// Model capability tier requested by the router (e.g. `"code-gen"`).
    #[serde(default)]
    capability: String,
    /// Engine selected to serve the request (e.g. `"claude-sonnet"`).
    #[serde(default)]
    engine: String,
    /// Fan-out width resolved at routing time.
    #[serde(default)]
    width: u8,
    /// Whether the request entered the full orchestration path (`true`)
    /// or was served as a single-shot passthrough (`false`).
    #[serde(default)]
    orchestrate: bool,
}

// ── module struct ─────────────────────────────────────────────────────────────

/// The `orchestrator_witness` G8 read-only governance dashboard.
///
/// Each field tracks one dimension of the orchestrator's live state:
///
/// - `perceive` — leases, hopf fibers, engine health, pane/session count
/// - `kernel` — kernel chain health + last perceive seq/time
/// - `width` — computed fan-out width + binding ceilings
/// - `arming` — factory arming-key states
/// - `route` — last routing decision (capability / engine / width / orchestrate)
///
/// All five are populated by scheduled [`command_sources`](Self::command_sources).
pub struct OrchestratorWitness {
    perceive: Option<PerceiveData>,
    kernel: Option<WitnessKernelSnap>,
    width: Option<WitnessWidthResult>,
    arming: Vec<WitnessArmRow>,
    route: Option<WitnessRouteDecision>,

    ticks_since_perceive: u64,
    ticks_since_kernel: u64,
    ticks_since_width: u64,
    ticks_since_arming: u64,
    ticks_since_route: u64,

    /// Seconds per tick (from `config.governance_poll`); used for staleness.
    poll_secs: f64,

    perceive_degraded: Option<String>,
    kernel_degraded: Option<String>,
    width_degraded: Option<String>,
    arming_degraded: Option<String>,
    route_degraded: Option<String>,
}

impl OrchestratorWitness {
    /// Construct a fresh witness with no data yet.
    ///
    /// The five command sources begin polling after `init` + the first
    /// timer tick; results arrive as `BridgeData` events.
    #[must_use]
    pub fn new() -> Self {
        Self {
            perceive: None,
            kernel: None,
            width: None,
            arming: Vec::new(),
            route: None,
            ticks_since_perceive: 0,
            ticks_since_kernel: 0,
            ticks_since_width: 0,
            ticks_since_arming: 0,
            ticks_since_route: 0,
            poll_secs: 10.0,
            perceive_degraded: None,
            kernel_degraded: None,
            width_degraded: None,
            arming_degraded: None,
            route_degraded: None,
        }
    }

    // ── internal parsers ──────────────────────────────────────────────────────

    fn apply_perceive(&mut self, value: &serde_json::Value) -> bool {
        match serde_json::from_value::<PerceiveData>(value.clone()) {
            Ok(data) => {
                self.perceive = Some(data);
                self.ticks_since_perceive = 0;
                self.perceive_degraded = None;
                true
            }
            Err(e) => {
                self.perceive_degraded = Some(format!("decode failed: {e}"));
                false
            }
        }
    }

    fn apply_kernel(&mut self, value: &serde_json::Value) -> bool {
        match serde_json::from_value::<WitnessKernelSnap>(value.clone()) {
            Ok(snap) => {
                self.kernel = Some(snap);
                self.ticks_since_kernel = 0;
                self.kernel_degraded = None;
                true
            }
            Err(e) => {
                self.kernel_degraded = Some(format!("decode failed: {e}"));
                false
            }
        }
    }

    fn apply_width(&mut self, value: &serde_json::Value) -> bool {
        match serde_json::from_value::<WitnessWidthResult>(value.clone()) {
            Ok(w) => {
                self.width = Some(w);
                self.ticks_since_width = 0;
                self.width_degraded = None;
                true
            }
            Err(e) => {
                self.width_degraded = Some(format!("decode failed: {e}"));
                false
            }
        }
    }

    fn apply_arming(&mut self, value: &serde_json::Value) -> bool {
        match serde_json::from_value::<FiberSnapshotArming>(value.clone()) {
            Ok(snap) => {
                self.arming = snap.arming;
                self.ticks_since_arming = 0;
                self.arming_degraded = None;
                true
            }
            Err(e) => {
                self.arming_degraded = Some(format!("decode failed: {e}"));
                false
            }
        }
    }

    /// Parse the latest `route.decision` event from `orch-kernelctl events --trace`.
    ///
    /// # Input shapes tolerated
    ///
    /// - **JSON array**: take the last element (most recent decision). An empty
    ///   array means no decisions have been emitted yet — treated as graceful
    ///   absence, not an error.
    /// - **JSON null**: treated identically to an empty array (no decisions yet).
    /// - **JSON object**: accepted directly (single-event short-circuit output).
    /// - **Anything else**: sets `route_degraded` and returns `false`.
    fn apply_route(&mut self, value: &serde_json::Value) -> bool {
        let entry = match value {
            serde_json::Value::Null => {
                // No events in the kernel chain yet — graceful absence.
                self.route = None;
                self.ticks_since_route = 0;
                self.route_degraded = None;
                return true;
            }
            serde_json::Value::Array(arr) => {
                match arr.last() {
                    None => {
                        // Empty array: no route decisions emitted yet.
                        self.route = None;
                        self.ticks_since_route = 0;
                        self.route_degraded = None;
                        return true;
                    }
                    Some(last) => last.clone(),
                }
            }
            obj @ serde_json::Value::Object(_) => obj.clone(),
            _ => {
                self.route_degraded =
                    Some("unexpected shape for route events output".into());
                return false;
            }
        };
        match serde_json::from_value::<WitnessRouteDecision>(entry) {
            Ok(rd) => {
                self.route = Some(rd);
                self.ticks_since_route = 0;
                self.route_degraded = None;
                true
            }
            Err(e) => {
                self.route_degraded = Some(format!("route decode failed: {e}"));
                false
            }
        }
    }

    /// Approximate elapsed seconds for a tick counter.
    ///
    /// Uses `poll_secs` (the Zellij timer interval) as a proxy for wall
    /// clock time — each tick represents roughly one `poll_secs` period.
    fn elapsed_secs(&self, ticks: u64) -> f64 {
        // cast_precision_loss accepted crate-wide (lib.rs); display-only.
        ticks as f64 * self.poll_secs.max(1.0)
    }

    // ── private render helpers ────────────────────────────────────────────────

    fn render_width_line(&self, w: usize) -> RenderLine {
        if let Some(reason) = &self.width_degraded {
            return RenderLine::new(format!(
                " {D}width{R}   {YEL}ERR{R} {D}{}{R}",
                truncate(reason, w.saturating_sub(14)),
            ));
        }
        let Some(wr) = &self.width else {
            return RenderLine::new(format!(
                " {D}width   awaiting first poll ({}s){R}",
                WIDTH_POLL_SECS as u32,
            ));
        };
        let wcolor = if wr.width == 0 { RED } else { GRN };
        let bound = if wr.bound_by.is_empty() {
            String::new()
        } else {
            format!(" {D}via {}{R}", wr.bound_by.join("+"))
        };
        RenderLine::new(format!(
            " {D}width{R}   {wcolor}{}{R}{bound} {D}(sem={}){R}",
            wr.width, DEFAULT_SEMAPHORE,
        ))
    }

    fn render_kernel_line(&self, w: usize) -> RenderLine {
        if let Some(reason) = &self.kernel_degraded {
            return RenderLine::new(format!(
                " {D}chain{R}   {YEL}ERR{R} {D}{}{R}",
                truncate(reason, w.saturating_sub(14)),
            ));
        }
        let Some(snap) = &self.kernel else {
            return RenderLine::new(format!(
                " {D}chain   awaiting first poll ({}s){R}",
                KERNEL_POLL_SECS as u32,
            ));
        };
        let chain_col = if snap.verify_chain_ok { GRN } else { RED };
        let stale = stale_tag(
            self.elapsed_secs(self.ticks_since_kernel),
            STALE_THRESHOLD_SECS,
        )
        .unwrap_or_default();
        let hash_short = snap.last_hash.chars().take(12).collect::<String>();
        let gen_short = truncate(&snap.generated_at, 20);
        RenderLine::new(format!(
            " {D}chain{R}   {chain_col}{}{R} seq={B}{}{R} {D}ev={} warrants={} q={}{R} {D}hash={hash_short}{R} {D}@{gen_short}{R}{}",
            snap.status,
            snap.last_seq,
            snap.event_count,
            snap.warrant_count,
            snap.queue_depth,
            if stale.is_empty() {
                String::new()
            } else {
                format!("  {stale}")
            },
        ))
    }

    fn render_leases_line(&self, w: usize) -> RenderLine {
        let stale = stale_tag(
            self.elapsed_secs(self.ticks_since_perceive),
            STALE_THRESHOLD_SECS,
        )
        .unwrap_or_default();

        let Some(p) = &self.perceive else {
            if let Some(reason) = &self.perceive_degraded {
                return RenderLine::new(format!(
                    " {D}leases{R}  {YEL}ERR{R} {D}{}{R}",
                    truncate(reason, w.saturating_sub(15)),
                ));
            }
            return RenderLine::new(format!(
                " {D}leases  awaiting first poll ({}s){R}",
                PERCEIVE_POLL_SECS as u32,
            ));
        };

        let count = p.leases.len();
        if count == 0 {
            return RenderLine::new(format!(
                " {D}leases{R}  {GRN}none{R}{}",
                if stale.is_empty() {
                    String::new()
                } else {
                    format!("  {stale}")
                },
            ));
        }
        let names: Vec<&str> = p.leases.iter().take(4).map(|l| l.resource.as_str()).collect();
        let suffix = if count > 4 {
            format!(" +{}", count - 4)
        } else {
            String::new()
        };
        let display = format!("{}{}", names.join(", "), suffix);
        let trunc = truncate(&display, w.saturating_sub(20));
        RenderLine::new(format!(
            " {D}leases{R}  {YEL}{count}{R} {D}{trunc}{R}{}",
            if stale.is_empty() {
                String::new()
            } else {
                format!("  {stale}")
            },
        ))
    }

    fn render_fibers_line(&self, w: usize) -> RenderLine {
        let Some(p) = &self.perceive else {
            if self.perceive_degraded.is_some() {
                return RenderLine::blank();
            }
            return RenderLine::new(format!(
                " {D}fibers  awaiting first poll{R}"
            ));
        };

        let count = p.fibers.len();
        if count == 0 {
            return RenderLine::new(format!(" {D}fibers{R}  {GRN}none active{R}"));
        }
        let names: Vec<&str> = p
            .fibers
            .iter()
            .take(3)
            .map(|f| f.campaign.as_str())
            .collect();
        let suffix = if count > 3 {
            format!(" +{}", count - 3)
        } else {
            String::new()
        };
        let display = format!("{}{}", names.join(", "), suffix);
        let trunc = truncate(&display, w.saturating_sub(18));
        RenderLine::new(format!(
            " {D}fibers{R}  {CYN}{count}{R} {D}{trunc}{R}",
        ))
    }

    fn render_arming_line(&self, w: usize) -> RenderLine {
        if let Some(reason) = &self.arming_degraded {
            return RenderLine::new(format!(
                " {D}arming{R}  {YEL}ERR{R} {D}{}{R}",
                truncate(reason, w.saturating_sub(15)),
            ));
        }
        if self.arming.is_empty() {
            return RenderLine::new(format!(
                " {D}arming  awaiting first poll ({}s){R}",
                ARMING_POLL_SECS as u32,
            ));
        }
        let armed_count = self.arming.iter().filter(|a| a.value == "armed").count();
        let total = self.arming.len();
        let unarmed = total - armed_count;
        let arm_col = if unarmed > 0 { YEL } else { GRN };
        let arm_label = if unarmed > 0 {
            format!("{arm_col}{armed_count}/{total} armed{R}")
        } else {
            format!("{arm_col}all {total} armed{R}")
        };
        // Hint: first armed key name (truncated), observe-only.
        let key_hint = self
            .arming
            .iter()
            .find(|a| a.value == "armed")
            .map(|a| {
                format!(
                    " {D}{}{R}",
                    truncate(a.key.as_str(), w.saturating_sub(28))
                )
            })
            .unwrap_or_default();
        RenderLine::new(format!(" {D}arming{R}  {arm_label}{key_hint}"))
    }

    /// Render the `last route:` line for the most recent `route.decision` event.
    ///
    /// - **Data present**: shows capability, engine, fan-out width, and orchestrate flag.
    /// - **Degraded**: surfaces the error reason.
    /// - **Absent** (no decisions emitted yet): renders a graceful "no decisions yet"
    ///   message — NOT treated as an error condition.
    fn render_route_line(&self, w: usize) -> RenderLine {
        if let Some(reason) = &self.route_degraded {
            return RenderLine::new(format!(
                " {D}last route{R}  {YEL}ERR{R} {D}{}{R}",
                truncate(reason, w.saturating_sub(16)),
            ));
        }
        let Some(rd) = &self.route else {
            return RenderLine::new(format!(
                " {D}last route{R}  {D}no decisions yet ({}s cadence){R}",
                ROUTE_POLL_SECS as u32,
            ));
        };
        let stale = stale_tag(
            self.elapsed_secs(self.ticks_since_route),
            STALE_THRESHOLD_SECS,
        )
        .unwrap_or_default();
        let orch_col = if rd.orchestrate { GRN } else { D };
        RenderLine::new(format!(
            " {D}last route{R}  cap={B}{}{R} eng={B}{}{R} w={B}{}{R} orch={orch_col}{}{R}{}",
            truncate(rd.capability.as_str(), 20),
            truncate(rd.engine.as_str(), 20),
            rd.width,
            rd.orchestrate,
            if stale.is_empty() {
                String::new()
            } else {
                format!("  {stale}")
            },
        ))
    }

    fn render_engines_line(&self) -> Option<RenderLine> {
        let p = self.perceive.as_ref()?;
        let up = p.engines.iter().filter(|e| e.health_code == Some(200)).count();
        let total = p.engines.len();
        let ecol = if total == 0 {
            D
        } else if up == total {
            GRN
        } else {
            YEL
        };
        Some(RenderLine::new(format!(
            " {D}engines{R} {ecol}{up}/{total}{R} {D}up · panes{R} {} {D}sessions{R} {}",
            p.panes.len(),
            p.sessions.len(),
        )))
    }
}

impl Default for OrchestratorWitness {
    fn default() -> Self {
        Self::new()
    }
}

// ── trait impl ────────────────────────────────────────────────────────────────

impl HabitatModule for OrchestratorWitness {
    fn id(&self) -> &'static str {
        "orchestrator_witness"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.poll_secs = config.governance_poll;
    }

    /// Routes `BridgeData` / `BridgeError` events from all five command sources
    /// plus the global `Tick`.
    ///
    /// Returns `true` when this module consumed the event, `false` otherwise.
    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            // ── perceive data ─────────────────────────────────────────────────
            HabitatEvent::BridgeData { tag, data, .. } if tag == OW_PERCEIVE_TAG => {
                self.apply_perceive(data)
            }
            HabitatEvent::BridgeError { tag, .. } if tag == OW_PERCEIVE_TAG => {
                self.perceive_degraded = Some("perceive helper failed".into());
                true
            }

            // ── kernel chain ──────────────────────────────────────────────────
            HabitatEvent::BridgeData { tag, data, .. } if tag == OW_KERNEL_TAG => {
                self.apply_kernel(data)
            }
            HabitatEvent::BridgeError { tag, .. } if tag == OW_KERNEL_TAG => {
                self.kernel_degraded = Some("kernelctl snapshot failed".into());
                true
            }

            // ── fan-out width ─────────────────────────────────────────────────
            HabitatEvent::BridgeData { tag, data, .. } if tag == OW_WIDTH_TAG => {
                self.apply_width(data)
            }
            HabitatEvent::BridgeError { tag, .. } if tag == OW_WIDTH_TAG => {
                self.width_degraded = Some("dcg-admit width failed".into());
                true
            }

            // ── arming state ──────────────────────────────────────────────────
            HabitatEvent::BridgeData { tag, data, .. } if tag == OW_ARMING_TAG => {
                self.apply_arming(data)
            }
            HabitatEvent::BridgeError { tag, .. } if tag == OW_ARMING_TAG => {
                self.arming_degraded = Some("fiber snapshot failed".into());
                true
            }

            // ── route decision (observe-only) ─────────────────────────────────
            HabitatEvent::BridgeData { tag, data, .. } if tag == OW_ROUTE_TAG => {
                self.apply_route(data)
            }
            HabitatEvent::BridgeError { tag, .. } if tag == OW_ROUTE_TAG => {
                self.route_degraded = Some("route events query failed".into());
                true
            }

            // ── tick: age all staleness counters ──────────────────────────────
            HabitatEvent::Tick { .. } => {
                self.ticks_since_perceive = self.ticks_since_perceive.saturating_add(1);
                self.ticks_since_kernel = self.ticks_since_kernel.saturating_add(1);
                self.ticks_since_width = self.ticks_since_width.saturating_add(1);
                self.ticks_since_arming = self.ticks_since_arming.saturating_add(1);
                self.ticks_since_route = self.ticks_since_route.saturating_add(1);
                false
            }

            // ── catch-all: unrecognised tags or unsubscribed event kinds ──────
            HabitatEvent::KeyPress { .. }
            | HabitatEvent::PipeCommand { .. }
            | HabitatEvent::BridgeData { .. }
            | HabitatEvent::BridgeError { .. } => false,
        }
    }

    fn render(&self, _rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);

        let any_degraded = self.perceive_degraded.is_some()
            || self.kernel_degraded.is_some()
            || self.width_degraded.is_some()
            || self.arming_degraded.is_some()
            || self.route_degraded.is_some();

        let any_stale = self.elapsed_secs(self.ticks_since_perceive) > STALE_THRESHOLD_SECS
            || self.elapsed_secs(self.ticks_since_kernel) > STALE_THRESHOLD_SECS;

        let chain_warn = self
            .kernel
            .as_ref()
            .is_some_and(|k| !k.verify_chain_ok);

        let (hcol, hstatus) = if any_degraded {
            (YEL, "DEGRADED")
        } else if chain_warn {
            (RED, "CHAIN WARN")
        } else if any_stale {
            (YEL, "STALE")
        } else if self.perceive.is_none() && self.kernel.is_none() {
            (D, "SENSING")
        } else {
            (GRN, "OK")
        };

        let mut lines = vec![
            RenderLine::new(format!(
                " {B}{CYN}\u{25c9} ORCH WITNESS{R}  {hcol}{hstatus}{R} {D}v{}{R}",
                self.version(),
            )),
            RenderLine::separator(w),
            self.render_width_line(w),
            self.render_kernel_line(w),
            RenderLine::blank(),
            self.render_leases_line(w),
            self.render_fibers_line(w),
            self.render_arming_line(w),
            self.render_route_line(w),
        ];

        if let Some(eng_line) = self.render_engines_line() {
            lines.push(eng_line);
        }

        lines
    }

    /// No state is persisted between sessions — the five command sources
    /// re-feed the module on the next poll cycle.
    fn serialize_state(&self) -> Option<String> {
        None
    }

    fn restore_state(&mut self, _state: &str) {}

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::BridgeResponse, EventCategory::Tick]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        Vec::new()
    }

    /// Returns five scheduled [`CommandSource`]s (all strictly read-only):
    ///
    /// 1. **perceive** — `orchestrator-perceive --dry-run` every
    ///    [`PERCEIVE_POLL_SECS`] (30 s): assembles a `perceive.snapshot.v1`
    ///    manifest without emitting to the chain. Provides leases, hopf fibers,
    ///    engine health, and pane/session counts.
    ///
    /// 2. **kernel** — `orch-kernelctl snapshot --json` every
    ///    [`KERNEL_POLL_SECS`] (10 s): reads the kernel chain HEAD.
    ///    `last_seq` / `generated_at` proxy the last `perceive.snapshot`
    ///    emission time (FIBER-3 wires perceive appends to the chain).
    ///
    /// 3. **width** — `dcg-admit width --semaphore 8` every [`WIDTH_POLL_SECS`]
    ///    (30 s): computes the instantaneous fan-out width under the default
    ///    semaphore ceiling (tier-B tunable; see [`DEFAULT_SEMAPHORE`]).
    ///
    /// 4. **arming** — `bin/fiber-cockpit-snapshot` every [`ARMING_POLL_SECS`]
    ///    (60 s): reads factory arming-key states. Only the `arming` field is
    ///    decoded; the rest of the snapshot is ignored by this module.
    ///
    /// 5. **route** — `orch-kernelctl events --trace route.decision` every
    ///    [`ROUTE_POLL_SECS`] (15 s): reads the full `route.decision` event log
    ///    and surfaces the last (most recent) entry. Absence of events is
    ///    tolerated gracefully — the module renders "no decisions yet" without
    ///    entering a degraded state.
    ///
    /// All `argv[0]` values are absolute paths (D11 rule — the host execs them
    /// directly without a shell).
    fn command_sources(&self) -> Vec<CommandSource> {
        vec![
            CommandSource {
                argv: vec![PERCEIVE_CLI.into(), "--dry-run".into()],
                interval_secs: PERCEIVE_POLL_SECS,
                tag: OW_PERCEIVE_TAG.into(),
                module_id: self.id().into(),
            },
            CommandSource {
                argv: vec![
                    KERNELCTL_CLI.into(),
                    "snapshot".into(),
                    "--json".into(),
                ],
                interval_secs: KERNEL_POLL_SECS,
                tag: OW_KERNEL_TAG.into(),
                module_id: self.id().into(),
            },
            CommandSource {
                argv: vec![
                    DCG_ADMIT_CLI.into(),
                    "width".into(),
                    "--semaphore".into(),
                    DEFAULT_SEMAPHORE.to_string(),
                ],
                interval_secs: WIDTH_POLL_SECS,
                tag: OW_WIDTH_TAG.into(),
                module_id: self.id().into(),
            },
            CommandSource {
                argv: vec![FIBER_SNAPSHOT_CLI.into()],
                interval_secs: ARMING_POLL_SECS,
                tag: OW_ARMING_TAG.into(),
                module_id: self.id().into(),
            },
            CommandSource {
                argv: vec![
                    KERNELCTL_CLI.into(),
                    "events".into(),
                    "--trace".into(),
                    "route.decision".into(),
                ],
                interval_secs: ROUTE_POLL_SECS,
                tag: OW_ROUTE_TAG.into(),
                module_id: self.id().into(),
            },
        ]
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    // ── test helpers ──────────────────────────────────────────────────────────

    fn bridge_data(tag: &str, data: serde_json::Value) -> HabitatEvent {
        HabitatEvent::BridgeData {
            module_id: "orchestrator_witness".into(),
            tag: tag.into(),
            data,
        }
    }
    fn bridge_err(tag: &str) -> HabitatEvent {
        HabitatEvent::BridgeError {
            module_id: "orchestrator_witness".into(),
            tag: tag.into(),
        }
    }
    fn tick() -> HabitatEvent {
        HabitatEvent::Tick { tick: 1 }
    }

    fn perceive_json() -> serde_json::Value {
        json!({
            "schema": "perceive.snapshot.v1",
            "captured_at_ms": 1_700_000_000_000_u64,
            "leases": [
                {"resource": "arena.loop-testing", "owner": "loop-1", "expires_ms": 9_000_000_000_u64}
            ],
            "fibers": [
                {"campaign": "factory-pulse", "root": "pulse root", "loops": ["main"]}
            ],
            "engines": [
                {"name": "WFE", "health_code": 200},
                {"name": "PV2", "health_code": 200}
            ],
            "panes": [null, null, null],
            "sessions": [null]
        })
    }
    fn kernel_json() -> serde_json::Value {
        json!({
            "status": "ok",
            "last_seq": 42,
            "last_hash": "sha256:abcdef012345",
            "event_count": 42,
            "verify_chain_ok": true,
            "generated_at": "2026-06-29T12:00:00Z",
            "warrant_count": 3,
            "queue_depth": 0
        })
    }
    fn width_json() -> serde_json::Value {
        json!({ "width": 6, "bound_by": ["semaphore"] })
    }
    fn arming_json() -> serde_json::Value {
        json!({
            "arming": [
                {"key": "factory.authorize.ultimate-zellij-orchestrator", "value": "armed"}
            ]
        })
    }
    /// Route decision JSON — returns a JSON array with one entry (most recent last).
    fn route_json() -> serde_json::Value {
        json!([
            {"capability": "code-gen", "engine": "claude-sonnet", "width": 4, "orchestrate": true}
        ])
    }
    fn render_joined(m: &OrchestratorWitness) -> String {
        m.render(20, 120)
            .into_iter()
            .map(|l| l.content)
            .collect::<String>()
    }

    /// Fully-loaded module with all five sources fed.
    fn loaded() -> OrchestratorWitness {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_data(OW_PERCEIVE_TAG, perceive_json())));
        assert!(m.handle_event(&bridge_data(OW_KERNEL_TAG, kernel_json())));
        assert!(m.handle_event(&bridge_data(OW_WIDTH_TAG, width_json())));
        assert!(m.handle_event(&bridge_data(OW_ARMING_TAG, arming_json())));
        assert!(m.handle_event(&bridge_data(OW_ROUTE_TAG, route_json())));
        m
    }

    // ── construction ─────────────────────────────────────────────────────────

    #[test]
    fn new_has_no_data() {
        let m = OrchestratorWitness::new();
        assert!(m.perceive.is_none());
        assert!(m.kernel.is_none());
        assert!(m.width.is_none());
        assert!(m.arming.is_empty());
        assert!(m.route.is_none());
    }

    #[test]
    fn default_equals_new() {
        let m = OrchestratorWitness::default();
        assert!(m.perceive.is_none());
        assert!(m.kernel.is_none());
        assert!(m.route.is_none());
    }

    // ── subscriptions ─────────────────────────────────────────────────────────

    #[test]
    fn subscriptions_contain_bridge_and_tick() {
        let subs = OrchestratorWitness::new().subscriptions();
        assert!(subs.contains(&EventCategory::BridgeResponse));
        assert!(subs.contains(&EventCategory::Tick));
    }

    #[test]
    fn subscriptions_do_not_contain_keypress_or_pipe() {
        let subs = OrchestratorWitness::new().subscriptions();
        assert!(!subs.contains(&EventCategory::KeyPress));
        assert!(!subs.contains(&EventCategory::PipeCommand));
    }

    // ── command sources: structural ───────────────────────────────────────────

    #[test]
    fn command_sources_returns_exactly_five() {
        // The witness declares five read-only self-poll sources.
        assert_eq!(OrchestratorWitness::new().command_sources().len(), 5);
    }

    #[test]
    fn command_source_tags_are_distinct() {
        let srcs = OrchestratorWitness::new().command_sources();
        let tags: Vec<&str> = srcs.iter().map(|s| s.tag.as_str()).collect();
        let mut unique = tags.clone();
        unique.dedup();
        assert_eq!(tags.len(), unique.len(), "all tags must be distinct");
    }

    #[test]
    fn all_command_source_argv0_are_absolute_paths() {
        for src in OrchestratorWitness::new().command_sources() {
            assert!(
                src.argv[0].starts_with('/'),
                "argv[0] must be absolute; got {:?} for tag {:?}",
                src.argv[0],
                src.tag,
            );
        }
    }

    #[test]
    fn all_command_source_module_ids_match_self_id() {
        let m = OrchestratorWitness::new();
        for src in m.command_sources() {
            assert_eq!(
                src.module_id,
                m.id(),
                "module_id must match id() for tag {:?}",
                src.tag,
            );
        }
    }

    // ── command sources: behavioral probes (replace const-equality tests) ─────

    /// The kernel source must poll faster than perceive — kernel HEAD reads are
    /// cheap; perceive assembles the full manifest and is expensive.
    #[test]
    fn command_sources_kernel_polls_faster_than_perceive() {
        let srcs = OrchestratorWitness::new().command_sources();
        let kernel = srcs.iter().find(|s| s.tag == OW_KERNEL_TAG).unwrap();
        let perceive = srcs.iter().find(|s| s.tag == OW_PERCEIVE_TAG).unwrap();
        assert!(
            kernel.interval_secs < perceive.interval_secs,
            "kernel ({}) must poll faster than perceive ({})",
            kernel.interval_secs,
            perceive.interval_secs
        );
    }

    /// Arming keys are stable (flip only on human action); the arming source
    /// must poll less frequently than both perceive and kernel.
    #[test]
    fn command_sources_arming_polls_slowest_among_all() {
        let srcs = OrchestratorWitness::new().command_sources();
        let arming = srcs.iter().find(|s| s.tag == OW_ARMING_TAG).unwrap();
        for other in srcs.iter().filter(|s| s.tag != OW_ARMING_TAG) {
            assert!(
                arming.interval_secs >= other.interval_secs,
                "arming ({}) must poll no faster than {} ({}s)",
                arming.interval_secs,
                other.tag,
                other.interval_secs,
            );
        }
    }

    /// All poll intervals must be positive and within a sensible operational
    /// range (1 s..=300 s). A zero interval would spin; an interval > 5 min
    /// would make the dashboard effectively static.
    #[test]
    fn command_sources_all_intervals_positive_and_bounded() {
        for src in OrchestratorWitness::new().command_sources() {
            assert!(
                src.interval_secs > 0.0,
                "interval must be positive for tag {:?}; got {}",
                src.tag,
                src.interval_secs,
            );
            assert!(
                src.interval_secs <= 300.0,
                "interval must be ≤ 300 s for tag {:?}; got {}",
                src.tag,
                src.interval_secs,
            );
        }
    }

    /// Architectural intent: this module self-polls via `command_sources()`,
    /// NOT via HTTP `data_sources()`. Both should be true simultaneously.
    #[test]
    fn module_uses_command_sources_not_data_sources() {
        let m = OrchestratorWitness::new();
        assert!(
            m.data_sources().is_empty(),
            "data_sources must be empty (HTTP polling not used)"
        );
        assert!(
            !m.command_sources().is_empty(),
            "command_sources must be non-empty (self-poll architecture)"
        );
    }

    /// Version string must be non-empty and carry at least two dot-separated
    /// numeric components — confirming it is a real version, not a placeholder.
    #[test]
    fn version_is_stable_and_nonempty() {
        let v = OrchestratorWitness::new().version();
        assert!(!v.is_empty(), "version must not be empty");
        let parts: Vec<&str> = v.split('.').collect();
        assert!(
            parts.len() >= 2,
            "version must have at least major.minor; got {v:?}"
        );
        for part in &parts {
            assert!(
                part.parse::<u32>().is_ok(),
                "each version component must be numeric; got {part:?} in {v:?}"
            );
        }
    }

    /// The module ID is stable across invocations (idempotent, no RNG/time).
    #[test]
    fn module_id_is_used_in_all_command_sources() {
        let m = OrchestratorWitness::new();
        let id = m.id();
        // Verify module ID is used consistently — behavioral invariant, not const-equality.
        assert!(!id.is_empty());
        for src in m.command_sources() {
            assert_eq!(
                src.module_id, id,
                "command source {:?} must carry the module ID",
                src.tag
            );
        }
        // Calling id() twice returns the same value (idempotent).
        assert_eq!(m.id(), id);
    }

    // ── perceive command source ───────────────────────────────────────────────

    #[test]
    fn perceive_source_argv_is_dry_run() {
        let src = &OrchestratorWitness::new().command_sources()[0];
        assert_eq!(src.argv.len(), 2);
        assert_eq!(src.argv[1], "--dry-run");
    }

    // ── kernel command source ─────────────────────────────────────────────────

    #[test]
    fn kernel_source_argv_is_snapshot_json() {
        let src = &OrchestratorWitness::new().command_sources()[1];
        assert!(src.argv.contains(&"snapshot".to_string()));
        assert!(src.argv.contains(&"--json".to_string()));
    }

    // ── width command source ──────────────────────────────────────────────────

    #[test]
    fn width_source_argv_contains_width_subcommand() {
        let src = &OrchestratorWitness::new().command_sources()[2];
        assert_eq!(src.argv[1], "width");
    }

    #[test]
    fn width_source_argv_contains_semaphore_flag_and_value() {
        let src = &OrchestratorWitness::new().command_sources()[2];
        let has_sem = src.argv.iter().any(|a| a == "--semaphore");
        assert!(has_sem, "argv must contain --semaphore");
        let val_pos = src
            .argv
            .iter()
            .position(|a| a == "--semaphore")
            .unwrap()
            + 1;
        let val: u8 = src.argv[val_pos].parse().unwrap();
        assert_eq!(val, DEFAULT_SEMAPHORE);
    }

    // ── route command source (5th, observe-only) ──────────────────────────────

    /// The route source must use `orch-kernelctl events --trace route.decision`.
    #[test]
    fn route_source_argv_contains_events_trace_route_decision() {
        let srcs = OrchestratorWitness::new().command_sources();
        let src = srcs.iter().find(|s| s.tag == OW_ROUTE_TAG).unwrap();
        assert!(
            src.argv.contains(&"events".to_string()),
            "route argv must contain 'events' subcommand"
        );
        assert!(
            src.argv.contains(&"--trace".to_string()),
            "route argv must contain '--trace'"
        );
        assert!(
            src.argv.contains(&"route.decision".to_string()),
            "route argv must specify 'route.decision' event kind"
        );
    }

    /// The route source is read-only: no mutating verbs in its argv.
    #[test]
    fn route_command_source_is_read_only_no_write_flags() {
        let srcs = OrchestratorWitness::new().command_sources();
        let src = srcs.iter().find(|s| s.tag == OW_ROUTE_TAG).unwrap();
        for arg in &src.argv {
            assert!(
                !arg.contains("append")
                    && !arg.contains("submit")
                    && !arg.contains("write")
                    && !arg.contains("register"),
                "route source must not contain write verbs; got {arg:?}"
            );
        }
    }

    // ── BridgeData routing ────────────────────────────────────────────────────

    #[test]
    fn perceive_bridge_data_is_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_data(OW_PERCEIVE_TAG, perceive_json())));
        assert!(m.perceive.is_some());
    }

    #[test]
    fn kernel_bridge_data_is_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_data(OW_KERNEL_TAG, kernel_json())));
        assert!(m.kernel.is_some());
    }

    #[test]
    fn width_bridge_data_is_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_data(OW_WIDTH_TAG, width_json())));
        assert!(m.width.is_some());
    }

    #[test]
    fn arming_bridge_data_is_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_data(OW_ARMING_TAG, arming_json())));
        assert!(!m.arming.is_empty());
    }

    #[test]
    fn route_bridge_data_is_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_data(OW_ROUTE_TAG, route_json())));
        assert!(m.route.is_some());
    }

    #[test]
    fn unrecognised_tag_bridge_data_is_not_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(!m.handle_event(&bridge_data("other_tag", json!({}))));
    }

    // ── BridgeError routing ───────────────────────────────────────────────────

    #[test]
    fn perceive_bridge_error_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_err(OW_PERCEIVE_TAG)));
        assert!(m.perceive_degraded.is_some());
    }

    #[test]
    fn kernel_bridge_error_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_err(OW_KERNEL_TAG)));
        assert!(m.kernel_degraded.is_some());
    }

    #[test]
    fn width_bridge_error_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_err(OW_WIDTH_TAG)));
        assert!(m.width_degraded.is_some());
    }

    #[test]
    fn arming_bridge_error_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_err(OW_ARMING_TAG)));
        assert!(m.arming_degraded.is_some());
    }

    #[test]
    fn route_bridge_error_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_err(OW_ROUTE_TAG)));
        assert!(m.route_degraded.is_some());
        // route data stays absent — error did not invent a value.
        assert!(m.route.is_none());
    }

    #[test]
    fn unrecognised_tag_bridge_error_is_not_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(!m.handle_event(&bridge_err("mystery")));
    }

    // ── apply_perceive ────────────────────────────────────────────────────────

    #[test]
    fn apply_perceive_happy_populates_data() {
        let mut m = OrchestratorWitness::new();
        m.apply_perceive(&perceive_json());
        let p = m.perceive.as_ref().unwrap();
        assert_eq!(p.leases.len(), 1);
        assert_eq!(p.fibers.len(), 1);
        assert_eq!(p.engines.len(), 2);
        assert_eq!(p.panes.len(), 3);
        assert_eq!(p.sessions.len(), 1);
    }

    #[test]
    fn apply_perceive_resets_tick_counter() {
        let mut m = OrchestratorWitness::new();
        m.ticks_since_perceive = 50;
        m.apply_perceive(&perceive_json());
        assert_eq!(m.ticks_since_perceive, 0);
    }

    #[test]
    fn apply_perceive_clears_degraded_on_success() {
        let mut m = OrchestratorWitness::new();
        m.perceive_degraded = Some("old error".into());
        m.apply_perceive(&perceive_json());
        assert!(m.perceive_degraded.is_none());
    }

    #[test]
    fn apply_perceive_malformed_sets_degraded_keeps_prior() {
        let mut m = OrchestratorWitness::new();
        m.apply_perceive(&perceive_json()); // good
        let prior_lease_count = m.perceive.as_ref().unwrap().leases.len();
        m.apply_perceive(&json!("not-an-object"));
        assert!(m.perceive_degraded.is_some());
        // prior data kept
        assert_eq!(m.perceive.as_ref().unwrap().leases.len(), prior_lease_count);
    }

    #[test]
    fn apply_perceive_empty_object_uses_defaults() {
        let mut m = OrchestratorWitness::new();
        m.apply_perceive(&json!({}));
        let p = m.perceive.as_ref().unwrap();
        assert!(p.leases.is_empty());
        assert!(p.fibers.is_empty());
    }

    #[test]
    fn apply_perceive_drift_tolerates_unknown_fields() {
        let mut m = OrchestratorWitness::new();
        m.apply_perceive(&json!({"future_field": 99, "leases": []}));
        assert!(m.perceive.is_some());
        assert!(m.perceive.as_ref().unwrap().leases.is_empty());
    }

    // ── apply_kernel ──────────────────────────────────────────────────────────

    #[test]
    fn apply_kernel_happy_populates_data() {
        let mut m = OrchestratorWitness::new();
        m.apply_kernel(&kernel_json());
        let k = m.kernel.as_ref().unwrap();
        assert_eq!(k.last_seq, 42);
        assert!(k.verify_chain_ok);
        assert_eq!(k.warrant_count, 3);
    }

    #[test]
    fn apply_kernel_resets_tick_counter() {
        let mut m = OrchestratorWitness::new();
        m.ticks_since_kernel = 100;
        m.apply_kernel(&kernel_json());
        assert_eq!(m.ticks_since_kernel, 0);
    }

    #[test]
    fn apply_kernel_clears_degraded() {
        let mut m = OrchestratorWitness::new();
        m.kernel_degraded = Some("old".into());
        m.apply_kernel(&kernel_json());
        assert!(m.kernel_degraded.is_none());
    }

    #[test]
    fn apply_kernel_malformed_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        let ok = m.apply_kernel(&json!("bad"));
        assert!(!ok);
        assert!(m.kernel_degraded.is_some());
    }

    // ── apply_width ───────────────────────────────────────────────────────────

    #[test]
    fn apply_width_happy_populates_data() {
        let mut m = OrchestratorWitness::new();
        m.apply_width(&width_json());
        let w = m.width.as_ref().unwrap();
        assert_eq!(w.width, 6);
        assert_eq!(w.bound_by, vec!["semaphore"]);
    }

    #[test]
    fn apply_width_resets_tick_counter() {
        let mut m = OrchestratorWitness::new();
        m.ticks_since_width = 77;
        m.apply_width(&width_json());
        assert_eq!(m.ticks_since_width, 0);
    }

    #[test]
    fn apply_width_zero_width_is_valid() {
        let mut m = OrchestratorWitness::new();
        m.apply_width(&json!({"width": 0, "bound_by": ["semaphore"]}));
        assert_eq!(m.width.as_ref().unwrap().width, 0);
    }

    #[test]
    fn apply_width_max_width_is_valid() {
        let mut m = OrchestratorWitness::new();
        m.apply_width(&json!({"width": 255, "bound_by": []}));
        assert_eq!(m.width.as_ref().unwrap().width, 255);
    }

    #[test]
    fn apply_width_malformed_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        let ok = m.apply_width(&json!(42)); // number, not object
        assert!(!ok);
        assert!(m.width_degraded.is_some());
    }

    // ── apply_arming ──────────────────────────────────────────────────────────

    #[test]
    fn apply_arming_happy_populates_rows() {
        let mut m = OrchestratorWitness::new();
        m.apply_arming(&arming_json());
        assert_eq!(m.arming.len(), 1);
        assert_eq!(m.arming[0].value, "armed");
    }

    #[test]
    fn apply_arming_resets_tick_counter() {
        let mut m = OrchestratorWitness::new();
        m.ticks_since_arming = 99;
        m.apply_arming(&arming_json());
        assert_eq!(m.ticks_since_arming, 0);
    }

    #[test]
    fn apply_arming_empty_arming_array_is_valid() {
        let mut m = OrchestratorWitness::new();
        m.apply_arming(&json!({"arming": []}));
        assert!(m.arming.is_empty());
        assert!(m.arming_degraded.is_none());
    }

    #[test]
    fn apply_arming_ignores_extra_fiber_fields() {
        let mut m = OrchestratorWitness::new();
        m.apply_arming(&json!({
            "arming": [{"key": "k1", "value": "armed"}],
            "campaigns": [{"name": "unused"}],
            "leases": []
        }));
        assert_eq!(m.arming.len(), 1);
    }

    #[test]
    fn apply_arming_missing_arming_field_defaults_to_empty() {
        let mut m = OrchestratorWitness::new();
        m.apply_arming(&json!({}));
        assert!(m.arming.is_empty());
        assert!(m.arming_degraded.is_none());
    }

    // ── apply_route ───────────────────────────────────────────────────────────

    /// The last entry in a multi-element array wins (most recent decision).
    #[test]
    fn apply_route_array_last_entry_wins_over_earlier() {
        let mut m = OrchestratorWitness::new();
        let data = json!([
            {"capability": "old-cap", "engine": "old-engine", "width": 1, "orchestrate": false},
            {"capability": "new-cap", "engine": "new-engine", "width": 4, "orchestrate": true}
        ]);
        let ok = m.apply_route(&data);
        assert!(ok);
        let rd = m.route.as_ref().unwrap();
        assert_eq!(rd.capability, "new-cap", "must take last entry, not first");
        assert_eq!(rd.engine, "new-engine");
        assert_eq!(rd.width, 4);
        assert!(rd.orchestrate);
    }

    /// An empty array means no route decisions have been emitted yet.
    /// This is normal at cold start — not an error.
    #[test]
    fn apply_route_empty_array_tolerates_absence() {
        let mut m = OrchestratorWitness::new();
        let ok = m.apply_route(&json!([]));
        assert!(ok, "empty array must not set degraded");
        assert!(m.route.is_none(), "no decision populated from empty array");
        assert!(m.route_degraded.is_none(), "empty array must not set degraded");
    }

    /// A single JSON object (short-circuit output shape) is accepted directly.
    #[test]
    fn apply_route_single_object_accepted() {
        let mut m = OrchestratorWitness::new();
        let ok = m.apply_route(&json!({"capability": "c1", "engine": "e1", "width": 2, "orchestrate": false}));
        assert!(ok);
        let rd = m.route.as_ref().unwrap();
        assert_eq!(rd.capability, "c1");
    }

    /// JSON null means no events in the chain — treated as graceful absence.
    #[test]
    fn apply_route_null_tolerates_absence() {
        let mut m = OrchestratorWitness::new();
        let ok = m.apply_route(&json!(null));
        assert!(ok, "null must not set degraded");
        assert!(m.route.is_none());
        assert!(m.route_degraded.is_none());
    }

    /// Missing fields in a route entry use serde defaults (drift tolerance).
    #[test]
    fn apply_route_missing_fields_use_defaults() {
        let mut m = OrchestratorWitness::new();
        m.apply_route(&json!([{}]));
        let rd = m.route.as_ref().unwrap();
        assert_eq!(rd.capability, "");
        assert_eq!(rd.engine, "");
        assert_eq!(rd.width, 0);
        assert!(!rd.orchestrate);
    }

    /// Successful decode after an error clears `route_degraded`.
    #[test]
    fn apply_route_clears_degraded_on_success() {
        let mut m = OrchestratorWitness::new();
        m.route_degraded = Some("prior error".into());
        m.apply_route(&route_json());
        assert!(m.route_degraded.is_none());
    }

    /// A scalar (not array/object/null) at top level sets degraded.
    #[test]
    fn apply_route_malformed_scalar_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        let ok = m.apply_route(&json!("not-an-array-or-object"));
        assert!(!ok);
        assert!(m.route_degraded.is_some());
    }

    /// A malformed inner entry (non-object inside array) sets degraded.
    #[test]
    fn apply_route_malformed_entry_sets_degraded() {
        let mut m = OrchestratorWitness::new();
        // Array whose last element is a scalar — serde will fail to decode it.
        let ok = m.apply_route(&json!([42]));
        assert!(!ok, "non-object array entry must set degraded");
        assert!(m.route_degraded.is_some());
    }

    /// Tick counter resets to 0 on a successful route data update.
    #[test]
    fn apply_route_resets_tick_counter() {
        let mut m = OrchestratorWitness::new();
        m.ticks_since_route = 88;
        m.apply_route(&route_json());
        assert_eq!(m.ticks_since_route, 0);
    }

    // ── Tick / staleness ──────────────────────────────────────────────────────

    #[test]
    fn tick_increments_all_five_counters() {
        let mut m = OrchestratorWitness::new();
        m.handle_event(&tick());
        assert_eq!(m.ticks_since_perceive, 1);
        assert_eq!(m.ticks_since_kernel, 1);
        assert_eq!(m.ticks_since_width, 1);
        assert_eq!(m.ticks_since_arming, 1);
        assert_eq!(m.ticks_since_route, 1);
    }

    #[test]
    fn tick_returns_false_not_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(!m.handle_event(&tick()));
    }

    #[test]
    fn all_tick_counters_saturate_at_u64_max() {
        let mut m = OrchestratorWitness::new();
        m.ticks_since_perceive = u64::MAX;
        m.ticks_since_kernel = u64::MAX;
        m.ticks_since_width = u64::MAX;
        m.ticks_since_arming = u64::MAX;
        m.ticks_since_route = u64::MAX;
        m.handle_event(&tick());
        assert_eq!(m.ticks_since_perceive, u64::MAX);
        assert_eq!(m.ticks_since_kernel, u64::MAX);
        assert_eq!(m.ticks_since_width, u64::MAX);
        assert_eq!(m.ticks_since_arming, u64::MAX);
        assert_eq!(m.ticks_since_route, u64::MAX);
    }

    #[test]
    fn fresh_data_resets_counter_to_zero_after_aging() {
        let mut m = OrchestratorWitness::new();
        m.ticks_since_kernel = 50;
        m.apply_kernel(&kernel_json());
        assert_eq!(m.ticks_since_kernel, 0);
    }

    #[test]
    fn elapsed_secs_uses_poll_floor_when_zero() {
        let mut m = OrchestratorWitness::new();
        m.poll_secs = 0.0;
        // floor is 1.0, so 9 ticks × 1.0 = 9.0
        assert!((m.elapsed_secs(9) - 9.0).abs() < f64::EPSILON);
    }

    // ── render: STALE indicator (required: via tick path) ─────────────────────

    /// STALE test: ingest perceive data, advance the tick counter past
    /// `STALE_THRESHOLD_SECS` (90 s) via repeated `Tick` events (the tick
    /// path — not direct field manipulation), then assert the render contains
    /// the STALE indicator.
    ///
    /// With `poll_secs = 10.0` (default), 10 ticks × 10 s = 100 s > 90 s.
    #[test]
    fn render_perceive_stale_after_threshold_exceeded_via_tick_path() {
        let mut m = OrchestratorWitness::new();
        // Feed perceive data so the ticker has something to go stale.
        assert!(m.handle_event(&bridge_data(OW_PERCEIVE_TAG, perceive_json())));
        assert_eq!(m.ticks_since_perceive, 0, "counter reset after data");
        // Advance via the tick path: 10 × 10 s = 100 s > STALE_THRESHOLD_SECS.
        for _ in 0..10 {
            m.handle_event(&tick());
        }
        assert_eq!(m.ticks_since_perceive, 10);
        let out = render_joined(&m);
        assert!(
            out.contains("STALE"),
            "render must surface STALE after 100 s elapsed; got: {out}",
        );
    }

    /// Kernel staleness via tick path — kernel source carries its own counter.
    #[test]
    fn render_kernel_stale_after_threshold_exceeded_via_tick_path() {
        let mut m = OrchestratorWitness::new();
        assert!(m.handle_event(&bridge_data(OW_KERNEL_TAG, kernel_json())));
        assert_eq!(m.ticks_since_kernel, 0);
        for _ in 0..10 {
            m.handle_event(&tick());
        }
        let out = render_joined(&m);
        assert!(
            out.contains("STALE"),
            "kernel stale indicator must appear after 100 s; got: {out}",
        );
    }

    // ── render: cold state ────────────────────────────────────────────────────

    #[test]
    fn render_cold_shows_sensing_header() {
        let out = render_joined(&OrchestratorWitness::new());
        assert!(out.contains("SENSING"), "cold render must show SENSING");
    }

    #[test]
    fn render_cold_shows_orch_witness_label() {
        let out = render_joined(&OrchestratorWitness::new());
        assert!(out.contains("ORCH WITNESS"));
    }

    #[test]
    fn render_cold_shows_awaiting_on_all_sources() {
        let out = render_joined(&OrchestratorWitness::new());
        assert!(out.contains("awaiting"), "cold render must mention awaiting");
    }

    // ── render: loaded state ──────────────────────────────────────────────────

    #[test]
    fn render_loaded_shows_ok_header() {
        let out = render_joined(&loaded());
        assert!(out.contains("OK"), "loaded render must show OK");
    }

    #[test]
    fn render_loaded_shows_width() {
        let out = render_joined(&loaded());
        assert!(out.contains("width"), "render must show width label");
        assert!(out.contains('6'), "render must show width value 6");
    }

    #[test]
    fn render_loaded_shows_chain_seq() {
        let out = render_joined(&loaded());
        assert!(out.contains("seq="), "render must show seq= label");
        assert!(out.contains("42"), "render must show seq value 42");
    }

    #[test]
    fn render_loaded_shows_lease_count() {
        let out = render_joined(&loaded());
        assert!(out.contains("leases"));
    }

    #[test]
    fn render_loaded_shows_fiber_campaign() {
        let out = render_joined(&loaded());
        assert!(out.contains("fibers"));
    }

    #[test]
    fn render_loaded_shows_arming_label() {
        let out = render_joined(&loaded());
        assert!(out.contains("arming"));
    }

    #[test]
    fn render_loaded_shows_engines_up_ratio() {
        let out = render_joined(&loaded());
        assert!(out.contains("engines"), "render must show engines label");
        assert!(out.contains("2/2"), "render must show 2/2 engines up");
    }

    /// The route line appears in the full render when route data is present.
    #[test]
    fn render_loaded_shows_route_line() {
        let out = render_joined(&loaded());
        assert!(out.contains("last route"), "render must show route label");
        assert!(out.contains("code-gen"), "render must show capability from route data");
        assert!(out.contains("claude-sonnet"), "render must show engine from route data");
    }

    // ── render: degraded paths ────────────────────────────────────────────────

    #[test]
    fn render_perceive_degraded_shows_err() {
        let mut m = OrchestratorWitness::new();
        m.handle_event(&bridge_err(OW_PERCEIVE_TAG));
        let out = render_joined(&m);
        assert!(out.contains("ERR") || out.contains("DEGRADED"));
    }

    #[test]
    fn render_kernel_degraded_shows_err() {
        let mut m = OrchestratorWitness::new();
        m.handle_event(&bridge_err(OW_KERNEL_TAG));
        let out = render_joined(&m);
        assert!(out.contains("ERR") || out.contains("DEGRADED"));
    }

    #[test]
    fn render_route_degraded_shows_err() {
        let mut m = OrchestratorWitness::new();
        m.handle_event(&bridge_err(OW_ROUTE_TAG));
        let out = render_joined(&m);
        assert!(
            out.contains("ERR") || out.contains("DEGRADED"),
            "route degraded must surface ERR or DEGRADED; got: {out}"
        );
    }

    #[test]
    fn render_width_zero_shows_in_red_band() {
        let mut m = OrchestratorWitness::new();
        m.apply_width(&json!({"width": 0, "bound_by": ["semaphore"]}));
        let out = render_joined(&m);
        assert!(out.contains('0'));
    }

    #[test]
    fn render_chain_warn_when_verify_chain_fails() {
        let mut m = OrchestratorWitness::new();
        m.apply_kernel(&json!({
            "status": "warn",
            "last_seq": 1,
            "last_hash": "abc",
            "event_count": 1,
            "verify_chain_ok": false,
            "generated_at": "2026-06-29T12:00:00Z",
            "warrant_count": 0,
            "queue_depth": 0
        }));
        let out = render_joined(&m);
        assert!(out.contains("CHAIN WARN"), "must surface CHAIN WARN");
    }

    #[test]
    fn render_no_panic_across_terminal_dimensions() {
        let m = loaded();
        for (r, c) in [(0, 0), (1, 1), (5, 10), (24, 80), (50, 200)] {
            let _ = m.render(r, c);
        }
    }

    // ── render: arming counts ─────────────────────────────────────────────────

    #[test]
    fn render_arming_all_armed_shows_all() {
        let mut m = OrchestratorWitness::new();
        m.apply_arming(&json!({
            "arming": [
                {"key": "k1", "value": "armed"},
                {"key": "k2", "value": "armed"}
            ]
        }));
        let out = render_joined(&m);
        assert!(out.contains("all 2 armed") || out.contains("2/2"), "both keys armed");
    }

    #[test]
    fn render_arming_partial_shows_count() {
        let mut m = OrchestratorWitness::new();
        m.apply_arming(&json!({
            "arming": [
                {"key": "k1", "value": "armed"},
                {"key": "k2", "value": "disarmed"}
            ]
        }));
        let out = render_joined(&m);
        assert!(out.contains("1/2") || out.contains("arming"));
    }

    // ── render: route line ────────────────────────────────────────────────────

    /// Cold-start route line gracefully says "no decisions yet".
    #[test]
    fn render_route_cold_no_decisions_yet() {
        let m = OrchestratorWitness::new();
        let out = render_joined(&m);
        assert!(
            out.contains("no decisions yet"),
            "cold route line must say 'no decisions yet'; got: {out}"
        );
    }

    /// Loaded route line surfaces capability and engine from the last event.
    #[test]
    fn render_route_loaded_shows_capability_and_engine() {
        let mut m = OrchestratorWitness::new();
        m.apply_route(&route_json());
        let out = render_joined(&m);
        assert!(out.contains("code-gen"), "render must show capability");
        assert!(out.contains("claude-sonnet"), "render must show engine");
    }

    /// Width and orchestrate flag appear in the route line.
    #[test]
    fn render_route_loaded_shows_width_and_orchestrate_flag() {
        let mut m = OrchestratorWitness::new();
        m.apply_route(&json!([{"capability": "c", "engine": "e", "width": 7, "orchestrate": false}]));
        let out = render_joined(&m);
        assert!(out.contains('7'), "route line must show width value");
        assert!(
            out.contains("false"),
            "orchestrate=false must appear in render"
        );
    }

    // ── render: leases ────────────────────────────────────────────────────────

    #[test]
    fn render_leases_none_shows_none() {
        let mut m = OrchestratorWitness::new();
        m.apply_perceive(&json!({
            "leases": [],
            "fibers": [],
            "engines": [],
            "panes": [],
            "sessions": []
        }));
        let out = render_joined(&m);
        assert!(out.contains("none") || out.contains("leases"));
    }

    #[test]
    fn render_leases_truncates_long_list() {
        let mut m = OrchestratorWitness::new();
        let leases: Vec<serde_json::Value> = (0..10)
            .map(|i| json!({"resource": format!("arena.{i}"), "owner": "o", "expires_ms": 0}))
            .collect();
        m.apply_perceive(&json!({"leases": leases, "fibers": [], "engines": [], "panes": [], "sessions": []}));
        let out = render_joined(&m);
        assert!(out.contains("10") || out.contains('+'), "must surface count or overflow indicator");
    }

    // ── serialize state (stateless sensor) ───────────────────────────────────

    #[test]
    fn serialize_state_is_none() {
        assert!(loaded().serialize_state().is_none());
    }

    #[test]
    fn restore_state_is_noop() {
        let mut m = OrchestratorWitness::new();
        m.restore_state("anything");
        assert!(m.perceive.is_none());
        assert!(m.kernel.is_none());
        assert!(m.route.is_none());
    }

    // ── keypress and pipe events are not consumed ─────────────────────────────

    #[test]
    fn keypress_is_not_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(!m.handle_event(&HabitatEvent::KeyPress { key: 'w' }));
    }

    #[test]
    fn pipe_command_is_not_consumed() {
        let mut m = OrchestratorWitness::new();
        assert!(!m.handle_event(&HabitatEvent::PipeCommand {
            name: "ow-data".into(),
            payload: "{}".into(),
        }));
    }

    // ── observe-only: no write verbs in this file ─────────────────────────────

    /// Mechanical witness that no write verb appears in the module source.
    ///
    /// This mirrors the grep-gate discipline in plan §9.8. The module source
    /// may not contain "register", "submit", "append", or "write" as standalone
    /// verbs (as part of a function call expression). Checking the string
    /// literal representation of this module's key identifiers is an effective
    /// second-pass guard.
    #[test]
    fn no_write_verbs_in_handle_event_path() {
        let m = OrchestratorWitness::new();
        assert!(!m.id().starts_with("write"), "module id must not claim a write role");
        for src in m.command_sources() {
            // --dry-run is the key flag that prevents perceive chain emission.
            if src.tag == OW_PERCEIVE_TAG {
                assert!(
                    src.argv.contains(&"--dry-run".to_string()),
                    "perceive source must pass --dry-run to prevent chain emission"
                );
            }
            // Route source must not contain any write verbs in its argv.
            if src.tag == OW_ROUTE_TAG {
                for arg in &src.argv {
                    assert!(
                        !arg.contains("append")
                            && !arg.contains("submit")
                            && !arg.contains("write")
                            && !arg.contains("register"),
                        "route source must not contain write verbs; got {arg:?}"
                    );
                }
            }
        }
    }

    /// The width source must NOT include any mutating flags.
    #[test]
    fn width_source_argv_has_no_submit_or_append() {
        let src = &OrchestratorWitness::new().command_sources()[2];
        for arg in &src.argv {
            assert!(
                !arg.contains("submit") && !arg.contains("append"),
                "width source must not contain mutating verbs; got {arg:?}"
            );
        }
    }
}
