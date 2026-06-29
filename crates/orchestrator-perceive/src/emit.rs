//! Spine emission.
//!
//! Serializes a [`PerceiveSnapshot`] and appends it to the orchestrator-kernel
//! spine as a `perceive.snapshot` event. This is an observation event, NOT task
//! admission: it uses the warrant-free `append` verb (via `orch-kernelctl append`)
//! and never the warrant-gated `submit` path. The append-only chain invariant is
//! preserved (`verify-chain` stays green).

use crate::error::PerceiveError;
use crate::exec::CommandRunner;
use crate::manifest::PerceiveSnapshot;
use crate::Result;

/// The event kind under which perception snapshots are appended.
pub const EVENT_KIND: &str = "perceive.snapshot";

/// Receipt returned after a successful append.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendReceipt {
    /// Monotonic sequence assigned by the spine.
    pub seq: i64,
    /// Durable event id assigned by the spine.
    pub event_id: String,
}

/// Serializes `snapshot` and appends it to the spine via `orch-kernelctl append`.
///
/// `ctl_path` is the absolute path to the `orch-kernelctl` binary and `actor`
/// identifies the writer (for example `"host-helper"` or `"body"`).
///
/// The command executed is:
/// ```text
/// <ctl_path> append --kind perceive.snapshot --actor <actor> --payload <json>
/// ```
///
/// The receipt JSON printed by `orch-kernelctl` is expected to contain at least
/// `"seq"` (integer) and `"event_id"` (string) keys.
///
/// # Errors
/// Returns [`PerceiveError::Empty`] if `ctl_path` or `actor` is empty.
/// Returns [`PerceiveError::Json`] if the snapshot cannot be serialized.
/// Returns [`PerceiveError::Subprocess`] if the command fails to run or exits
/// with a non-zero status.
/// Returns [`PerceiveError::Parse`] if the receipt JSON cannot be decoded.
pub fn append_snapshot(
    runner: &dyn CommandRunner,
    ctl_path: &str,
    actor: &str,
    snapshot: &PerceiveSnapshot,
) -> Result<AppendReceipt> {
    if ctl_path.is_empty() {
        return Err(PerceiveError::Empty { field: "ctl_path" });
    }
    if actor.is_empty() {
        return Err(PerceiveError::Empty { field: "actor" });
    }

    let payload = serde_json::to_string(snapshot)?;

    let out = runner.run(&[
        ctl_path.to_string(),
        "append".to_string(),
        "--kind".to_string(),
        EVENT_KIND.to_string(),
        "--actor".to_string(),
        actor.to_string(),
        "--payload".to_string(),
        payload,
    ])?;

    if out.status != 0 {
        return Err(PerceiveError::Subprocess {
            command: ctl_path.to_string(),
            detail: format!(
                "exit {}: {}",
                out.status,
                out.stderr.trim()
            ),
        });
    }

    parse_receipt(&out.stdout)
}

