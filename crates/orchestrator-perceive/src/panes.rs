//! Pane and session observation.
//!
//! Reads the live Zellij pane tree and session list and converts them into the
//! typed manifest fragments. Two strategies are supported: the body path (a
//! snapshot file written by the plugin's `command_sources()` self-poll) and the
//! host fallback (`zellij action dump-layout` plus `zellij list-sessions`).
//!
//! ## `ZellijCli` parsing
//!
//! `zellij list-sessions` emits one line per session; the current session is
//! marked with `(current)`.  `zellij action dump-layout` emits a KDL-like
//! text representation.  Because KDL parsing is heavyweight, we parse the layout
//! output with line-oriented heuristics that are sufficient for the manifest
//! (the goal is observation, not a full AST).

use std::fs;

use crate::error::PerceiveError;
use crate::exec::CommandRunner;
use crate::manifest::{ExitCode, PaneId, PaneObservation, SessionObservation, TabIndex};
use crate::Result;

/// Where pane state is sourced from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneInput {
    /// Read a body-emitted snapshot from an absolute file path.
    BodySnapshot {
        /// Absolute path to the body-written snapshot file.
        path: String,
    },
    /// Probe the live session via the `zellij` CLI.
    ZellijCli,
}

/// Collected pane and session observations.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PaneReport {
    /// Observed panes.
    pub panes: Vec<PaneObservation>,
    /// Observed sessions.
    pub sessions: Vec<SessionObservation>,
}

/// Observes the current panes and sessions.
///
/// When `input` is [`PaneInput::BodySnapshot`] the file at `path` is expected
/// to be a JSON object with `"panes"` and `"sessions"` keys holding arrays that
/// match [`PaneObservation`] and [`SessionObservation`] respectively.
///
/// When `input` is [`PaneInput::ZellijCli`] the runner is used to invoke
/// `zellij list-sessions` (session list) and `zellij action dump-layout`
/// (pane layout).  Both commands must be reachable at the conventional
/// `ZELLIJ_CLI` path or, if that is not available, the function falls back to
/// a best-effort empty report (no error is propagated for a missing Zellij
/// binary so that the assembler can still produce a partial snapshot from the
/// other sources).
///
/// # Errors
/// Returns an error if the body snapshot file cannot be read or is malformed
/// JSON.
pub fn observe(runner: &dyn CommandRunner, input: &PaneInput) -> Result<PaneReport> {
    match input {
        PaneInput::BodySnapshot { path } => observe_body(path),
        PaneInput::ZellijCli => Ok(observe_cli(runner)),
    }
}

// ---------------------------------------------------------------------------
// Body-snapshot path
// ---------------------------------------------------------------------------

fn observe_body(path: &str) -> Result<PaneReport> {
    let raw = fs::read_to_string(path).map_err(|err| PerceiveError::Subprocess {
        command: format!("read-body-snapshot:{path}"),
        detail: err.to_string(),
    })?;
    parse_body_snapshot(&raw)
}

fn parse_body_snapshot(raw: &str) -> Result<PaneReport> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|err| PerceiveError::Parse {
            source: "body-snapshot",
            detail: err.to_string(),
        })?;

    let panes = parse_panes_array(v.get("panes"))?;
    let sessions = parse_sessions_array(v.get("sessions"))?;
    Ok(PaneReport { panes, sessions })
}

fn parse_panes_array(val: Option<&serde_json::Value>) -> Result<Vec<PaneObservation>> {
    let arr = match val {
        Some(serde_json::Value::Array(a)) => a,
        Some(_) => {
            return Err(PerceiveError::Parse {
                source: "body-snapshot:panes",
                detail: "expected an array".to_string(),
            })
        }
        None => return Ok(Vec::new()),
    };

    arr.iter().map(parse_pane_object).collect()
}

