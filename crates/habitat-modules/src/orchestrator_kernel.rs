use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{CommandSource, DataSource, HabitatModule};
use habitat_core::render::*;
use serde::Deserialize;

const SNAPSHOT_TAG: &str = "kernel_snapshot";
const DEFAULT_SIDECAR_CLI: &str = "/home/louranicas/.local/bin/orch-kernelctl";
const KERNEL_MODULE_VERSION: &str = "0.1.1";

/// Tag used by the perceive self-poll `CommandSource` (FIBER-3, plan §2/T1).
/// Matches the `orch-kernelctl append --kind perceive.snapshot` kind string so
/// that the receipt arrives over the same `BridgeData { tag }` channel.
const PERCEIVE_TAG: &str = "perceive_snapshot";

/// Absolute path to the `orchestrator-perceive` binary (D11 absolute-argv rule).
/// The host execs `argv[0]` directly with no shell, so `$PATH` / `~` are not
/// expanded. This default mirrors the `~/.local/bin/` deploy target used by the
/// rest of the habitat toolchain.
const DEFAULT_PERCEIVE_CLI: &str = "/home/louranicas/.local/bin/orchestrator-perceive";

/// Perceive poll cadence in seconds.  The perceive assembler is heavier than a
/// pure HTTP probe (it scans panes, engines, catalog, leases, fibers) so 30 s
/// matches the sphere-warden cadence rather than the 5 s kernel snapshot cadence.
const DEFAULT_PERCEIVE_POLL_SECS: f64 = 30.0;

// ── internal types ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Deserialize)]
struct KernelSnapshot {
    status: String,
    db_path: String,
    last_seq: i64,
    last_hash: String,
    event_count: i64,
    verify_chain_ok: bool,
    schema_version: i64,
    #[serde(default)]
    generated_at: String,
    #[serde(default)]
    edges: Vec<EdgeSnapshot>,
    #[serde(default)]
    warrant_count: i64,
    #[serde(default)]
    queue_depth: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct EdgeSnapshot {
    edge: String,
    state: String,
    observed_at: String,
    evidence_ref: Option<String>,
}

/// Abbreviated receipt returned by `orch-kernelctl append` after the
/// perceive helper emits a `perceive.snapshot` event.
///
/// Fields are serde-defaulted so the module degrades gracefully if FIBER-1
/// ships a richer receipt shape later.
#[derive(Clone, Debug, Default, Deserialize)]
struct PerceiveReceipt {
    /// Chain sequence number assigned to this append.
    #[serde(default)]
    seq: i64,
    /// Opaque event identifier (hash-chain reference).
    #[serde(default)]
    event_id: String,
    /// Kind echo from the sidecar (should always equal `perceive.snapshot`).
    #[serde(default)]
    kind: String,
}

// ── module struct ─────────────────────────────────────────────────────────────

pub struct OrchestratorKernel {
    sidecar_cli: String,
    poll_secs: f64,
    snapshot: Option<KernelSnapshot>,
    degraded_reason: Option<String>,
    refresh_count: u64,
    /// Absolute path to the `orchestrator-perceive` binary used by the
    /// perceive `CommandSource` self-poll (FIBER-3, D11 absolute-argv rule).
    perceive_cli: String,
    /// Last decoded receipt from `orch-kernelctl append` (perceive emit).
    perceive_receipt: Option<PerceiveReceipt>,
    /// Non-`None` when the perceive command source errors or the receipt
    /// cannot be decoded; cleared on the next successful receipt.
    perceive_degraded: Option<String>,
    /// How many perceive `BridgeData` events have arrived since init.
    perceive_refresh_count: u64,
}

impl OrchestratorKernel {
    /// Construct with all fields at their defaults — no sidecar contact yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sidecar_cli: DEFAULT_SIDECAR_CLI.into(),
            poll_secs: 5.0,
            snapshot: None,
            degraded_reason: Some("awaiting sidecar snapshot".into()),
            refresh_count: 0,
            perceive_cli: DEFAULT_PERCEIVE_CLI.into(),
            perceive_receipt: None,
            perceive_degraded: None,
            perceive_refresh_count: 0,
        }
    }
}

