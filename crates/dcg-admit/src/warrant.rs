//! Warrant / policy routing — the actuation verb.
//!
//! Routes a delegated actuation through the existing orchestrator-kernel warrant
//! organ via `orch-kernelctl submit`. This is the consequential verb (cf. the
//! warrant-free `append` used for perception). Per build-plan §8 A2 a policy
//! denial surfaces as a **non-zero exit / `Err`**, never a `SubmitVerdict::Nack`
//! (the only `Nack` is an idempotency conflict). This module therefore treats
//! **Err-as-denial**: a non-zero `submit` exit maps to [`DcgError::Denied`]; it
//! does not parse for a `Nack` verdict. There is no `consent` concept — it is a
//! policy/warrant gate. The policy itself is NOT rebuilt here
//! (`config/zellij-orchestrator-kernel-warrants.v2.json` is authoritative).

use serde::Serialize;

use crate::error::DcgError;
use crate::exec::CommandRunner;
use crate::Result;

/// A delegated actuation to be routed through the warrant organ.
///
/// `payload` is a [`serde_json::Value`], so this type is `PartialEq` but not
/// `Eq` (JSON numbers are floats).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Actuation {
    /// The warrant kind (for example `recipe.execution`).
    pub kind: String,
    /// Trace id correlating this actuation across the spine event trail.
    pub trace_id: String,
    /// The actor on whose behalf the actuation is submitted (for example
    /// `cortex`).
    pub actor: String,
    /// The opaque actuation payload.
    pub payload: serde_json::Value,
}

/// Receipt returned when the warrant organ admits an actuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarrantReceipt {
    /// Monotonic sequence of the admitted warrant, read from a spine snapshot
    /// taken immediately after the durable submit.
    pub seq: i64,
    /// Durable event id of the admitted warrant.
    pub event_id: String,
}

/// Routes `actuation` through `orch-kernelctl submit` at `ctl_path`.
///
/// On a successful (durable) admission this returns a [`WarrantReceipt`]. A
/// policy refusal (non-zero exit) MUST be surfaced as [`DcgError::Denied`]
/// carrying the refusal trail — never swallowed and never mistaken for a `Nack`
/// verdict (build-plan §8 A2).
///
/// Implementation makes two runner calls on the success path: `submit` then
/// `snapshot` (to resolve the admitted event's monotonic seq). On denial, only
/// the `submit` call is made.
///
/// # Errors
/// Returns [`DcgError::Denied`] when the policy refuses the actuation
/// (non-zero exit from `orch-kernelctl submit`).
/// Returns [`DcgError::Subprocess`] if `orch-kernelctl` cannot be run.
/// Returns [`DcgError::Parse`] if the success receipt or subsequent snapshot
/// cannot be decoded.
pub fn submit_actuation(
    runner: &dyn CommandRunner,
    ctl_path: &str,
    actuation: &Actuation,
) -> Result<WarrantReceipt> {
    // Build the SubmitRequest JSON.  Field names match the sidecar's
    // SubmitRequest struct: schema, trace_id, idempotency_key, kind, operator,
    // payload.  `actor` maps to `operator`; idempotency key reuses trace_id.
    let submit_req = serde_json::json!({
        "schema": "task.submit.v1",
        "trace_id": actuation.trace_id,
        "idempotency_key": actuation.trace_id,
        "kind": actuation.kind,
        "operator": actuation.actor,
        "payload": actuation.payload
    });
    let submit_json = serde_json::to_string(&submit_req)?;

    let submit_argv = [
        ctl_path.to_string(),
        "submit".to_string(),
        "--json".to_string(),
        submit_json,
    ];

    let submit_out = runner.run(&submit_argv)?;

    // Non-zero exit = Err-as-denial (build-plan §8 A2). Do NOT parse for Nack.
    if !submit_out.is_success() {
        let trail = build_denial_reason(submit_out.status, &submit_out.stderr);
        return Err(DcgError::Denied { reason: trail });
    }

    // Parse the success response to extract event_id.
    let resp: serde_json::Value =
        serde_json::from_str(submit_out.stdout.trim()).map_err(|e| DcgError::Parse {
            source: "submit-response",
            detail: e.to_string(),
        })?;

    let event_id = resp["event_id"]
        .as_str()
        .ok_or_else(|| DcgError::Parse {
            source: "submit-response",
            detail: "missing or null `event_id` field".to_string(),
        })?
        .to_string();

    // Resolve the admitted event's spine seq via snapshot (submit response
    // does not include seq directly).
    let snap_argv = [
        ctl_path.to_string(),
        "snapshot".to_string(),
        "--json".to_string(),
    ];
    let snap_out = runner.run(&snap_argv)?;

    if !snap_out.is_success() {
        return Err(DcgError::Subprocess {
            command: format!("{ctl_path} snapshot"),
            detail: format!("exited with status {}", snap_out.status),
        });
    }

    let snap: serde_json::Value =
        serde_json::from_str(snap_out.stdout.trim()).map_err(|e| DcgError::Parse {
            source: "snapshot-response",
            detail: e.to_string(),
        })?;

    let seq = snap["last_seq"]
        .as_i64()
        .ok_or_else(|| DcgError::Parse {
            source: "snapshot-response",
            detail: "missing or non-integer `last_seq` field".to_string(),
        })?;

    Ok(WarrantReceipt { seq, event_id })
}