fn parse_pane_object(v: &serde_json::Value) -> Result<PaneObservation> {
    let tab = v
        .get("tab")
        .and_then(serde_json::Value::as_u64)
        .map(|n| TabIndex::new(u32::try_from(n).unwrap_or(u32::MAX)))
        .ok_or(PerceiveError::Parse {
            source: "body-snapshot:pane.tab",
            detail: "missing or invalid".to_string(),
        })?;

    let pane_id = v
        .get("pane_id")
        .and_then(serde_json::Value::as_u64)
        .map(|n| PaneId::new(u32::try_from(n).unwrap_or(u32::MAX)))
        .ok_or(PerceiveError::Parse {
            source: "body-snapshot:pane.pane_id",
            detail: "missing or invalid".to_string(),
        })?;

    let pid = if let Some(pid_val) = v.get("pid") {
        if pid_val.is_null() {
            None
        } else {
            let raw = pid_val.as_u64().ok_or(PerceiveError::Parse {
                source: "body-snapshot:pane.pid",
                detail: "not a number".to_string(),
            })?;
            let pid32 = u32::try_from(raw).map_err(|_| PerceiveError::Parse {
                source: "body-snapshot:pane.pid",
                detail: "overflow".to_string(),
            })?;
            Some(crate::manifest::Pid::new(pid32)?)
        }
    } else {
        None
    };

    let exit_code = if let Some(ec_val) = v.get("exit_code") {
        if ec_val.is_null() {
            None
        } else {
            let raw = ec_val.as_i64().ok_or(PerceiveError::Parse {
                source: "body-snapshot:pane.exit_code",
                detail: "not a number".to_string(),
            })?;
            let ec32 = i32::try_from(raw).map_err(|_| PerceiveError::Parse {
                source: "body-snapshot:pane.exit_code",
                detail: "overflow".to_string(),
            })?;
            Some(ExitCode::new(ec32))
        }
    } else {
        None
    };

    Ok(PaneObservation {
        tab,
        tab_name: str_field(v, "tab_name", "body-snapshot:pane.tab_name")?,
        pos: str_field(v, "pos", "body-snapshot:pane.pos")?,
        pane_id,
        title: str_field(v, "title", "body-snapshot:pane.title")?,
        cwd: str_field(v, "cwd", "body-snapshot:pane.cwd")?,
        pid,
        running_command: str_field(
            v,
            "running_command",
            "body-snapshot:pane.running_command",
        )?,
        is_focused: v
            .get("is_focused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        exit_code,
    })
}

fn str_field(v: &serde_json::Value, key: &str, source: &'static str) -> Result<String> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(PerceiveError::Parse {
            source,
            detail: "missing or not a string".to_string(),
        })
}

fn parse_sessions_array(val: Option<&serde_json::Value>) -> Result<Vec<SessionObservation>> {
    let arr = match val {
        Some(serde_json::Value::Array(a)) => a,
        Some(_) => {
            return Err(PerceiveError::Parse {
                source: "body-snapshot:sessions",
                detail: "expected an array".to_string(),
            })
        }
        None => return Ok(Vec::new()),
    };
    arr.iter().map(parse_session_object).collect()
}

fn parse_session_object(v: &serde_json::Value) -> Result<SessionObservation> {
    Ok(SessionObservation {
        name: str_field(v, "name", "body-snapshot:session.name")?,
        is_current: v
            .get("is_current")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

// ---------------------------------------------------------------------------
// ZellijCli path
// ---------------------------------------------------------------------------

/// Default path to the zellij binary.
const ZELLIJ_PATH: &str = "/home/louranicas/.cargo/bin/zellij";

fn observe_cli(runner: &dyn CommandRunner) -> PaneReport {
    // Sessions — `zellij list-sessions`
    let sessions_out = runner.run(&[
        ZELLIJ_PATH.to_string(),
        "list-sessions".to_string(),
        "--no-formatting".to_string(),
    ]);

    let sessions = match sessions_out {
        Ok(out) if out.status == 0 => parse_session_lines(&out.stdout),
        // Zellij not available or not in a session — return empty, not an error
        _ => Vec::new(),
    };

    // Panes — `zellij action dump-layout`
    let layout_out = runner.run(&[
        ZELLIJ_PATH.to_string(),
        "action".to_string(),
        "dump-layout".to_string(),
    ]);

    let panes = match layout_out {
        Ok(out) if out.status == 0 => parse_layout_panes(&out.stdout),
        _ => Vec::new(),
    };

    PaneReport { panes, sessions }
}

fn parse_session_lines(text: &str) -> Vec<SessionObservation> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let is_current = line.contains("(current)");
            let name = line
                .split_whitespace()
                .next()
                .unwrap_or(line)
                .trim()
                .to_string();
            SessionObservation { name, is_current }
        })
        .collect()
}