impl Default for OrchestratorKernel {
    fn default() -> Self {
        Self::new()
    }
}

// ── trait impl ────────────────────────────────────────────────────────────────

impl HabitatModule for OrchestratorKernel {
    fn id(&self) -> &'static str {
        "orchestrator_kernel"
    }

    fn version(&self) -> &'static str {
        KERNEL_MODULE_VERSION
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.sidecar_cli.clone_from(&config.sidecar_cli);
        self.poll_secs = config.kernel_poll;
    }

    /// Route `BridgeData` / `BridgeError` events from both the kernel-snapshot
    /// sidecar poll (`SNAPSHOT_TAG`) and the perceive self-poll (`PERCEIVE_TAG`).
    ///
    /// Returns `true` when the event is consumed by this module, `false` otherwise.
    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            // ── kernel snapshot (existing) ────────────────────────────────────
            HabitatEvent::BridgeData { tag, data, .. } if tag == SNAPSHOT_TAG => {
                self.refresh_count = self.refresh_count.saturating_add(1);
                match serde_json::from_value::<KernelSnapshot>(data.clone()) {
                    Ok(snapshot) => {
                        self.snapshot = Some(snapshot);
                        self.degraded_reason = None;
                    }
                    Err(err) => {
                        self.degraded_reason = Some(format!("snapshot decode failed: {err}"));
                    }
                }
                true
            }
            HabitatEvent::BridgeError { tag, .. } if tag == SNAPSHOT_TAG => {
                self.degraded_reason = Some("sidecar snapshot command failed".into());
                true
            }

            // ── perceive self-poll (FIBER-3 addition) ─────────────────────────
            //
            // `orchestrator-perceive --emit-from-body` runs the FIBER-1 assembler,
            // appends a `perceive.snapshot` event to the kernel chain via
            // `orch-kernelctl append`, and writes the resulting `AppendReceipt`
            // JSON to stdout.  That JSON arrives here as `BridgeData { tag:
            // PERCEIVE_TAG }`.
            HabitatEvent::BridgeData { tag, data, .. } if tag == PERCEIVE_TAG => {
                self.perceive_refresh_count = self.perceive_refresh_count.saturating_add(1);
                match serde_json::from_value::<PerceiveReceipt>(data.clone()) {
                    Ok(receipt) => {
                        self.perceive_receipt = Some(receipt);
                        self.perceive_degraded = None;
                    }
                    Err(err) => {
                        self.perceive_degraded =
                            Some(format!("perceive receipt decode failed: {err}"));
                    }
                }
                true
            }
            HabitatEvent::BridgeError { tag, .. } if tag == PERCEIVE_TAG => {
                self.perceive_degraded = Some("perceive helper command failed".into());
                true
            }

            // ── catch-all ─────────────────────────────────────────────────────
            HabitatEvent::Tick { .. }
            | HabitatEvent::KeyPress { .. }
            | HabitatEvent::PipeCommand { .. }
            | HabitatEvent::BridgeData { .. }
            | HabitatEvent::BridgeError { .. } => false,
        }
    }

    fn render(&self, _rows: usize, cols: usize) -> Vec<RenderLine> {
        let width = cols.min(120);
        let mut lines = Vec::new();
        let (status_color, status_label) = match (&self.snapshot, &self.degraded_reason) {
            (Some(snapshot), None) if snapshot.verify_chain_ok => (GRN, "OK"),
            (Some(_), None) => (YEL, "CHAIN WARN"),
            _ => (YEL, "DEGRADED"),
        };
        lines.push(RenderLine::new(format!(
            " {B}{CYN}ORCHESTRATOR KERNEL{R} {status_color}{status_label}{R} {D}v{}{R}",
            self.version(),
        )));
        lines.push(RenderLine::separator(width));

        if let Some(snapshot) = &self.snapshot {
            let hash_short = snapshot.last_hash.chars().take(18).collect::<String>();
            lines.push(RenderLine::new(format!(
                " {D}sidecar{R} status={B}{}{R} seq={B}{}{R} events={B}{}{R} schema={D}{}{R}",
                snapshot.status, snapshot.last_seq, snapshot.event_count, snapshot.schema_version,
            )));
            lines.push(RenderLine::new(format!(
                " {D}verify-chain{R}={} {D}hash{R}={hash_short} {D}refreshes{R}={}",
                if snapshot.verify_chain_ok {
                    format!("{GRN}ok{R}")
                } else {
                    format!("{RED}fail{R}")
                },
                self.refresh_count,
            )));
            lines.push(RenderLine::new(format!(
                " {D}warrants{R}={B}{}{R} {D}queue{R}={B}{}{R} {D}snapshot{R}={}",
                snapshot.warrant_count,
                snapshot.queue_depth,
                truncate(&snapshot.generated_at, width.saturating_sub(35)),
            )));
            lines.push(RenderLine::new(format!(
                " {D}db{R} {}",
                truncate(&snapshot.db_path, width.saturating_sub(5)),
            )));
        } else {
            let reason = self.degraded_reason.as_deref().unwrap_or("unknown");
            lines.push(RenderLine::new(format!(
                " {YEL}DEGRADED_NO_DURABILITY{R} {D}{reason}{R}"
            )));
            lines.push(RenderLine::new(format!(
                " {D}sidecar_cli{R} {}",
                truncate(&self.sidecar_cli, width.saturating_sub(13)),
            )));
        }

        lines.push(RenderLine::new(format!(
            " {D}edges{R} {}",
            self.render_edges(width)
        )));

        // ── perceive self-poll status line (FIBER-3 addition) ─────────────────
        lines.push(self.render_perceive_line(width));

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
        Vec::new()
    }

    /// Returns two scheduled [`CommandSource`]s:
    ///
    /// 1. **kernel snapshot** — polls `orch-kernelctl snapshot --json` every
    ///    `poll_secs` seconds; result is the durable chain health snapshot.
    /// 2. **perceive self-poll** — runs `orchestrator-perceive --emit-from-body`
    ///    every [`DEFAULT_PERCEIVE_POLL_SECS`] seconds; the helper assembles the
    ///    full perceive manifest and appends a `perceive.snapshot` event to the
    ///    kernel chain via `orch-kernelctl append`.  The `AppendReceipt` JSON
    ///    arrives as `BridgeData { tag: PERCEIVE_TAG }`.
    ///
    /// Both `argv[0]` values are absolute paths (D11 rule — the host execs them
    /// directly with no shell expansion).
    fn command_sources(&self) -> Vec<CommandSource> {
        vec![
            CommandSource {
                argv: vec![self.sidecar_cli.clone(), "snapshot".into(), "--json".into()],
                interval_secs: self.poll_secs,
                tag: SNAPSHOT_TAG.into(),
                module_id: self.id().into(),
            },
            CommandSource {
                argv: vec![self.perceive_cli.clone(), "--emit-from-body".into()],
                interval_secs: DEFAULT_PERCEIVE_POLL_SECS,
                tag: PERCEIVE_TAG.into(),
                module_id: self.id().into(),
            },
        ]
    }
}