/// Constructs a human-readable denial reason from the process exit status and
/// stderr. Used internally only.
fn build_denial_reason(status: i32, stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        format!("orch-kernelctl submit exited with status {status}")
    } else {
        format!("orch-kernelctl submit exited {status}: {trimmed}")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};

    // -----------------------------------------------------------------------
    // Test infrastructure
    // -----------------------------------------------------------------------

    /// Plays back canned responses in order and records argv for each call.
    struct FakeRunner {
        responses: Vec<CommandOutput>,
        calls: std::cell::RefCell<Vec<Vec<String>>>,
        cursor: std::cell::RefCell<usize>,
    }

    impl FakeRunner {
        fn new(responses: Vec<CommandOutput>) -> Self {
            Self {
                responses,
                calls: std::cell::RefCell::new(Vec::new()),
                cursor: std::cell::RefCell::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }

        fn argv_at(&self, index: usize) -> Vec<String> {
            self.calls.borrow().get(index).cloned().unwrap_or_default()
        }

        #[allow(dead_code)]
        fn last_argv(&self) -> Vec<String> {
            self.calls.borrow().last().cloned().unwrap_or_default()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, argv: &[String]) -> crate::Result<CommandOutput> {
            self.calls.borrow_mut().push(argv.to_vec());
            let mut idx = self.cursor.borrow_mut();
            let out = self
                .responses
                .get(*idx)
                .cloned()
                .ok_or_else(|| DcgError::Subprocess {
                    command: "fake".to_string(),
                    detail: format!("no response at index {idx}"),
                })?;
            *idx += 1;
            Ok(out)
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn fail(status: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    fn sample() -> Actuation {
        Actuation {
            kind: "recipe.execution".to_string(),
            trace_id: "t-probe".to_string(),
            actor: "cortex".to_string(),
            payload: serde_json::json!({}),
        }
    }

    fn submit_ok_json() -> &'static str {
        r#"{"schema":"task.submit.v1","verdict":"ACK_DURABLE","trace_id":"t-probe","event_id":"evt-42","event_hash":"abc","integration_state":"admitted","idempotency":"NEW","reason":"","request_hash":"xyz"}"#
    }

    fn snapshot_json(last_seq: i64) -> String {
        format!(r#"{{"last_seq":{last_seq},"verify_chain_ok":true}}"#)
    }

    // -----------------------------------------------------------------------
    // Happy path
    // -----------------------------------------------------------------------

    #[test]
    fn happy_path_returns_receipt_with_correct_seq() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(42))]);
        let r = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r.seq, 42);
    }

    #[test]
    fn happy_path_returns_receipt_with_correct_event_id() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        let r = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r.event_id, "evt-42");
    }

    #[test]
    fn happy_path_makes_exactly_two_runner_calls() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(runner.call_count(), 2);
    }

    #[test]
    fn high_seq_value_preserved() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(999_999))]);
        let r = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r.seq, 999_999);
    }

    #[test]
    fn seq_zero_is_valid() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(0))]);
        let r = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r.seq, 0);
    }

    #[test]
    fn stdout_with_trailing_whitespace_parsed_correctly() {
        let padded = format!("{}\n", submit_ok_json());
        let runner = FakeRunner::new(vec![ok(&padded), ok(&snapshot_json(7))]);
        let r = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r.event_id, "evt-42");
    }

    // -----------------------------------------------------------------------
    // Submit argv shape
    // -----------------------------------------------------------------------

    #[test]
    fn submit_argv_first_element_is_ctl_path() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/usr/bin/ctl", &sample()).unwrap();
        assert_eq!(runner.argv_at(0).first().map(String::as_str), Some("/usr/bin/ctl"));
    }

    #[test]
    fn submit_argv_second_element_is_submit_verb() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(runner.argv_at(0).get(1).map(String::as_str), Some("submit"));
    }

    #[test]
    fn submit_argv_third_element_is_json_flag() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(runner.argv_at(0).get(2).map(String::as_str), Some("--json"));
    }

    #[test]
    fn submit_json_contains_trace_id() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["trace_id"], "t-probe");
    }

    #[test]
    fn submit_json_contains_kind() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["kind"], "recipe.execution");
    }

    #[test]
    fn submit_json_maps_actor_to_operator() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["operator"], "cortex");
    }

    #[test]
    fn submit_json_schema_is_task_submit_v1() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["schema"], "task.submit.v1");
    }

    #[test]
    fn submit_json_idempotency_key_equals_trace_id() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["idempotency_key"], v["trace_id"]);
    }

    #[test]
    fn submit_json_payload_included() {
        let mut act = sample();
        act.payload = serde_json::json!({"key": "val"});
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["payload"]["key"], "val");
    }

    #[test]
    fn empty_payload_serializes_as_empty_object() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["payload"], serde_json::json!({}));
    }

    // -----------------------------------------------------------------------
    // Snapshot argv shape
    // -----------------------------------------------------------------------

    #[test]
    fn snapshot_argv_first_element_is_ctl_path() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/my/ctl", &sample()).unwrap();
        // Second call is snapshot
        assert_eq!(runner.argv_at(1).first().map(String::as_str), Some("/my/ctl"));
    }

    #[test]
    fn snapshot_argv_includes_snapshot_verb() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let snap_argv = runner.argv_at(1);
        assert!(snap_argv.contains(&"snapshot".to_string()));
    }

    #[test]
    fn snapshot_argv_includes_json_flag() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let snap_argv = runner.argv_at(1);
        assert!(snap_argv.contains(&"--json".to_string()));
    }

    // -----------------------------------------------------------------------
    // Denial — Err-as-denial (§8 A2)
    // -----------------------------------------------------------------------

    #[test]
    fn nonzero_exit_returns_denied_error() {
        let runner = FakeRunner::new(vec![fail(1, "RECIPE_EXECUTION default DENY")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
    }

    #[test]
    fn denied_reason_includes_stderr_content() {
        let runner = FakeRunner::new(vec![fail(1, "policy: RECIPE_EXECUTION denied")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        let DcgError::Denied { reason } = err else { panic!("expected Denied") };
        assert!(reason.contains("policy: RECIPE_EXECUTION denied"));
    }

    #[test]
    fn denied_reason_includes_exit_status_when_stderr_empty() {
        let runner = FakeRunner::new(vec![fail(42, "")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        let DcgError::Denied { reason } = err else { panic!("expected Denied") };
        assert!(reason.contains("42"));
    }

    #[test]
    fn denied_with_status_127_maps_to_denied() {
        let runner = FakeRunner::new(vec![fail(127, "not found")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
    }

    #[test]
    fn snapshot_not_called_on_denial() {
        let runner = FakeRunner::new(vec![fail(1, "denied")]);
        let _ = submit_actuation(&runner, "/ctl", &sample());
        assert_eq!(runner.call_count(), 1, "snapshot must not be called on denial");
    }

    #[test]
    fn denied_captures_multiline_stderr() {
        let runner = FakeRunner::new(vec![fail(1, "line1\nline2\nline3")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        let DcgError::Denied { reason } = err else { panic!("expected Denied") };
        assert!(reason.contains("line1"));
    }

    // -----------------------------------------------------------------------
    // Parse errors
    // -----------------------------------------------------------------------

    #[test]
    fn parse_error_on_non_json_submit_stdout() {
        let runner = FakeRunner::new(vec![ok("not json")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "submit-response", .. }));
    }

    #[test]
    fn parse_error_when_event_id_missing_from_response() {
        let runner = FakeRunner::new(vec![ok(r#"{"verdict":"ACK_DURABLE","trace_id":"t"}"#)]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "submit-response", .. }));
    }

    #[test]
    fn parse_error_when_event_id_is_null() {
        let runner = FakeRunner::new(vec![ok(r#"{"event_id":null,"verdict":"ACK_DURABLE"}"#)]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "submit-response", .. }));
    }

    #[test]
    fn snapshot_subprocess_error_when_nonzero_exit() {
        let runner =
            FakeRunner::new(vec![ok(submit_ok_json()), fail(1, "snapshot failed")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Subprocess { .. }));
    }

    #[test]
    fn snapshot_parse_error_on_bad_json() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok("bad json")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "snapshot-response", .. }));
    }

    #[test]
    fn snapshot_parse_error_when_last_seq_missing() {
        let runner = FakeRunner::new(vec![
            ok(submit_ok_json()),
            ok(r#"{"verify_chain_ok":true}"#),
        ]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(
            matches!(err, DcgError::Parse { source: "snapshot-response", .. }),
            "expected snapshot parse error, got: {err}"
        );
    }

    #[test]
    fn snapshot_parse_error_when_last_seq_is_string() {
        let runner = FakeRunner::new(vec![
            ok(submit_ok_json()),
            ok(r#"{"last_seq":"not-a-number"}"#),
        ]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "snapshot-response", .. }));
    }

    // -----------------------------------------------------------------------
    // Struct coverage — non-reflexive, non-serialization-only checks
    //
    // Removed: actuation_fields_accessible (reflexive field read = const-equality),
    // actuation_serializes_{kind,trace_id,actor} (Actuation::Serialize is never
    // exercised in the production path — submit_actuation builds serde_json::json!
    // manually from individual fields), actuation_partial_eq_same_is_true
    // (reflexive equality), actuation_clone_is_independent and
    // actuation_debug_contains_trace_id (test derive macros, not production flow),
    // warrant_receipt_{fields_accessible,eq,clone_is_equal,debug_contains_seq}
    // (same: reflexive / derive-macro tests, not production behaviour).
    // -----------------------------------------------------------------------

    #[test]
    fn actuation_partial_eq_different_kind_is_false() {
        let mut b = sample();
        b.kind = "other".to_string();
        assert_ne!(sample(), b);
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn actuation_with_complex_payload_round_trips_through_submit_json() {
        let mut act = sample();
        act.payload = serde_json::json!({"nested": {"a": 1, "b": [1, 2, 3]}});
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(5))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["payload"]["nested"]["b"][2], 3);
    }

    #[test]
    fn very_long_trace_id_propagates_to_submit_json() {
        let mut act = sample();
        act.trace_id = "a".repeat(1024);
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["trace_id"].as_str().unwrap().len(), 1024);
    }

    #[test]
    fn snapshot_subprocess_error_includes_ctl_path_and_status() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), fail(99, "")]);
        let err = submit_actuation(&runner, "/my/ctl", &sample()).unwrap_err();
        let DcgError::Subprocess { command, detail } = err else {
            panic!("expected Subprocess")
        };
        assert!(command.contains("/my/ctl"));
        assert!(detail.contains("99"));
    }

    #[test]
    fn two_independent_calls_are_independent() {
        let runner = FakeRunner::new(vec![
            ok(submit_ok_json()),
            ok(&snapshot_json(10)),
            ok(submit_ok_json()),
            ok(&snapshot_json(11)),
        ]);
        let r1 = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let r2 = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r1.seq, 10);
        assert_eq!(r2.seq, 11);
    }

    #[test]
    fn build_denial_reason_with_nonempty_stderr() {
        let reason = build_denial_reason(1, " denied by policy ");
        assert!(reason.contains("denied by policy"));
        assert!(reason.contains('1'));
    }

    #[test]
    fn build_denial_reason_with_empty_stderr_uses_status() {
        let reason = build_denial_reason(127, "");
        assert!(reason.contains("127"));
        assert!(!reason.contains(':'));
    }

    // -----------------------------------------------------------------------
    // Additional behavioral tests — submit_actuation request/response
    // -----------------------------------------------------------------------

    /// Empty `actor` must map to an empty `"operator"` field, not be omitted.
    #[test]
    fn actor_empty_string_maps_to_empty_operator() {
        let mut act = sample();
        act.actor = String::new();
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["operator"], "");
    }

    /// A `null` JSON payload must arrive as literal `null` in the submit request.
    #[test]
    fn submit_json_null_payload_is_literal_null() {
        let mut act = sample();
        act.payload = serde_json::Value::Null;
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert!(v["payload"].is_null(), "null payload must serialise as null, not be dropped");
    }

    /// `idempotency_key` must equal `trace_id` even when `trace_id` contains
    /// colons and hyphens (fence-trace format).
    #[test]
    fn idempotency_key_matches_trace_id_with_special_chars() {
        let mut act = sample();
        act.trace_id = "dcg:fence:my-resource:0001".to_string();
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(
            v["idempotency_key"], v["trace_id"],
            "idempotency_key must equal trace_id regardless of special chars"
        );
        assert_eq!(v["trace_id"], "dcg:fence:my-resource:0001");
    }

    /// The submit request must have exactly six fields: `schema`, `trace_id`,
    /// `idempotency_key`, `kind`, `operator`, `payload` — no more, no fewer.
    #[test]
    fn submit_request_has_exactly_six_keys() {
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(
            obj.len(),
            6,
            "submit request must have exactly 6 fields (schema, trace_id, \
             idempotency_key, kind, operator, payload), got: {}",
            obj.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }

    /// Whitespace surrounding `stderr` content is trimmed before embedding in
    /// the denial reason (prevents double-spaces in the formatted message).
    #[test]
    fn denial_reason_trims_stderr_whitespace() {
        let runner = FakeRunner::new(vec![fail(1, "  policy: denied  ")]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        let DcgError::Denied { reason } = err else { panic!("expected Denied") };
        assert!(reason.contains("policy: denied"), "denial reason must contain trimmed stderr");
        assert!(
            !reason.contains("  policy"),
            "denial reason must not contain leading whitespace from stderr"
        );
    }

    /// Extra unknown fields in the submit response must not prevent `event_id`
    /// extraction — the organ is forward-compatible with schema additions.
    #[test]
    fn submit_response_with_extra_unknown_fields_is_accepted() {
        let resp = r#"{"event_id":"evt-99","verdict":"ACK_DURABLE","extra_future_field":"ignored","another":42}"#;
        let runner = FakeRunner::new(vec![ok(resp), ok(&snapshot_json(5))]);
        let r = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r.event_id, "evt-99");
        assert_eq!(r.seq, 5);
    }

    /// `null` for `last_seq` in the snapshot response is not a valid integer
    /// and must produce a `Parse` error, not silently default to 0.
    #[test]
    fn snapshot_parse_error_when_last_seq_is_null() {
        let runner = FakeRunner::new(vec![
            ok(submit_ok_json()),
            ok(r#"{"last_seq":null,"verify_chain_ok":true}"#),
        ]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(
            matches!(err, DcgError::Parse { source: "snapshot-response", .. }),
            "null last_seq must produce a snapshot-response Parse error, got: {err}"
        );
    }

    /// A floating-point `last_seq` (e.g. `42.5`) is not a valid i64 and must
    /// produce a `Parse` error — the seq field is always monotonic integer.
    #[test]
    fn snapshot_parse_error_when_last_seq_is_float() {
        let runner = FakeRunner::new(vec![
            ok(submit_ok_json()),
            ok(r#"{"last_seq":42.5}"#),
        ]);
        let err = submit_actuation(&runner, "/ctl", &sample()).unwrap_err();
        assert!(
            matches!(err, DcgError::Parse { source: "snapshot-response", .. }),
            "float last_seq must produce a snapshot-response Parse error"
        );
    }

    /// Two successive actuations with distinct `event_id` responses must each
    /// carry only their own `event_id` and `seq` in the returned receipt.
    #[test]
    fn two_actuations_with_different_event_ids_produce_independent_receipts() {
        let resp_a = r#"{"event_id":"evt-A","verdict":"ACK_DURABLE"}"#;
        let resp_b = r#"{"event_id":"evt-B","verdict":"ACK_DURABLE"}"#;
        let runner = FakeRunner::new(vec![
            ok(resp_a),
            ok(&snapshot_json(100)),
            ok(resp_b),
            ok(&snapshot_json(101)),
        ]);
        let r1 = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        let r2 = submit_actuation(&runner, "/ctl", &sample()).unwrap();
        assert_eq!(r1.event_id, "evt-A");
        assert_eq!(r2.event_id, "evt-B");
        assert_ne!(r1.event_id, r2.event_id, "receipts must carry independent event_ids");
        assert_eq!(r1.seq, 100);
        assert_eq!(r2.seq, 101);
    }

    /// A `kind` value with version-suffixed dots propagates verbatim to the
    /// `"kind"` field of the submit request with no truncation or modification.
    #[test]
    fn kind_with_version_suffix_propagates_verbatim() {
        let mut act = sample();
        act.kind = "recipe.execution.v2.3".to_string();
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["kind"], "recipe.execution.v2.3");
    }

    /// An `actor` containing hyphens (typical fiber names like `dcg-fiber-7`)
    /// must reach the `"operator"` field exactly as given.
    #[test]
    fn actor_with_hyphen_maps_to_operator_verbatim() {
        let mut act = sample();
        act.actor = "dcg-fiber-7".to_string();
        let runner = FakeRunner::new(vec![ok(submit_ok_json()), ok(&snapshot_json(1))]);
        submit_actuation(&runner, "/ctl", &act).unwrap();
        let json_arg = &runner.argv_at(0)[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["operator"], "dcg-fiber-7");
    }
}
