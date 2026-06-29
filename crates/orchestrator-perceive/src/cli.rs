//! Command-line surface for the `orchestrator-perceive` binary.
//!
//! Parses arguments (for example `--emit-from-body`, `--dry-run`, target
//! overrides), builds the [`crate::assemble::ManifestInputs`], assembles the
//! manifest with an [`crate::exec::SystemRunner`], and (unless dry-run) emits it
//! via [`crate::emit::append_snapshot`]. Returns the process exit code.
//!
//! ## Accepted flags
//!
//! | Flag | Effect |
//! |------|--------|
//! | `--emit-from-body` | Use [`crate::panes::PaneInput::BodySnapshot`] (reads body-written file at `--body-snapshot-path`) |
//! | `--body-snapshot-path <path>` | Override the body snapshot path (default: `$PERCEIVE_BODY_SNAPSHOT`) |
//! | `--dry-run` | Assemble but do not emit; print JSON to stdout |
//! | `--ctl-path <path>` | Override `orch-kernelctl` path (default `/home/louranicas/.local/bin/orch-kernelctl`) |
//! | `--actor <name>` | Override the actor name (default `host-helper`) |
//! | `--workspace <dir>` | Override the workspace root used for catalog scanning (default `/home/louranicas/claude-code-workspace`) |

use crate::assemble::{assemble, ManifestInputs};
use crate::catalog::ScanRoots;
use crate::emit::append_snapshot;
use crate::engines::default_targets;
use crate::error::PerceiveError;
use crate::exec::SystemRunner;
use crate::manifest::Source;
use crate::panes::PaneInput;
use crate::Result;

/// Default path to the `orch-kernelctl` binary.
const DEFAULT_CTL_PATH: &str = "/home/louranicas/.local/bin/orch-kernelctl";

/// Default workspace root used to locate `.claude/workflows` and `.claude/agents`.
const DEFAULT_WORKSPACE: &str = "/home/louranicas/claude-code-workspace";

/// Default actor written into the append event.
const DEFAULT_ACTOR: &str = "host-helper";

/// Parsed CLI configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CliConfig {
    emit_from_body: bool,
    body_snapshot_path: Option<String>,
    dry_run: bool,
    ctl_path: String,
    actor: String,
    workspace: String,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            emit_from_body: false,
            body_snapshot_path: None,
            dry_run: false,
            ctl_path: DEFAULT_CTL_PATH.to_string(),
            actor: DEFAULT_ACTOR.to_string(),
            workspace: DEFAULT_WORKSPACE.to_string(),
        }
    }
}

/// Parses CLI arguments and runs the perception assembler.
///
/// Returns the process exit code (`0` on success, `1` on any error).
///
/// # Errors
/// Returns a [`PerceiveError`] if argument parsing fails (for example an unknown
/// flag or a missing value), or if assembly or emission fails.
pub fn run(args: &[String]) -> Result<i32> {
    let config = parse_args(args)?;
    let inputs = build_inputs(&config)?;
    let runner = SystemRunner;

    let snapshot = assemble(&runner, &inputs)?;

    if config.dry_run {
        let json = serde_json::to_string_pretty(&snapshot)?;
        println!("{json}");
        return Ok(0);
    }

    append_snapshot(&runner, &config.ctl_path, &config.actor, &snapshot)?;
    Ok(0)
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args(args: &[String]) -> Result<CliConfig> {
    let mut cfg = CliConfig::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--emit-from-body" => {
                cfg.emit_from_body = true;
            }
            "--dry-run" => {
                cfg.dry_run = true;
            }
            "--body-snapshot-path" => {
                i += 1;
                cfg.body_snapshot_path = Some(require_value(args, i, "--body-snapshot-path")?);
            }
            "--ctl-path" => {
                i += 1;
                cfg.ctl_path = require_value(args, i, "--ctl-path")?;
            }
            "--actor" => {
                i += 1;
                cfg.actor = require_value(args, i, "--actor")?;
            }
            "--workspace" => {
                i += 1;
                cfg.workspace = require_value(args, i, "--workspace")?;
            }
            other => {
                return Err(PerceiveError::Parse {
                    source: "cli",
                    detail: format!("unknown argument: {other}"),
                });
            }
        }
        i += 1;
    }
    Ok(cfg)
}