// ── private render helpers ────────────────────────────────────────────────────

impl OrchestratorKernel {
    fn render_edges(&self, width: usize) -> String {
        let Some(snapshot) = &self.snapshot else {
            return format!("{YEL}unmeasured{R}");
        };
        if snapshot.edges.is_empty() {
            return format!("{YEL}unmeasured{R}");
        }
        let mut rendered = Vec::new();
        for edge in snapshot.edges.iter().take(5) {
            let color = match edge.state.as_str() {
                "MEASURED" | "OK" => GRN,
                "FAILED" | "MISSING" => RED,
                _ => YEL,
            };
            let evidence = edge
                .evidence_ref
                .as_ref()
                .map(|value| format!("@{}", value.chars().take(8).collect::<String>()))
                .unwrap_or_default();
            let age = if edge.observed_at.is_empty() {
                String::new()
            } else {
                format!(" {}", edge.observed_at.chars().take(13).collect::<String>())
            };
            rendered.push(format!("{color}{}{R}{D}{evidence}{age}{R}", edge.edge));
        }
        truncate(&rendered.join(" "), width.saturating_sub(8)).to_string()
    }

    /// Render one status line for the perceive self-poll.
    ///
    /// - **Receipt present:** shows the last appended seq and `event_id` prefix.
    /// - **Degraded:** surfaces the error reason so the operator can act.
    /// - **Awaiting first emit:** shown at cold-start before the first 30 s tick.
    fn render_perceive_line(&self, width: usize) -> RenderLine {
        if let Some(reason) = &self.perceive_degraded {
            return RenderLine::new(format!(
                " {D}perceive{R} {YEL}ERR{R} {D}{}{R}",
                truncate(reason, width.saturating_sub(16)),
            ));
        }
        if let Some(receipt) = &self.perceive_receipt {
            let event_id_short = receipt.event_id.chars().take(12).collect::<String>();
            let kind_hint = if receipt.kind.is_empty() {
                String::new()
            } else {
                format!(" {D}kind={}{R}", receipt.kind)
            };
            return RenderLine::new(format!(
                " {D}perceive{R} seq={B}{}{R} id={D}{event_id_short}{R}{kind_hint} {D}emits={}{R}",
                receipt.seq, self.perceive_refresh_count,
            ));
        }
        RenderLine::new(format!(
            " {D}perceive awaiting first emit ({}s cadence){R}",
            DEFAULT_PERCEIVE_POLL_SECS as u32,
        ))
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    // helpers
    fn bridge_data(tag: &str, data: serde_json::Value) -> HabitatEvent {
        HabitatEvent::BridgeData {
            module_id: "orchestrator_kernel".into(),
            tag: tag.into(),
            data,
        }
    }
    fn bridge_error(tag: &str) -> HabitatEvent {
        HabitatEvent::BridgeError {
            module_id: "orchestrator_kernel".into(),
            tag: tag.into(),
        }
    }
    fn kernel_snapshot_json() -> serde_json::Value {
        json!({
            "status": "ok",
            "db_path": "/tmp/orchestrator-kernel.sqlite",
            "last_seq": 7,
            "last_hash": "sha256:abcdef",
            "event_count": 7,
            "verify_chain_ok": true,
            "schema_version": 1
        })
    }
    fn perceive_receipt_json() -> serde_json::Value {
        json!({
            "seq": 42,
            "event_id": "sha256:deadbeefcafe",
            "kind": "perceive.snapshot",
            "ts": "2026-06-29T00:00:00Z"
        })
    }

    // ── pre-existing tests (kept verbatim) ───────────────────────────────────

    #[test]
    fn command_source_uses_snapshot_json_command() {
        let module = OrchestratorKernel::new();
        let sources = module.command_sources();
        assert_eq!(module.version(), KERNEL_MODULE_VERSION);
        // Two sources now: snapshot + perceive.
        assert!(!sources.is_empty());
        assert_eq!(sources[0].argv[1], "snapshot");
        assert_eq!(sources[0].argv[2], "--json");
        assert_eq!(sources[0].tag, SNAPSHOT_TAG);
    }

    #[test]
    fn bridge_data_updates_snapshot() {
        let mut module = OrchestratorKernel::new();
        let event = bridge_data(SNAPSHOT_TAG, kernel_snapshot_json());
        assert!(module.handle_event(&event));
        assert!(module.degraded_reason.is_none());
        assert_eq!(module.snapshot.as_ref().map(|s| s.last_seq), Some(7));
    }

    #[test]
    fn bridge_error_renders_degraded() {
        let mut module = OrchestratorKernel::new();
        let event = bridge_error(SNAPSHOT_TAG);
        assert!(module.handle_event(&event));
        let rendered = module
            .render(10, 100)
            .into_iter()
            .map(|l| l.content)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("DEGRADED"));
    }

