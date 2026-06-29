//! Top-level manifest composition.
//!
//! Drives every source module through a single [`crate::exec::CommandRunner`] and
//! folds the results into one [`PerceiveSnapshot`]. This is the L1 join point
//! referenced by the build plan: panes, engines, catalog, leases, and fibers are
//! gathered, stamped with a capture time and provenance, and returned for
//! emission.

use std::time::SystemTime;

use crate::exec::CommandRunner;
use crate::manifest::{PerceiveSnapshot, Source, TimestampMs, SCHEMA};
use crate::{catalog, engines, fibers, leases, panes};
use crate::Result;

/// Inputs required to assemble a full manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestInputs {
    /// Where pane and session state is read from.
    pub panes: panes::PaneInput,
    /// Engine endpoints to probe.
    pub engines: Vec<engines::ProbeTarget>,
    /// Filesystem roots for the catalog scan.
    pub catalog: catalog::ScanRoots,
    /// Lease resource keys to observe.
    pub leases: Vec<String>,
    /// Provenance recorded in the manifest.
    pub source: Source,
}

/// Assembles a complete [`PerceiveSnapshot`] from all observation sources.
///
/// Errors from the panes, catalog, and engines sources are propagated
/// immediately.  Leases and fibers use best-effort collection: individual
/// probe failures are swallowed internally by [`leases::observe`] and
/// [`fibers::observe`] rather than stopping the overall assembly.
///
/// # Errors
/// Returns the first error produced by pane observation, engine probing, or
/// catalog scanning.
pub fn assemble(runner: &dyn CommandRunner, inputs: &ManifestInputs) -> Result<PerceiveSnapshot> {
    let captured_at_ms = TimestampMs::from_millis(current_epoch_ms());

    let pane_report = panes::observe(runner, &inputs.panes)?;
    let engine_probes = engines::probe(runner, &inputs.engines)?;
    let catalog_obs = catalog::assemble(runner, &inputs.catalog)?;
    let lease_obs = leases::observe(runner, &inputs.leases)?;
    let fiber_obs = fibers::observe(runner)?;

    Ok(PerceiveSnapshot {
        schema: SCHEMA.to_string(),
        captured_at_ms,
        source: inputs.source,
        panes: pane_report.panes,
        sessions: pane_report.sessions,
        engines: engine_probes,
        catalog: catalog_obs,
        leases: lease_obs,
        fibers: fiber_obs,
    })
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};
    use crate::engines::ProbeTarget;
    use crate::manifest::{Port, SCHEMA};
    use crate::panes::PaneInput;

    // -----------------------------------------------------------------------
    // Multi-source fake runner
    // -----------------------------------------------------------------------

    /// Routes commands by inspecting argv[0] and argv[1].
    struct MultiRunner {
        sessions_stdout: String,
        layout_stdout: String,
        curl_code: String,
        just_stdout: String,
        kv_lease_json: Option<String>,
        hopf_stdout: String,
    }

    impl Default for MultiRunner {
        fn default() -> Self {
            Self {
                sessions_stdout: String::new(),
                layout_stdout: String::new(),
                curl_code: "200".to_string(),
                just_stdout: String::new(),
                kv_lease_json: None,
                hopf_stdout: String::new(),
            }
        }
    }

    impl CommandRunner for MultiRunner {
        fn run(&self, argv: &[String]) -> crate::Result<CommandOutput> {
            let cmd = argv.first().map_or("", String::as_str);
            let sub = argv.get(1).map_or("", String::as_str);

            let stdout = if cmd.contains("curl") {
                self.curl_code.clone()
            } else if cmd.contains("zellij") && sub == "list-sessions" {
                self.sessions_stdout.clone()
            } else if cmd.contains("zellij") {
                self.layout_stdout.clone()
            } else if cmd.contains("just") {
                self.just_stdout.clone()
            } else if cmd.contains("kv-lease") {
                match &self.kv_lease_json {
                    Some(j) => return Ok(CommandOutput {
                        status: 0,
                        stdout: j.clone(),
                        stderr: String::new(),
                    }),
                    None => return Ok(CommandOutput {
                        status: 1,
                        stdout: String::new(),
                        stderr: String::new(),
                    }),
                }
            } else if cmd.contains("hopf-anchor") {
                self.hopf_stdout.clone()
            } else {
                String::new()
            };

            Ok(CommandOutput {
                status: 0,
                stdout,
                stderr: String::new(),
            })
        }
    }

    fn default_inputs() -> ManifestInputs {
        ManifestInputs {
            panes: PaneInput::ZellijCli,
            engines: vec![ProbeTarget {
                name: "WFE".to_string(),
                port: Port::new(8142).unwrap(),
                health_path: "/health".to_string(),
            }],
            catalog: catalog::ScanRoots {
                workflows_dir: "/nonexistent/workflows".to_string(),
                agents_dir: "/nonexistent/agents".to_string(),
                just_dir: "/nonexistent".to_string(),
            },
            leases: Vec::new(),
            source: Source::HostHelper,
        }
    }

    // -----------------------------------------------------------------------
    // Schema / shape tests
    // -----------------------------------------------------------------------

    #[test]
    fn assemble_schema_field_equals_const() {
        let runner = MultiRunner::default();
        let inputs = default_inputs();
        let snap = assemble(&runner, &inputs).unwrap();
        assert_eq!(snap.schema, SCHEMA);
    }

    #[test]
    fn assemble_source_field_matches_input() {
        let runner = MultiRunner::default();
        let mut inputs = default_inputs();
        inputs.source = Source::Body;
        let snap = assemble(&runner, &inputs).unwrap();
        assert_eq!(snap.source, Source::Body);
    }

    #[test]
    fn assemble_captured_at_ms_is_nonzero() {
        let runner = MultiRunner::default();
        let snap = assemble(&runner, &default_inputs()).unwrap();
        assert!(snap.captured_at_ms.get() > 0);
    }

    // -----------------------------------------------------------------------
    // Serde round-trip (JSON shape matches contract)
    // -----------------------------------------------------------------------

    #[test]
    fn assemble_snapshot_serialises_to_json() {
        let runner = MultiRunner::default();
        let snap = assemble(&runner, &default_inputs()).unwrap();
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"schema\""));
        assert!(json.contains("\"captured_at_ms\""));
        assert!(json.contains("\"source\""));
        assert!(json.contains("\"panes\""));
        assert!(json.contains("\"sessions\""));
        assert!(json.contains("\"engines\""));
        assert!(json.contains("\"catalog\""));
        assert!(json.contains("\"leases\""));
        assert!(json.contains("\"fibers\""));
    }

    #[test]
    fn assemble_snapshot_json_schema_value_is_v1() {
        let runner = MultiRunner::default();
        let snap = assemble(&runner, &default_inputs()).unwrap();
        let json = serde_json::to_string(&snap).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["schema"].as_str().unwrap(), "perceive.snapshot.v1");
    }

    #[test]
    fn assemble_snapshot_json_panes_is_array() {
        let runner = MultiRunner::default();
        let snap = assemble(&runner, &default_inputs()).unwrap();
        let json = serde_json::to_string(&snap).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["panes"].is_array());
    }

    #[test]
    fn assemble_snapshot_json_engines_is_array() {
        let runner = MultiRunner::default();
        let snap = assemble(&runner, &default_inputs()).unwrap();
        let json = serde_json::to_string(&snap).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["engines"].is_array());
    }

    #[test]
    fn assemble_engines_count_matches_targets() {
        let runner = MultiRunner::default();
        let mut inputs = default_inputs();
        inputs.engines = engines::default_targets();
        let snap = assemble(&runner, &inputs).unwrap();
        assert_eq!(snap.engines.len(), inputs.engines.len());
    }

    // -----------------------------------------------------------------------
    // Source propagation
    // -----------------------------------------------------------------------

    #[test]
    fn assemble_host_helper_source() {
        let runner = MultiRunner::default();
        let mut inputs = default_inputs();
        inputs.source = Source::HostHelper;
        let snap = assemble(&runner, &inputs).unwrap();
        assert_eq!(snap.source, Source::HostHelper);
    }

    #[test]
    fn assemble_body_source() {
        let runner = MultiRunner::default();
        let mut inputs = default_inputs();
        inputs.source = Source::Body;
        let snap = assemble(&runner, &inputs).unwrap();
        assert_eq!(snap.source, Source::Body);
    }

    // -----------------------------------------------------------------------
    // Degraded-input tests (best-effort sources)
    // -----------------------------------------------------------------------

    #[test]
    fn assemble_with_no_leases_input_gives_empty_leases() {
        let runner = MultiRunner::default();
        let mut inputs = default_inputs();
        inputs.leases = Vec::new();
        let snap = assemble(&runner, &inputs).unwrap();
        assert!(snap.leases.is_empty());
    }

    #[test]
    fn assemble_with_hopf_unavailable_gives_empty_fibers() {
        // hopf_stdout empty → hopf fails → fibers empty
        let runner = MultiRunner {
            hopf_stdout: String::new(),
            ..MultiRunner::default()
        };
        let snap = assemble(&runner, &default_inputs()).unwrap();
        assert!(snap.fibers.is_empty());
    }

    #[test]
    fn assemble_with_sessions_text_populates_sessions() {
        let runner = MultiRunner {
            sessions_stdout: "main (current)\ndev\n".to_string(),
            ..MultiRunner::default()
        };
        let snap = assemble(&runner, &default_inputs()).unwrap();
        assert_eq!(snap.sessions.len(), 2);
    }

    #[test]
    fn assemble_with_fibers_json_populates_fibers() {
        let runner = MultiRunner {
            hopf_stdout: r#"[{"campaign":"c1","root":"/r","loops":["la"]}]"#.to_string(),
            ..MultiRunner::default()
        };
        let snap = assemble(&runner, &default_inputs()).unwrap();
        assert_eq!(snap.fibers.len(), 1);
        assert_eq!(snap.fibers[0].campaign, "c1");
    }
}
