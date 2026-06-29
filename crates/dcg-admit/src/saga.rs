//! Saga compensation for multi-step delegated writes.
//!
//! When an admitted write is composed of several steps and a later step fails,
//! the earlier effects must be neutralised. Compensation is **append-only**: a
//! compensating event is appended to the spine (via `orch-kernelctl append`) to
//! record the reversal. Chain rows are NEVER deleted or rewritten — the
//! append-only invariant must hold so `verify-chain` stays green (build-plan
//! §3 T3, §0).

use serde::Serialize;

use crate::error::DcgError;
use crate::exec::CommandRunner;
use crate::Result;

/// A single compensating action to append when reversing a partial write.
///
/// `payload` is a [`serde_json::Value`], so this type is `PartialEq` but not
/// `Eq` (JSON numbers are floats).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompensationStep {
    /// The event kind of the compensating record (for example
    /// `dcg.compensate`).
    pub kind: String,
    /// Trace id linking the compensation to the failed forward write.
    pub trace_id: String,
    /// The actor recording the compensation.
    pub actor: String,
    /// The compensation payload (what is being reversed and why).
    pub payload: serde_json::Value,
}

/// Appends `steps` as compensating events to the spine at `ctl_path`, in order.
///
/// Each step is appended via `orch-kernelctl append --kind <kind> --trace-id
/// <trace_id> --actor <actor> --payload <json>`. This uses the append-only
/// verb and NEVER issues `delete`, `submit`, or any command that could mutate
/// or remove existing chain rows.
///
/// Processing stops on the first failure: the error is surfaced to the caller
/// so they can escalate. No failure is silently swallowed.
///
/// # Errors
/// Returns [`DcgError::Subprocess`] if an `orch-kernelctl append` command
/// exits non-zero or cannot be run.
/// Returns [`DcgError::Parse`] if the serialised payload cannot be encoded as
/// JSON.
/// Returns [`DcgError::Json`] if a payload value fails serde serialisation.
pub fn compensate(
    runner: &dyn CommandRunner,
    ctl_path: &str,
    steps: &[CompensationStep],
) -> Result<()> {
    for step in steps {
        let payload_json = serde_json::to_string(&step.payload)?;

        let argv = [
            ctl_path.to_string(),
            "append".to_string(),
            "--kind".to_string(),
            step.kind.clone(),
            "--trace-id".to_string(),
            step.trace_id.clone(),
            "--actor".to_string(),
            step.actor.clone(),
            "--payload".to_string(),
            payload_json,
        ];

        let out = runner.run(&argv)?;

        if !out.is_success() {
            let detail = if out.stderr.trim().is_empty() {
                format!("exit {}", out.status)
            } else {
                format!("exit {}: {}", out.status, out.stderr.trim())
            };
            return Err(DcgError::Subprocess {
                command: format!("{ctl_path} append"),
                detail,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};

    // -----------------------------------------------------------------------
    // Test infrastructure
    // -----------------------------------------------------------------------

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

        fn all_argv_flat(&self) -> Vec<String> {
            self.calls.borrow().iter().flatten().cloned().collect()
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

    fn append_ok_json() -> &'static str {
        r#"{"seq":1,"event_id":"evt-1","trace_id":"t-1","kind":"dcg.compensate","actor":"dcg-admit","payload_json":"{}","hash":"abc"}"#
    }

    fn step() -> CompensationStep {
        CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t-1".to_string(),
            actor: "dcg-admit".to_string(),
            payload: serde_json::json!({"reverses": "step-1"}),
        }
    }

    fn step2() -> CompensationStep {
        CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t-2".to_string(),
            actor: "dcg-admit".to_string(),
            payload: serde_json::json!({"reverses": "step-2"}),
        }
    }

    // -----------------------------------------------------------------------
    // Basic behaviour
    // -----------------------------------------------------------------------

    #[test]
    fn empty_steps_is_ok_and_makes_no_calls() {
        let runner = FakeRunner::new(vec![]);
        compensate(&runner, "/ctl", &[]).unwrap();
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn single_step_appended_ok() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        assert_eq!(runner.call_count(), 1);
    }

    #[test]
    fn two_steps_make_two_runner_calls() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step(), step2()]).unwrap();
        assert_eq!(runner.call_count(), 2);
    }

    #[test]
    fn three_steps_all_succeed() {
        let runner =
            FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json()), ok(append_ok_json())]);
        let steps = [step(), step2(), step()];
        compensate(&runner, "/ctl", &steps).unwrap();
        assert_eq!(runner.call_count(), 3);
    }

    #[test]
    fn steps_processed_in_order_trace_ids_match() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step(), step2()]).unwrap();
        let first_argv = runner.argv_at(0);
        let second_argv = runner.argv_at(1);
        let first_trace_pos = first_argv.iter().position(|a| a == "--trace-id").unwrap();
        let second_trace_pos = second_argv.iter().position(|a| a == "--trace-id").unwrap();
        assert_eq!(first_argv[first_trace_pos + 1], "t-1");
        assert_eq!(second_argv[second_trace_pos + 1], "t-2");
    }

    // -----------------------------------------------------------------------
    // Argv shape
    // -----------------------------------------------------------------------

    #[test]
    fn argv_first_element_is_ctl_path() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/my/ctl", &[step()]).unwrap();
        assert_eq!(runner.argv_at(0).first().map(String::as_str), Some("/my/ctl"));
    }

    #[test]
    fn argv_second_element_is_append_verb() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        assert_eq!(runner.argv_at(0).get(1).map(String::as_str), Some("append"));
    }

    #[test]
    fn argv_includes_kind_flag_and_value() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--kind").expect("--kind missing");
        assert_eq!(argv[pos + 1], "dcg.compensate");
    }

    #[test]
    fn argv_includes_trace_id_flag_and_value() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--trace-id").expect("--trace-id missing");
        assert_eq!(argv[pos + 1], "t-1");
    }

    #[test]
    fn argv_includes_actor_flag_and_value() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--actor").expect("--actor missing");
        assert_eq!(argv[pos + 1], "dcg-admit");
    }

    #[test]
    fn argv_includes_payload_flag() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        let argv = runner.argv_at(0);
        assert!(argv.contains(&"--payload".to_string()));
    }

    #[test]
    fn payload_arg_is_valid_json() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        let payload_str = &argv[pos + 1];
        serde_json::from_str::<serde_json::Value>(payload_str)
            .expect("payload arg must be valid JSON");
    }

    #[test]
    fn payload_json_contains_reverses_key() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        let payload: serde_json::Value = serde_json::from_str(&argv[pos + 1]).unwrap();
        assert_eq!(payload["reverses"], "step-1");
    }

    // -----------------------------------------------------------------------
    // Append-only invariant: never issues delete or submit
    // -----------------------------------------------------------------------

    #[test]
    fn compensate_never_issues_delete() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step(), step2()]).unwrap();
        let all = runner.all_argv_flat();
        assert!(
            !all.iter().any(|a| a == "delete"),
            "compensate must never issue `delete`"
        );
    }

    #[test]
    fn compensate_never_issues_submit() {
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        let all = runner.all_argv_flat();
        assert!(
            !all.iter().any(|a| a == "submit"),
            "compensate must never issue `submit`"
        );
    }

    #[test]
    fn compensate_only_issues_append() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step(), step2()]).unwrap();
        for (i, _) in [step(), step2()].iter().enumerate() {
            assert_eq!(runner.argv_at(i).get(1).map(String::as_str), Some("append"));
        }
    }

    // -----------------------------------------------------------------------
    // Failure behaviour — fail fast, no swallow
    // -----------------------------------------------------------------------

    #[test]
    fn first_step_failure_returns_subprocess_error() {
        let runner = FakeRunner::new(vec![fail(1, "append rejected")]);
        let err = compensate(&runner, "/ctl", &[step()]).unwrap_err();
        assert!(matches!(err, DcgError::Subprocess { .. }));
    }

    #[test]
    fn first_step_failure_stops_processing_second_step() {
        let runner = FakeRunner::new(vec![fail(1, "fail"), ok(append_ok_json())]);
        let _ = compensate(&runner, "/ctl", &[step(), step2()]);
        assert_eq!(runner.call_count(), 1, "second step must not be attempted");
    }

    #[test]
    fn second_step_failure_after_first_success_returns_error() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), fail(2, "second fail")]);
        let err = compensate(&runner, "/ctl", &[step(), step2()]).unwrap_err();
        assert!(matches!(err, DcgError::Subprocess { .. }));
    }

    #[test]
    fn nonzero_exit_error_includes_exit_status() {
        let runner = FakeRunner::new(vec![fail(77, "")]);
        let err = compensate(&runner, "/ctl", &[step()]).unwrap_err();
        assert!(err.to_string().contains("77"));
    }

    #[test]
    fn nonzero_exit_error_includes_stderr() {
        let runner = FakeRunner::new(vec![fail(1, "chain locked")]);
        let err = compensate(&runner, "/ctl", &[step()]).unwrap_err();
        assert!(err.to_string().contains("chain locked"));
    }

    #[test]
    fn nonzero_exit_error_includes_ctl_path() {
        let runner = FakeRunner::new(vec![fail(1, "")]);
        let err = compensate(&runner, "/my/ctl", &[step()]).unwrap_err();
        assert!(err.to_string().contains("/my/ctl"));
    }

    #[test]
    fn each_step_generates_exactly_one_runner_call() {
        let n = 5_usize;
        let responses = vec![ok(append_ok_json()); n];
        let steps: Vec<CompensationStep> = (0..n)
            .map(|i| CompensationStep {
                kind: "dcg.compensate".to_string(),
                trace_id: format!("t-{i}"),
                actor: "dcg-admit".to_string(),
                payload: serde_json::json!({}),
            })
            .collect();
        let runner = FakeRunner::new(responses);
        compensate(&runner, "/ctl", &steps).unwrap();
        assert_eq!(runner.call_count(), n);
    }

    // -----------------------------------------------------------------------
    // Struct tests — non-reflexive, non-serialization-only checks
    //
    // Removed: compensation_step_fields_accessible (reflexive field read),
    // compensation_step_serializes_{kind,trace_id,payload,actor_field}
    // (CompensationStep::Serialize is never used in production — compensate()
    // calls serde_json::to_string(&step.payload) on the payload Value only,
    // never on the CompensationStep struct), compensation_step_partial_eq_same
    // (reflexive equality), compensation_step_{clone_is_independent,
    // debug_contains_kind} (test derive macros, not production behaviour).
    // -----------------------------------------------------------------------

    #[test]
    fn compensation_step_partial_eq_different_trace() {
        let mut b = step();
        b.trace_id = "other".to_string();
        assert_ne!(step(), b);
    }

    #[test]
    fn compensation_step_with_complex_payload() {
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t".to_string(),
            actor: "a".to_string(),
            payload: serde_json::json!({"arr": [1,2,3], "nested": {"x": true}}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        let v: serde_json::Value = serde_json::from_str(&argv[pos + 1]).unwrap();
        assert_eq!(v["nested"]["x"], true);
    }

    #[test]
    fn step_with_trace_id_containing_colons_propagates() {
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "dcg-fence:my.resource".to_string(),
            actor: "dcg-admit".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--trace-id").unwrap();
        assert_eq!(argv[pos + 1], "dcg-fence:my.resource");
    }

    #[test]
    fn empty_payload_serializes_as_empty_object() {
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t".to_string(),
            actor: "a".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        assert_eq!(argv[pos + 1], "{}");
    }

    #[test]
    fn nonzero_exit_with_empty_stderr_still_surfaces_error() {
        let runner = FakeRunner::new(vec![fail(1, "")]);
        assert!(compensate(&runner, "/ctl", &[step()]).is_err());
    }

    #[test]
    fn kind_from_step_verbatim_in_argv() {
        let s = CompensationStep {
            kind: "my.custom.compensate".to_string(),
            trace_id: "t".to_string(),
            actor: "a".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--kind").unwrap();
        assert_eq!(argv[pos + 1], "my.custom.compensate");
    }

    #[test]
    fn steps_not_mutated_by_compensate() {
        let steps = vec![step(), step2()];
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &steps).unwrap();
        // Verify the original steps are unchanged
        assert_eq!(steps[0].trace_id, "t-1");
        assert_eq!(steps[1].trace_id, "t-2");
    }

    #[test]
    fn compensate_with_null_payload_serializes_correctly() {
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t".to_string(),
            actor: "a".to_string(),
            payload: serde_json::Value::Null,
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        assert_eq!(argv[pos + 1], "null");
    }

    #[test]
    fn compensate_with_array_payload_serializes_correctly() {
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t".to_string(),
            actor: "a".to_string(),
            payload: serde_json::json!([1, 2, 3]),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        let v: serde_json::Value = serde_json::from_str(&argv[pos + 1]).unwrap();
        assert_eq!(v[1], 2);
    }

    #[test]
    fn second_step_argv_uses_its_own_kind() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step(), step2()]).unwrap();
        let argv = runner.argv_at(1);
        let pos = argv.iter().position(|a| a == "--kind").unwrap();
        // step2 also uses "dcg.compensate"
        assert_eq!(argv[pos + 1], "dcg.compensate");
    }

    #[test]
    fn ctl_path_is_verbatim_for_each_step() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/absolute/ctl", &[step(), step2()]).unwrap();
        assert_eq!(runner.argv_at(0)[0], "/absolute/ctl");
        assert_eq!(runner.argv_at(1)[0], "/absolute/ctl");
    }

    #[test]
    fn empty_actor_propagates_to_argv() {
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t".to_string(),
            actor: String::new(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--actor").unwrap();
        assert_eq!(argv[pos + 1], "");
    }

    #[test]
    fn step_with_boolean_true_payload() {
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t".to_string(),
            actor: "a".to_string(),
            payload: serde_json::Value::Bool(true),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--payload").unwrap();
        assert_eq!(argv[pos + 1], "true");
    }

    #[test]
    fn nonzero_exit_error_detail_includes_exit_code_when_stderr_empty() {
        let runner = FakeRunner::new(vec![fail(55, "")]);
        let err = compensate(&runner, "/ctl", &[step()]).unwrap_err();
        let DcgError::Subprocess { detail, .. } = err else { panic!("expected Subprocess") };
        assert!(detail.contains("55"));
    }

    #[test]
    fn single_step_with_all_fields_set_explicitly() {
        let s = CompensationStep {
            kind: "saga.rollback".to_string(),
            trace_id: "trace-abc".to_string(),
            actor: "workflow-fiber-7".to_string(),
            payload: serde_json::json!({"target": "pane-3", "action": "close"}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/usr/local/bin/orch-kernelctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let kp = argv.iter().position(|a| a == "--kind").unwrap();
        assert_eq!(argv[kp + 1], "saga.rollback");
        let ap = argv.iter().position(|a| a == "--actor").unwrap();
        assert_eq!(argv[ap + 1], "workflow-fiber-7");
    }

    #[test]
    fn compensate_result_is_unit_ok_on_success() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        let result = compensate(&runner, "/ctl", &[step(), step2()]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ());
    }

    #[test]
    fn last_step_failure_makes_correct_number_of_calls() {
        // 3 steps; third fails
        let runner = FakeRunner::new(vec![
            ok(append_ok_json()),
            ok(append_ok_json()),
            fail(1, "third step fails"),
        ]);
        let steps = vec![step(), step2(), step()];
        let _ = compensate(&runner, "/ctl", &steps);
        assert_eq!(runner.call_count(), 3);
    }

    #[test]
    fn compensation_step_partial_eq_different_payload() {
        let mut s2 = step();
        s2.payload = serde_json::json!({"different": true});
        assert_ne!(step(), s2);
    }

    #[test]
    fn append_argv_length_is_exactly_ten_elements() {
        // ctl_path append --kind K --trace-id T --actor A --payload P = 10 elements
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step()]).unwrap();
        assert_eq!(runner.argv_at(0).len(), 10);
    }

    // -----------------------------------------------------------------------
    // Additional behavioral tests
    // -----------------------------------------------------------------------

    /// When step 2 of 3 fails, step 3 must not be attempted (fail-fast).
    /// This probes the mid-sequence failure path not covered by first-step or
    /// last-step failure tests.
    #[test]
    fn mid_step_failure_stops_subsequent_steps() {
        let step3 = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t-3".to_string(),
            actor: "dcg-admit".to_string(),
            payload: serde_json::json!({"reverses": "step-3"}),
        };
        let runner = FakeRunner::new(vec![
            ok(append_ok_json()),
            fail(2, "mid-step failure"),
            ok(append_ok_json()), // must not be consumed
        ]);
        let result = compensate(&runner, "/ctl", &[step(), step2(), step3]);
        assert!(result.is_err());
        assert_eq!(
            runner.call_count(),
            2,
            "third step must not be attempted after mid-step failure"
        );
    }

    /// Three steps must be sent to the runner in index order, each carrying the
    /// `trace_id` from its own `CompensationStep`, verifying append ORDER.
    #[test]
    fn three_steps_carry_trace_ids_in_order() {
        let step3 = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t-3".to_string(),
            actor: "dcg-admit".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![
            ok(append_ok_json()),
            ok(append_ok_json()),
            ok(append_ok_json()),
        ]);
        compensate(&runner, "/ctl", &[step(), step2(), step3]).unwrap();
        for (i, expected_trace) in ["t-1", "t-2", "t-3"].iter().enumerate() {
            let argv = runner.argv_at(i);
            let pos = argv.iter().position(|a| a == "--trace-id").unwrap();
            assert_eq!(
                &argv[pos + 1], expected_trace,
                "step {i} must carry trace_id {expected_trace}"
            );
        }
    }

    /// Even after some steps succeed and one fails, the successful calls must
    /// all have used the `append` verb — never `submit` or any other verb.
    #[test]
    fn append_verb_present_on_each_call_even_after_partial_success() {
        let step3 = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t-3".to_string(),
            actor: "dcg-admit".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![
            ok(append_ok_json()),
            ok(append_ok_json()),
            fail(1, "third fails"),
        ]);
        let _ = compensate(&runner, "/ctl", &[step(), step2(), step3]);
        for i in 0..2 {
            assert_eq!(
                runner.argv_at(i).get(1).map(String::as_str),
                Some("append"),
                "call {i} must use append verb even when a later step fails"
            );
        }
    }

    /// `snapshot` must never appear in any argv across any number of steps —
    /// compensate is append-only; snapshot is a warrant-path concept.
    #[test]
    fn no_snapshot_verb_issued_in_any_call() {
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[step(), step2()]).unwrap();
        let all = runner.all_argv_flat();
        assert!(
            !all.iter().any(|a| a == "snapshot"),
            "compensate must never issue `snapshot`"
        );
    }

    /// Two steps with distinct `kind` values must each produce their own kind
    /// in argv, proving field isolation across iterations.
    #[test]
    fn different_kinds_per_step_propagate_independently() {
        let s1 = CompensationStep {
            kind: "saga.rollback.v1".to_string(),
            trace_id: "t-1".to_string(),
            actor: "actor-a".to_string(),
            payload: serde_json::json!({}),
        };
        let s2 = CompensationStep {
            kind: "saga.rollback.v2".to_string(),
            trace_id: "t-2".to_string(),
            actor: "actor-b".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s1, s2]).unwrap();
        let argv0 = runner.argv_at(0);
        let argv1 = runner.argv_at(1);
        let k0 = argv0.iter().position(|a| a == "--kind").unwrap();
        let k1 = argv1.iter().position(|a| a == "--kind").unwrap();
        assert_eq!(argv0[k0 + 1], "saga.rollback.v1");
        assert_eq!(argv1[k1 + 1], "saga.rollback.v2");
    }

    /// Two steps with distinct `actor` values must each propagate their own
    /// actor to `--actor`, not share or cross-contaminate.
    #[test]
    fn different_actors_per_step_propagate_independently() {
        let s1 = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t-1".to_string(),
            actor: "fiber-alpha".to_string(),
            payload: serde_json::json!({}),
        };
        let s2 = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: "t-2".to_string(),
            actor: "fiber-beta".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json()), ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s1, s2]).unwrap();
        let argv0 = runner.argv_at(0);
        let argv1 = runner.argv_at(1);
        let a0 = argv0.iter().position(|a| a == "--actor").unwrap();
        let a1 = argv1.iter().position(|a| a == "--actor").unwrap();
        assert_eq!(argv0[a0 + 1], "fiber-alpha");
        assert_eq!(argv1[a1 + 1], "fiber-beta");
    }

    /// A `trace_id` of 128 chars must arrive verbatim in `--trace-id` with no
    /// truncation — compensate must not impose its own length limit.
    #[test]
    fn long_trace_id_propagates_verbatim_to_argv() {
        let long_trace = "t-".repeat(64); // 128 chars
        let s = CompensationStep {
            kind: "dcg.compensate".to_string(),
            trace_id: long_trace.clone(),
            actor: "dcg-admit".to_string(),
            payload: serde_json::json!({}),
        };
        let runner = FakeRunner::new(vec![ok(append_ok_json())]);
        compensate(&runner, "/ctl", &[s]).unwrap();
        let argv = runner.argv_at(0);
        let pos = argv.iter().position(|a| a == "--trace-id").unwrap();
        assert_eq!(argv[pos + 1], long_trace);
    }

    /// On failure the `Subprocess` error must identify both the command (ctl
    /// path + append verb) and the non-zero exit code, so the caller can
    /// escalate with a useful trace.
    #[test]
    fn failure_error_identifies_command_and_exit_code() {
        let runner = FakeRunner::new(vec![fail(13, "step rejected")]);
        let err = compensate(&runner, "/ctl", &[step()]).unwrap_err();
        let DcgError::Subprocess { command, detail } = err else {
            panic!("expected Subprocess, got: {err}")
        };
        assert!(command.contains("append"), "command must identify the append verb, got: {command}");
        assert!(detail.contains("13"), "detail must include exit code 13, got: {detail}");
    }
}