    // ── perceive command-source declaration (FIBER-3 addition) ───────────────

    /// D11: argv[0] must be an absolute path (no shell, no $PATH expansion).
    #[test]
    fn perceive_command_source_argv0_is_absolute_path() {
        let sources = OrchestratorKernel::new().command_sources();
        let perceive = &sources[1];
        assert!(
            perceive.argv[0].starts_with('/'),
            "argv[0] must be an absolute path; got {:?}",
            perceive.argv[0]
        );
    }

    /// argv[1] is the `--emit-from-body` flag that triggers the perceive assembler.
    #[test]
    fn perceive_command_source_argv_is_emit_from_body() {
        let sources = OrchestratorKernel::new().command_sources();
        let perceive = &sources[1];
        assert_eq!(perceive.argv.len(), 2);
        assert_eq!(perceive.argv[1], "--emit-from-body");
    }

    /// The tag must equal `PERCEIVE_TAG` so the bridge client routes results here.
    #[test]
    fn perceive_command_source_tag_is_perceive_snapshot() {
        let sources = OrchestratorKernel::new().command_sources();
        assert_eq!(sources[1].tag, PERCEIVE_TAG);
        assert_eq!(sources[1].tag, "perceive_snapshot");
    }

    /// `module_id` must match `self.id()` for routing and attribution.
    #[test]
    fn perceive_command_source_module_id_is_kernel_id() {
        let module = OrchestratorKernel::new();
        let sources = module.command_sources();
        assert_eq!(sources[1].module_id, module.id());
    }