/// Heuristic parser for `zellij action dump-layout` output.
///
/// The output is KDL-like but we only need pane IDs and titles for the
/// manifest; a full parser is not warranted here.
fn parse_layout_panes(text: &str) -> Vec<PaneObservation> {
    let mut panes = Vec::new();
    let mut pane_counter: u32 = 0;
    let mut tab_index: u32 = 0;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("tab") {
            tab_index += 1;
        } else if trimmed.starts_with("pane") {
            pane_counter += 1;
            // Extract pane_id if present: pane id=<num>
            let id = extract_attr(trimmed, "id")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(pane_counter);
            let title = extract_attr(trimmed, "name").unwrap_or_default();
            let cwd = extract_attr(trimmed, "cwd").unwrap_or_default();
            let focused = trimmed.contains("focus=true");

            panes.push(PaneObservation {
                tab: TabIndex::new(tab_index),
                tab_name: format!("tab-{tab_index}"),
                pos: String::new(),
                pane_id: PaneId::new(id),
                title,
                cwd,
                pid: None,
                running_command: String::new(),
                is_focused: focused,
                exit_code: None,
            });
        }
    }
    panes
}

/// Extracts `key="value"` or `key=value` from a KDL-like attribute string.
fn extract_attr(text: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = text.find(needle.as_str())?;
    let rest = &text[start + needle.len()..];
    if rest.starts_with('"') {
        // Quoted string
        let inner = rest.trim_start_matches('"');
        let end = inner.find('"').unwrap_or(inner.len());
        Some(inner[..end].to_string())
    } else {
        // Bare token
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::CommandOutput;

    // -----------------------------------------------------------------------
    // FakeRunner wired for panes tests
    // -----------------------------------------------------------------------

    struct FixedRunner {
        sessions: CommandOutput,
        layout: CommandOutput,
    }

    impl CommandRunner for FixedRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput> {
            if argv.iter().any(|a| a == "list-sessions") {
                Ok(self.sessions.clone())
            } else {
                Ok(self.layout.clone())
            }
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn fail() -> CommandOutput {
        CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Body-snapshot tests
    // -----------------------------------------------------------------------

    #[test]
    fn body_snapshot_empty_arrays() {
        let raw = r#"{"panes":[],"sessions":[]}"#;
        let report = parse_body_snapshot(raw).unwrap();
        assert!(report.panes.is_empty());
        assert!(report.sessions.is_empty());
    }

    #[test]
    fn body_snapshot_missing_panes_key_gives_empty() {
        let raw = r#"{"sessions":[]}"#;
        let report = parse_body_snapshot(raw).unwrap();
        assert!(report.panes.is_empty());
    }

    #[test]
    fn body_snapshot_missing_sessions_key_gives_empty() {
        let raw = r#"{"panes":[]}"#;
        let report = parse_body_snapshot(raw).unwrap();
        assert!(report.sessions.is_empty());
    }

    #[test]
    fn body_snapshot_parses_one_pane() {
        let raw = r#"{
            "panes":[{"tab":0,"tab_name":"main","pos":"0,0","pane_id":1,"title":"editor","cwd":"/home","running_command":"nvim","is_focused":true,"pid":null,"exit_code":null}],
            "sessions":[]
        }"#;
        let report = parse_body_snapshot(raw).unwrap();
        assert_eq!(report.panes.len(), 1);
        let p = &report.panes[0];
        assert_eq!(p.tab.get(), 0);
        assert_eq!(p.pane_id.get(), 1);
        assert_eq!(p.title, "editor");
        assert!(p.is_focused);
        assert!(p.pid.is_none());
        assert!(p.exit_code.is_none());
    }

    #[test]
    fn body_snapshot_parses_pid_and_exit_code() {
        let raw = r#"{
            "panes":[{"tab":0,"tab_name":"t","pos":"","pane_id":2,"title":"","cwd":"","running_command":"","is_focused":false,"pid":1234,"exit_code":0}],
            "sessions":[]
        }"#;
        let report = parse_body_snapshot(raw).unwrap();
        let p = &report.panes[0];
        assert_eq!(p.pid.unwrap().get(), 1234);
        assert_eq!(p.exit_code.unwrap().get(), 0);
    }

    #[test]
    fn body_snapshot_parses_sessions() {
        let raw = r#"{
            "panes":[],
            "sessions":[{"name":"main","is_current":true},{"name":"dev","is_current":false}]
        }"#;
        let report = parse_body_snapshot(raw).unwrap();
        assert_eq!(report.sessions.len(), 2);
        assert!(report.sessions[0].is_current);
        assert!(!report.sessions[1].is_current);
    }

    #[test]
    fn body_snapshot_invalid_json_returns_parse_error() {
        let err = parse_body_snapshot("{not valid json").unwrap_err();
        assert!(matches!(err, PerceiveError::Parse { .. }));
    }

    #[test]
    fn body_snapshot_panes_not_array_returns_parse_error() {
        let raw = r#"{"panes":"oops","sessions":[]}"#;
        let err = parse_body_snapshot(raw).unwrap_err();
        assert!(matches!(err, PerceiveError::Parse { .. }));
    }

    #[test]
    fn body_snapshot_missing_required_field_returns_error() {
        // missing "title"
        let raw = r#"{"panes":[{"tab":0,"tab_name":"t","pos":"","pane_id":1,"cwd":"","running_command":"","is_focused":false}],"sessions":[]}"#;
        let err = parse_body_snapshot(raw).unwrap_err();
        assert!(matches!(err, PerceiveError::Parse { .. }));
    }

    #[test]
    fn body_snapshot_zero_pid_returns_out_of_range() {
        let raw = r#"{"panes":[{"tab":0,"tab_name":"t","pos":"","pane_id":1,"title":"","cwd":"","running_command":"","is_focused":false,"pid":0,"exit_code":null}],"sessions":[]}"#;
        let err = parse_body_snapshot(raw).unwrap_err();
        assert!(matches!(err, PerceiveError::OutOfRange { .. }));
    }

    // -----------------------------------------------------------------------
    // BodySnapshot file path tests
    // -----------------------------------------------------------------------

    #[test]
    fn observe_body_missing_file_returns_subprocess_error() {
        // File read goes through fs, not through the runner; SystemRunner used here
        // just to satisfy the type parameter (it's never invoked for BodySnapshot).
        let err = observe(
            &crate::exec::SystemRunner,
            &PaneInput::BodySnapshot {
                path: "/nonexistent/path/snapshot.json".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, PerceiveError::Subprocess { .. }));
    }

    // -----------------------------------------------------------------------
    // ZellijCli path tests
    // -----------------------------------------------------------------------

    #[test]
    fn zellij_cli_happy_path_parses_sessions() {
        let runner = FixedRunner {
            sessions: ok("main (current)\ndev\n"),
            layout: ok(""),
        };
        let report = observe(&runner, &PaneInput::ZellijCli).unwrap();
        assert_eq!(report.sessions.len(), 2);
        assert!(report.sessions[0].is_current);
        assert!(!report.sessions[1].is_current);
    }

    #[test]
    fn zellij_cli_session_names_extracted() {
        let runner = FixedRunner {
            sessions: ok("alpha (current)\nbeta\ngamma\n"),
            layout: ok(""),
        };
        let report = observe(&runner, &PaneInput::ZellijCli).unwrap();
        let names: Vec<_> = report.sessions.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn zellij_cli_failed_sessions_gives_empty() {
        let runner = FixedRunner {
            sessions: fail(),
            layout: ok(""),
        };
        let report = observe(&runner, &PaneInput::ZellijCli).unwrap();
        assert!(report.sessions.is_empty());
    }

    #[test]
    fn zellij_cli_failed_layout_gives_empty_panes() {
        let runner = FixedRunner {
            sessions: ok("main (current)\n"),
            layout: fail(),
        };
        let report = observe(&runner, &PaneInput::ZellijCli).unwrap();
        assert!(report.panes.is_empty());
    }

    #[test]
    fn parse_layout_panes_empty_string() {
        let panes = parse_layout_panes("");
        assert!(panes.is_empty());
    }

    #[test]
    fn parse_layout_panes_increments_tab_on_tab_keyword() {
        let layout = "tab\npane name=\"editor\"\ntab\npane name=\"shell\"\n";
        let panes = parse_layout_panes(layout);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].tab.get(), 1);
        assert_eq!(panes[1].tab.get(), 2);
    }

    #[test]
    fn parse_layout_panes_extracts_id_attr() {
        let layout = "pane id=5 name=\"foo\"\n";
        let panes = parse_layout_panes(layout);
        assert_eq!(panes[0].pane_id.get(), 5);
    }

    #[test]
    fn parse_layout_panes_focus_detected() {
        let layout = "pane focus=true name=\"x\"\n";
        let panes = parse_layout_panes(layout);
        assert!(panes[0].is_focused);
    }

    #[test]
    fn extract_attr_quoted_value() {
        let result = extract_attr(r#"pane name="my tab""#, "name");
        assert_eq!(result.unwrap(), "my tab");
    }

    #[test]
    fn extract_attr_bare_value() {
        let result = extract_attr("pane id=42 name=\"x\"", "id");
        assert_eq!(result.unwrap(), "42");
    }

    #[test]
    fn extract_attr_absent_key_returns_none() {
        let result = extract_attr("pane name=\"x\"", "cwd");
        assert!(result.is_none());
    }

}