fn parse_receipt(raw: &str) -> Result<AppendReceipt> {
    let v: serde_json::Value =
        serde_json::from_str(raw.trim()).map_err(|err| PerceiveError::Parse {
            source: "orch-kernelctl-append",
            detail: err.to_string(),
        })?;

    let seq = v
        .get("seq")
        .and_then(serde_json::Value::as_i64)
        .ok_or(PerceiveError::Parse {
            source: "orch-kernelctl-append:seq",
            detail: "missing or not an integer".to_string(),
        })?;

    let event_id = v
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or(PerceiveError::Parse {
            source: "orch-kernelctl-append:event_id",
            detail: "missing or not a string".to_string(),
        })?;

    Ok(AppendReceipt { seq, event_id })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};
    use crate::manifest::{CatalogObservation, Source, TimestampMs, SCHEMA};

    // -----------------------------------------------------------------------
    // Fake runner
    // -----------------------------------------------------------------------

    struct FakeKernelCtl {
        status: i32,
        stdout: String,
        /// Captures the argv of the last call
        last_argv: std::cell::RefCell<Vec<String>>,
    }

    impl FakeKernelCtl {
        fn success(receipt_json: &str) -> Self {
            Self {
                status: 0,
                stdout: receipt_json.to_string(),
                last_argv: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn failure(status: i32, stderr: &str) -> Self {
            Self {
                status,
                stdout: stderr.to_string(),
                last_argv: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn last_argv(&self) -> Vec<String> {
            self.last_argv.borrow().clone()
        }
    }

    impl CommandRunner for FakeKernelCtl {
        fn run(&self, argv: &[String]) -> crate::Result<CommandOutput> {
            *self.last_argv.borrow_mut() = argv.to_vec();
            let (status, stdout, stderr) = if self.status == 0 {
                (0, self.stdout.clone(), String::new())
            } else {
                (self.status, String::new(), self.stdout.clone())
            };
            Ok(CommandOutput {
                status,
                stdout,
                stderr,
            })
        }
    }

    fn minimal_snapshot() -> PerceiveSnapshot {
        PerceiveSnapshot {
            schema: SCHEMA.to_string(),
            captured_at_ms: TimestampMs::from_millis(1_700_000_000_000),
            source: Source::HostHelper,
            panes: Vec::new(),
            sessions: Vec::new(),
            engines: Vec::new(),
            catalog: CatalogObservation::default(),
            leases: Vec::new(),
            fibers: Vec::new(),
        }
    }

    fn receipt_json(seq: i64, event_id: &str) -> String {
        format!(r#"{{"seq":{seq},"event_id":"{event_id}"}}"#)
    }

    // -----------------------------------------------------------------------
    // Empty-field guard tests
    // -----------------------------------------------------------------------

    #[test]
    fn empty_ctl_path_returns_empty_error() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "evt-1"));
        let err = append_snapshot(&runner, "", "host-helper", &minimal_snapshot()).unwrap_err();
        assert!(matches!(err, PerceiveError::Empty { field: "ctl_path" }));
    }

    #[test]
    fn empty_actor_returns_empty_error() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "evt-1"));
        let err =
            append_snapshot(&runner, "/usr/bin/orch-kernelctl", "", &minimal_snapshot()).unwrap_err();
        assert!(matches!(err, PerceiveError::Empty { field: "actor" }));
    }

    // -----------------------------------------------------------------------
    // Happy-path tests
    // -----------------------------------------------------------------------

    #[test]
    fn happy_path_returns_receipt() {
        let runner = FakeKernelCtl::success(&receipt_json(42, "evt-abc"));
        let receipt = append_snapshot(
            &runner,
            "/usr/bin/orch-kernelctl",
            "host-helper",
            &minimal_snapshot(),
        )
        .unwrap();
        assert_eq!(receipt.seq, 42);
        assert_eq!(receipt.event_id, "evt-abc");
    }

    #[test]
    fn argv_contains_append_verb() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "e"));
        append_snapshot(&runner, "/ctl", "actor", &minimal_snapshot()).unwrap();
        let argv = runner.last_argv();
        assert_eq!(argv[1], "append");
    }

    #[test]
    fn argv_contains_kind_flag() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "e"));
        append_snapshot(&runner, "/ctl", "actor", &minimal_snapshot()).unwrap();
        let argv = runner.last_argv();
        let kind_pos = argv.iter().position(|a| a == "--kind").unwrap();
        assert_eq!(argv[kind_pos + 1], EVENT_KIND);
    }

    #[test]
    fn argv_contains_actor_flag() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "e"));
        append_snapshot(&runner, "/ctl", "my-actor", &minimal_snapshot()).unwrap();
        let argv = runner.last_argv();
        let pos = argv.iter().position(|a| a == "--actor").unwrap();
        assert_eq!(argv[pos + 1], "my-actor");
    }

    #[test]
    fn argv_contains_payload_flag() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "e"));
        append_snapshot(&runner, "/ctl", "a", &minimal_snapshot()).unwrap();
        let argv = runner.last_argv();
        assert!(argv.contains(&"--payload".to_string()));
    }

    #[test]
    fn payload_is_valid_json_in_argv() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "e"));
        append_snapshot(&runner, "/ctl", "a", &minimal_snapshot()).unwrap();
        let argv = runner.last_argv();
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        let json_str = &argv[pos + 1];
        let v: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["schema"].as_str().unwrap(), SCHEMA);
    }

    #[test]
    fn argv_never_uses_submit_verb() {
        let runner = FakeKernelCtl::success(&receipt_json(1, "e"));
        append_snapshot(&runner, "/ctl", "a", &minimal_snapshot()).unwrap();
        let argv = runner.last_argv();
        assert!(!argv.contains(&"submit".to_string()), "must not use submit — observation event only");
    }

    // -----------------------------------------------------------------------
    // Failure tests
    // -----------------------------------------------------------------------

    #[test]
    fn nonzero_exit_returns_subprocess_error() {
        let runner = FakeKernelCtl::failure(1, "chain error");
        let err =
            append_snapshot(&runner, "/ctl", "a", &minimal_snapshot()).unwrap_err();
        assert!(matches!(err, PerceiveError::Subprocess { .. }));
    }

    #[test]
    fn malformed_receipt_returns_parse_error() {
        let runner = FakeKernelCtl::success("not json");
        let err =
            append_snapshot(&runner, "/ctl", "a", &minimal_snapshot()).unwrap_err();
        assert!(matches!(err, PerceiveError::Parse { .. }));
    }

    #[test]
    fn receipt_missing_seq_returns_parse_error() {
        let runner = FakeKernelCtl::success(r#"{"event_id":"evt-1"}"#);
        let err =
            append_snapshot(&runner, "/ctl", "a", &minimal_snapshot()).unwrap_err();
        assert!(matches!(err, PerceiveError::Parse { .. }));
    }

    #[test]
    fn receipt_missing_event_id_returns_parse_error() {
        let runner = FakeKernelCtl::success(r#"{"seq":1}"#);
        let err =
            append_snapshot(&runner, "/ctl", "a", &minimal_snapshot()).unwrap_err();
        assert!(matches!(err, PerceiveError::Parse { .. }));
    }

    // -----------------------------------------------------------------------
    // parse_receipt unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_receipt_happy_path() {
        let r = parse_receipt(r#"{"seq":99,"event_id":"abc-123"}"#).unwrap();
        assert_eq!(r.seq, 99);
        assert_eq!(r.event_id, "abc-123");
    }

    #[test]
    fn parse_receipt_handles_whitespace() {
        let r = parse_receipt("  { \"seq\": 1, \"event_id\": \"x\" }  ").unwrap();
        assert_eq!(r.seq, 1);
    }

    #[test]
    fn parse_receipt_ignores_extra_fields() {
        let r = parse_receipt(r#"{"seq":3,"event_id":"e","extra":"ignored"}"#).unwrap();
        assert_eq!(r.seq, 3);
    }

    #[test]
    fn event_kind_constant_is_correct() {
        assert_eq!(EVENT_KIND, "perceive.snapshot");
    }
}