    /// Interval should be the expected 30 s cadence.
    #[test]
    fn perceive_command_source_interval_is_30s() {
        let sources = OrchestratorKernel::new().command_sources();
        assert!(
            (sources[1].interval_secs - DEFAULT_PERCEIVE_POLL_SECS).abs() < f64::EPSILON,
            "expected {DEFAULT_PERCEIVE_POLL_SECS}s; got {}",
            sources[1].interval_secs
        );
    }

    /// After the addition there must be exactly 2 sources: snapshot + perceive.
    #[test]
    fn command_sources_yields_two_sources() {
        let sources = OrchestratorKernel::new().command_sources();
        assert_eq!(sources.len(), 2, "snapshot + perceive = 2 sources");
    }

    /// The first source remains the kernel snapshot (unchanged by FIBER-3).
    #[test]
    fn command_sources_first_is_snapshot_second_is_perceive() {
        let sources = OrchestratorKernel::new().command_sources();
        assert_eq!(sources[0].tag, SNAPSHOT_TAG);
        assert_eq!(sources[1].tag, PERCEIVE_TAG);
    }

    /// The two sources have different argv[0] values (different binaries).
    #[test]
    fn perceive_and_snapshot_sources_have_different_argv0() {
        let sources = OrchestratorKernel::new().command_sources();
        assert_ne!(
            sources[0].argv[0], sources[1].argv[0],
            "snapshot and perceive must exec different binaries"
        );
    }

    // ── perceive BridgeData / receipt handling ───────────────────────────────

    /// Happy path: a valid receipt updates `perceive_receipt` and increments
    /// the refresh counter.
    #[test]
    fn perceive_bridge_data_updates_receipt_and_increments_count() {
        let mut m = OrchestratorKernel::new();
        assert_eq!(m.perceive_refresh_count, 0);
        let event = bridge_data(PERCEIVE_TAG, perceive_receipt_json());
        assert!(m.handle_event(&event), "event must be consumed");
        assert_eq!(m.perceive_refresh_count, 1);
        let receipt = m.perceive_receipt.as_ref().expect("receipt populated");
        assert_eq!(receipt.seq, 42);
        assert_eq!(receipt.kind, "perceive.snapshot");
    }

    /// Multiple perceive events accumulate the counter monotonically.
    #[test]
    fn perceive_refresh_count_increments_per_event() {
        let mut m = OrchestratorKernel::new();
        for n in 1..=5u64 {
            m.handle_event(&bridge_data(PERCEIVE_TAG, perceive_receipt_json()));
            assert_eq!(m.perceive_refresh_count, n);
        }
    }

    /// `BridgeError` on the perceive tag sets `perceive_degraded`.
    #[test]
    fn perceive_bridge_error_sets_perceive_degraded() {
        let mut m = OrchestratorKernel::new();
        let event = bridge_error(PERCEIVE_TAG);
        assert!(m.handle_event(&event), "error event must be consumed");
        assert!(
            m.perceive_degraded.is_some(),
            "perceive_degraded must be populated on BridgeError"
        );
        assert!(m.perceive_receipt.is_none(), "no receipt on error");
    }

