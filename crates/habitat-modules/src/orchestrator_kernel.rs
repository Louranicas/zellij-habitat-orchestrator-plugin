use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{CommandSource, DataSource, HabitatModule};
use habitat_core::render::*;
use serde::Deserialize;

const SNAPSHOT_TAG: &str = "kernel_snapshot";
const DEFAULT_SIDECAR_CLI: &str = "/home/louranicas/.local/bin/orch-kernelctl";
const KERNEL_MODULE_VERSION: &str = "0.1.1";

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

pub struct OrchestratorKernel {
    sidecar_cli: String,
    poll_secs: f64,
    snapshot: Option<KernelSnapshot>,
    degraded_reason: Option<String>,
    refresh_count: u64,
}

impl OrchestratorKernel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sidecar_cli: DEFAULT_SIDECAR_CLI.into(),
            poll_secs: 5.0,
            snapshot: None,
            degraded_reason: Some("awaiting sidecar snapshot".into()),
            refresh_count: 0,
        }
    }
}

impl Default for OrchestratorKernel {
    fn default() -> Self {
        Self::new()
    }
}

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

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
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

    fn command_sources(&self) -> Vec<CommandSource> {
        vec![CommandSource {
            argv: vec![self.sidecar_cli.clone(), "snapshot".into(), "--json".into()],
            interval_secs: self.poll_secs,
            tag: SNAPSHOT_TAG.into(),
            module_id: self.id().into(),
        }]
    }
}

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_source_uses_snapshot_json_command() {
        let module = OrchestratorKernel::new();
        let sources = module.command_sources();
        assert_eq!(module.version(), KERNEL_MODULE_VERSION);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].argv[1], "snapshot");
        assert_eq!(sources[0].argv[2], "--json");
        assert_eq!(sources[0].tag, SNAPSHOT_TAG);
    }

    #[test]
    fn bridge_data_updates_snapshot() {
        let mut module = OrchestratorKernel::new();
        let event = HabitatEvent::BridgeData {
            module_id: module.id().into(),
            tag: SNAPSHOT_TAG.into(),
            data: json!({
                "status": "ok",
                "db_path": "/tmp/orchestrator-kernel.sqlite",
                "last_seq": 7,
                "last_hash": "sha256:abcdef",
                "event_count": 7,
                "verify_chain_ok": true,
                "schema_version": 1
            }),
        };
        assert!(module.handle_event(&event));
        assert!(module.degraded_reason.is_none());
        assert_eq!(module.snapshot.as_ref().map(|s| s.last_seq), Some(7));
    }

    #[test]
    fn bridge_error_renders_degraded() {
        let mut module = OrchestratorKernel::new();
        let event = HabitatEvent::BridgeError {
            module_id: module.id().into(),
            tag: SNAPSHOT_TAG.into(),
        };
        assert!(module.handle_event(&event));
        let rendered = module
            .render(10, 100)
            .into_iter()
            .map(|l| l.content)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("DEGRADED"));
    }
}