fn require_value(args: &[String], idx: usize, flag: &'static str) -> Result<String> {
    args.get(idx)
        .filter(|v| !v.starts_with("--"))
        .cloned()
        .ok_or(PerceiveError::Empty { field: flag })
}

// ---------------------------------------------------------------------------
// Input construction
// ---------------------------------------------------------------------------

fn build_inputs(cfg: &CliConfig) -> Result<ManifestInputs> {
    let panes = if cfg.emit_from_body {
        let path = match &cfg.body_snapshot_path {
            Some(p) => p.clone(),
            None => {
                std::env::var("PERCEIVE_BODY_SNAPSHOT").map_err(|_| PerceiveError::Empty {
                    field: "body-snapshot-path (set --body-snapshot-path or PERCEIVE_BODY_SNAPSHOT)",
                })?
            }
        };
        PaneInput::BodySnapshot { path }
    } else {
        PaneInput::ZellijCli
    };

    Ok(ManifestInputs {
        panes,
        engines: default_targets(),
        catalog: ScanRoots {
            workflows_dir: format!("{}/.claude/workflows", cfg.workspace),
            agents_dir: format!("{}/.claude/agents", cfg.workspace),
            just_dir: cfg.workspace.clone(),
        },
        leases: Vec::new(),
        source: if cfg.emit_from_body {
            Source::Body
        } else {
            Source::HostHelper
        },
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|&a| a.to_string()).collect()
    }

    // -----------------------------------------------------------------------
    // parse_args tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_args_empty_gives_defaults() {
        let cfg = parse_args(&[]).unwrap();
        assert!(!cfg.emit_from_body);
        assert!(!cfg.dry_run);
        assert_eq!(cfg.ctl_path, DEFAULT_CTL_PATH);
        assert_eq!(cfg.actor, DEFAULT_ACTOR);
        assert_eq!(cfg.workspace, DEFAULT_WORKSPACE);
    }

    #[test]
    fn parse_args_emit_from_body_flag() {
        let cfg = parse_args(&args(&["--emit-from-body"])).unwrap();
        assert!(cfg.emit_from_body);
    }

    #[test]
    fn parse_args_dry_run_flag() {
        let cfg = parse_args(&args(&["--dry-run"])).unwrap();
        assert!(cfg.dry_run);
    }

    #[test]
    fn parse_args_ctl_path_override() {
        let cfg = parse_args(&args(&["--ctl-path", "/custom/orch-kernelctl"])).unwrap();
        assert_eq!(cfg.ctl_path, "/custom/orch-kernelctl");
    }

    #[test]
    fn parse_args_actor_override() {
        let cfg = parse_args(&args(&["--actor", "my-body"])).unwrap();
        assert_eq!(cfg.actor, "my-body");
    }

    #[test]
    fn parse_args_workspace_override() {
        let cfg = parse_args(&args(&["--workspace", "/tmp/workspace"])).unwrap();
        assert_eq!(cfg.workspace, "/tmp/workspace");
    }

    #[test]
    fn parse_args_body_snapshot_path_override() {
        let cfg = parse_args(&args(&[
            "--emit-from-body",
            "--body-snapshot-path",
            "/tmp/snap.json",
        ]))
        .unwrap();
        assert_eq!(cfg.body_snapshot_path, Some("/tmp/snap.json".to_string()));
    }

    #[test]
    fn parse_args_unknown_flag_returns_parse_error() {
        let err = parse_args(&args(&["--unknown-flag"])).unwrap_err();
        assert!(matches!(err, PerceiveError::Parse { .. }));
    }

    #[test]
    fn parse_args_flag_missing_value_returns_empty_error() {
        let err = parse_args(&args(&["--ctl-path"])).unwrap_err();
        assert!(matches!(err, PerceiveError::Empty { .. }));
    }

    #[test]
    fn parse_args_actor_missing_value_returns_empty() {
        let err = parse_args(&args(&["--actor"])).unwrap_err();
        assert!(matches!(err, PerceiveError::Empty { .. }));
    }

    #[test]
    fn parse_args_combined_flags() {
        let cfg = parse_args(&args(&[
            "--dry-run",
            "--actor",
            "my-actor",
            "--ctl-path",
            "/ctl",
        ]))
        .unwrap();
        assert!(cfg.dry_run);
        assert_eq!(cfg.actor, "my-actor");
        assert_eq!(cfg.ctl_path, "/ctl");
    }

    #[test]
    fn parse_args_workspace_sets_scan_roots() {
        let cfg = parse_args(&args(&["--workspace", "/custom"])).unwrap();
        let inputs = build_inputs(&cfg).unwrap();
        assert_eq!(inputs.catalog.workflows_dir, "/custom/.claude/workflows");
        assert_eq!(inputs.catalog.agents_dir, "/custom/.claude/agents");
        assert_eq!(inputs.catalog.just_dir, "/custom");
    }

    // -----------------------------------------------------------------------
    // build_inputs tests
    // -----------------------------------------------------------------------

    #[test]
    fn build_inputs_zellij_cli_by_default() {
        let cfg = CliConfig::default();
        let inputs = build_inputs(&cfg).unwrap();
        assert_eq!(inputs.panes, crate::panes::PaneInput::ZellijCli);
    }

    #[test]
    fn build_inputs_emit_from_body_requires_path_or_env() {
        // Without env var and without --body-snapshot-path this must error
        std::env::remove_var("PERCEIVE_BODY_SNAPSHOT");
        let cfg = CliConfig {
            emit_from_body: true,
            ..CliConfig::default()
        };
        let err = build_inputs(&cfg).unwrap_err();
        assert!(matches!(err, PerceiveError::Empty { .. }));
    }

    #[test]
    fn build_inputs_body_snapshot_path_used_when_set() {
        let cfg = CliConfig {
            emit_from_body: true,
            body_snapshot_path: Some("/tmp/snap.json".to_string()),
            ..CliConfig::default()
        };
        let inputs = build_inputs(&cfg).unwrap();
        assert_eq!(
            inputs.panes,
            crate::panes::PaneInput::BodySnapshot {
                path: "/tmp/snap.json".to_string()
            }
        );
    }

    #[test]
    fn build_inputs_source_is_host_helper_by_default() {
        let cfg = CliConfig::default();
        let inputs = build_inputs(&cfg).unwrap();
        assert_eq!(inputs.source, crate::manifest::Source::HostHelper);
    }

    #[test]
    fn build_inputs_source_is_body_when_emit_from_body() {
        let cfg = CliConfig {
            emit_from_body: true,
            body_snapshot_path: Some("/tmp/s.json".to_string()),
            ..CliConfig::default()
        };
        let inputs = build_inputs(&cfg).unwrap();
        assert_eq!(inputs.source, crate::manifest::Source::Body);
    }

    #[test]
    fn build_inputs_engines_uses_default_targets() {
        let cfg = CliConfig::default();
        let inputs = build_inputs(&cfg).unwrap();
        assert!(!inputs.engines.is_empty());
        assert_eq!(inputs.engines.len(), default_targets().len());
    }

    // -----------------------------------------------------------------------
    // require_value tests
    // -----------------------------------------------------------------------

    #[test]
    fn require_value_returns_value_at_index() {
        let a = args(&["--flag", "myvalue"]);
        assert_eq!(require_value(&a, 1, "--flag").unwrap(), "myvalue");
    }

    #[test]
    fn require_value_out_of_bounds_returns_empty() {
        let a = args(&["--flag"]);
        let err = require_value(&a, 1, "--flag").unwrap_err();
        assert!(matches!(err, PerceiveError::Empty { .. }));
    }

    #[test]
    fn require_value_next_arg_is_flag_returns_empty() {
        let a = args(&["--actor", "--dry-run"]);
        let err = require_value(&a, 1, "--actor").unwrap_err();
        assert!(matches!(err, PerceiveError::Empty { .. }));
    }
}