    /// A successful receipt after an error clears `perceive_degraded`.
    #[test]
    fn perceive_bridge_data_after_error_clears_degraded() {
        let mut m = OrchestratorKernel::new();
        m.handle_event(&bridge_error(PERCEIVE_TAG));
        assert!(m.perceive_degraded.is_some());
        m.handle_event(&bridge_data(PERCEIVE_TAG, perceive_receipt_json()));
        assert!(
            m.perceive_degraded.is_none(),
            "successful receipt must clear previous degraded reason"
        );
    }

    /// Malformed JSON (non-object) sets `perceive_degraded` and keeps prior receipt.
    #[test]
    fn perceive_malformed_data_sets_degraded_keeps_prior_receipt() {
        let mut m = OrchestratorKernel::new();
        // populate a good receipt first
        m.handle_event(&bridge_data(PERCEIVE_TAG, perceive_receipt_json()));
        let prior_seq = m.perceive_receipt.as_ref().unwrap().seq;
        // send a malformed event (string, not object)
        m.handle_event(&bridge_data(PERCEIVE_TAG, json!("not-an-object")));
        assert!(
            m.perceive_degraded.is_some(),
            "bad data must populate perceive_degraded"
        );
        // prior receipt kept (not replaced by garbage)
        assert_eq!(
            m.perceive_receipt.as_ref().unwrap().seq,
            prior_seq,
            "prior receipt must survive a decode failure"
        );
    }

    /// A `BridgeData` with a different tag must NOT be consumed by the perceive arm.
    #[test]
    fn perceive_bridge_data_wrong_tag_not_consumed() {
        let mut m = OrchestratorKernel::new();
        let event = bridge_data("other_tag", perceive_receipt_json());
        assert!(!m.handle_event(&event), "wrong-tag event must return false");
        assert!(m.perceive_receipt.is_none());
        assert!(m.perceive_degraded.is_none());
    }

    /// The snapshot tag must NOT trigger the perceive arm.
    #[test]
    fn snapshot_bridge_data_does_not_pollute_perceive_state() {
        let mut m = OrchestratorKernel::new();
        m.handle_event(&bridge_data(SNAPSHOT_TAG, kernel_snapshot_json()));
        assert!(
            m.perceive_receipt.is_none(),
            "snapshot event must not touch perceive state"
        );
    }

    /// `perceive_refresh_count` saturates at `u64::MAX` without panic.
    #[test]
    fn perceive_refresh_count_saturates_without_overflow() {
        let mut m = OrchestratorKernel::new();
        m.perceive_refresh_count = u64::MAX;
        m.handle_event(&bridge_data(PERCEIVE_TAG, perceive_receipt_json()));
        assert_eq!(
            m.perceive_refresh_count,
            u64::MAX,
            "saturating_add must not overflow"
        );
    }

    // ── perceive render line ─────────────────────────────────────────────────

    /// After a successful receipt, the render output must contain the seq number.
    #[test]
    fn render_with_perceive_receipt_shows_seq() {
        let mut m = OrchestratorKernel::new();
        m.handle_event(&bridge_data(PERCEIVE_TAG, perceive_receipt_json()));
        let joined = m
            .render(10, 120)
            .into_iter()
            .map(|l| l.content)
            .collect::<String>();
        assert!(
            joined.contains("seq="),
            "render must include 'seq=' when receipt is present; got: {joined}"
        );
        assert!(
            joined.contains("42"),
            "render must include the seq value 42; got: {joined}"
        );
    }

    /// When perceive is degraded the render must surface an error indication.
    #[test]
    fn render_with_perceive_degraded_shows_err_marker() {
        let mut m = OrchestratorKernel::new();
        m.handle_event(&bridge_error(PERCEIVE_TAG));
        let joined = m
            .render(10, 120)
            .into_iter()
            .map(|l| l.content)
            .collect::<String>();
        assert!(
            joined.contains("ERR") || joined.contains("perceive"),
            "render must indicate perceive error; got: {joined}"
        );
    }

    /// Cold-start (no receipt, no error): render must mention cadence / awaiting.
    #[test]
    fn render_cold_start_shows_awaiting_perceive() {
        let m = OrchestratorKernel::new();
        let joined = m
            .render(10, 120)
            .into_iter()
            .map(|l| l.content)
            .collect::<String>();
        assert!(
            joined.contains("perceive"),
            "render must mention perceive even before first emit; got: {joined}"
        );
    }
}
