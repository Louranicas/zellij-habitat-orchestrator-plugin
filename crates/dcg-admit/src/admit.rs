//! The admission join-point.
//!
//! [`admit_write`] is the single entry both the cortex and workflow fibers call
//! to admit a delegated write. It composes four guards in strict order,
//! fail-fast:
//!
//! 1. **Arming** — [`crate::arming::ArmingReader::read`] must return
//!    [`crate::arming::ArmingState::Armed`]; otherwise [`DcgError::NotArmed`].
//! 2. **Fence — lower bound** — the presented [`Fence`] must strictly
//!    [`Fence::supersedes`] the resource's last-admitted fence (read from the
//!    spine via `orch-kernelctl events`; defaults to [`Fence::GENESIS`] when no
//!    prior admission exists); otherwise [`DcgError::StaleFence`].
//! 3. **Fence — upper bound** — the presented [`Fence`] must not exceed the
//!    current live spine `last_seq` (read via `orch-kernelctl snapshot --json`).
//!    A fence above the spine's monotonic sequence cannot legitimately exist;
//!    recording it as the new floor permanently poisons the resource (no future
//!    fence ≤ `u64::MAX` can supersede it). A spine read-failure **fails
//!    CLOSED** — it never silently admits a bogus fence.  On failure:
//!    [`DcgError::Subprocess`] (spine fault) or [`DcgError::OutOfRange`]
//!    (fence above current seq).
//! 4. **Warrant** — the actuation is routed via
//!    [`crate::warrant::submit_actuation`] (Err-as-denial; build-plan §8 A2).
//!
//! On a multi-step write that partially fails (warrant admitted, fence record
//! failed), [`crate::saga::compensate`] is invoked to append a compensating
//! event (append-only; chain rows are NEVER deleted). The new fence floor is
//! persisted to the spine so the next admission sees it as the floor.
//!
//! # Fence storage
//!
//! The last-admitted fence for a resource is stored as a `dcg.fence.admitted`
//! event on the spine, keyed by the trace-id `dcg-fence:<resource>`. Reading
//! uses `orch-kernelctl events --trace <trace-id>` and takes the payload of
//! the last returned event. Writing uses `orch-kernelctl append`.
//!
//! # Guard ordering guarantee
//!
//! Arming is checked before fence-lower; fence-lower before fence-upper;
//! fence-upper before warrant. A failure at any guard short-circuits the
//! remaining guards. This is observable through the precise sequence of runner
//! calls made.

use crate::arming::{ArmingReader, ARMING_KEY};
use crate::error::DcgError;
use crate::exec::CommandRunner;
use crate::fence::Fence;
use crate::saga::CompensationStep;
use crate::warrant::Actuation;
use crate::Result;

/// A request to admit a single delegated write to a fenced resource.
///
/// `payload` is a [`serde_json::Value`], so this type is `PartialEq` but not
/// `Eq` (JSON numbers are floats).
#[derive(Clone, Debug, PartialEq)]
pub struct AdmitRequest {
    /// The fenced resource being written (for example `git.index.workspace`).
    pub resource: String,
    /// The lease owner presenting the write.
    pub owner: String,
    /// The fence the caller holds for the resource.
    pub presented_fence: Fence,
    /// The warrant kind for the actuation (for example `recipe.execution`).
    pub kind: String,
    /// Trace id correlating the admission across the spine event trail.
    pub trace_id: String,
    /// The actuation payload.
    pub payload: serde_json::Value,
}

/// Receipt returned when a delegated write is admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmitReceipt {
    /// Monotonic sequence assigned by the spine to the admitted write (from the
    /// warrant receipt).
    pub seq: i64,
    /// The fence recorded as the resource's new last-admitted floor.
    pub admitted_fence: Fence,
    /// Durable event id of the admitted write.
    pub event_id: String,
}

/// Admits (or refuses) a single delegated write.
///
/// Evaluates arming, fence-lower-bound, fence-upper-bound, and warrant guards
/// in order and, on success, records the admitted fence as the resource's new
/// floor. Partial failure (warrant admitted, fence persist failed) triggers
/// saga compensation.
///
/// # Errors
/// Returns [`DcgError::NotArmed`] if the arming key is not `armed`.
/// Returns [`DcgError::StaleFence`] if `req.presented_fence` does not
/// supersede the resource's last-admitted fence.
/// Returns [`DcgError::OutOfRange`] if `req.presented_fence` exceeds the
/// current live spine `last_seq` (floor-poison protection).
/// Returns [`DcgError::Denied`] if the warrant policy refuses the actuation.
/// Returns [`DcgError::Subprocess`] if `orch-kernelctl` cannot be run or
/// exits non-zero during fence read/upper-bound check/record or saga
/// compensation.
/// Returns [`DcgError::Parse`] if a spine response cannot be decoded.
pub fn admit_write(
    runner: &dyn CommandRunner,
    arming: &dyn ArmingReader,
    ctl_path: &str,
    req: &AdmitRequest,
) -> Result<AdmitReceipt> {
    // Guard 1: arming (checked first; short-circuits everything else).
    let arming_state = arming.read()?;
    if !arming_state.is_armed() {
        return Err(DcgError::NotArmed {
            key: ARMING_KEY.to_string(),
        });
    }

    // Guard 2a: fence lower-bound — presented fence must supersede last-admitted
    // (the Kleppmann / Azure-Redlock stale-write guard).
    let last_fence = read_last_fence(runner, ctl_path, &req.resource)?;
    if !req.presented_fence.supersedes(last_fence) {
        return Err(DcgError::StaleFence {
            resource: req.resource.clone(),
            presented: req.presented_fence.get(),
            last_admitted: last_fence.get(),
        });
    }

    // Guard 2b: fence upper-bound — presented fence must not exceed the live
    // spine seq.  Fences are minted from spine events that have already been
    // committed; a fence above the current last_seq cannot legitimately exist.
    // Recording such a fence as the new floor would permanently poison the
    // resource (no future fence can supersede u64::MAX).  Fail CLOSED: a spine
    // read-failure must not silently permit a bogus fence through.
    let current_seq = read_current_seq(runner, ctl_path)?;
    if req.presented_fence.get() > current_seq {
        return Err(DcgError::OutOfRange {
            field: "presented_fence",
            value: format!(
                "{} exceeds current spine seq {current_seq}",
                req.presented_fence.get()
            ),
        });
    }

    // Guard 3: warrant (Err-as-denial; build-plan §8 A2).
    let actuation = Actuation {
        kind: req.kind.clone(),
        trace_id: req.trace_id.clone(),
        actor: req.owner.clone(),
        payload: req.payload.clone(),
    };
    let receipt = crate::warrant::submit_actuation(runner, ctl_path, &actuation)?;

    // Persist the admitted fence floor so the next admission sees it.
    match record_fence(runner, ctl_path, &req.resource, req.presented_fence) {
        Ok(()) => Ok(AdmitReceipt {
            seq: receipt.seq,
            admitted_fence: req.presented_fence,
            event_id: receipt.event_id,
        }),
        Err(record_err) => {
            // Partial failure: warrant admitted but fence not recorded.
            // Append a compensating event (append-only; never delete).
            let comp = CompensationStep {
                kind: "dcg.compensate".to_string(),
                trace_id: req.trace_id.clone(),
                actor: "dcg-admit".to_string(),
                payload: serde_json::json!({
                    "reason": "fence_record_failed",
                    "admitted_event_id": receipt.event_id,
                    "resource": req.resource,
                    "fence": req.presented_fence.get(),
                    "error": record_err.to_string()
                }),
            };
            match crate::saga::compensate(runner, ctl_path, &[comp]) {
                Ok(()) => Err(record_err),
                Err(comp_err) => Err(DcgError::Subprocess {
                    command: "saga-compensate".to_string(),
                    detail: format!(
                        "fence-record failed: {record_err}; compensation also failed: {comp_err}"
                    ),
                }),
            }
        }
    }
}

/// Evaluates arming, fence-lower-bound, and fence-upper-bound guards without
/// submitting the warrant. Used by the `--dry-run` CLI path to preview
/// admission without actuation.
///
/// # Errors
/// Returns [`DcgError::NotArmed`] if the arming key is not `armed`.
/// Returns [`DcgError::StaleFence`] if the presented fence is stale.
/// Returns [`DcgError::OutOfRange`] if the presented fence exceeds the current
/// live spine seq.
/// Returns [`DcgError::Subprocess`] / [`DcgError::Parse`] on transport faults.
pub(crate) fn check_guards(
    runner: &dyn CommandRunner,
    arming: &dyn ArmingReader,
    ctl_path: &str,
    req: &AdmitRequest,
) -> Result<()> {
    let arming_state = arming.read()?;
    if !arming_state.is_armed() {
        return Err(DcgError::NotArmed {
            key: ARMING_KEY.to_string(),
        });
    }
    let last_fence = read_last_fence(runner, ctl_path, &req.resource)?;
    if !req.presented_fence.supersedes(last_fence) {
        return Err(DcgError::StaleFence {
            resource: req.resource.clone(),
            presented: req.presented_fence.get(),
            last_admitted: last_fence.get(),
        });
    }
    // Guard 2b: upper-bound check (same semantics as admit_write).
    let current_seq = read_current_seq(runner, ctl_path)?;
    if req.presented_fence.get() > current_seq {
        return Err(DcgError::OutOfRange {
            field: "presented_fence",
            value: format!(
                "{} exceeds current spine seq {current_seq}",
                req.presented_fence.get()
            ),
        });
    }
    Ok(())
}

/// Returns the deterministic spine trace-id used for fence events of a resource.
fn fence_trace_id(resource: &str) -> String {
    format!("dcg-fence:{resource}")
}

/// Reads the last admitted fence for `resource` from the spine.
///
/// Calls `orch-kernelctl events --trace "dcg-fence:<resource>"`, parses the
/// returned JSON array, and extracts the `fence` field from the last event's
/// `payload_json`. Returns [`Fence::GENESIS`] when the array is empty (no
/// prior admission). A transport or parse failure is surfaced; it is never
/// silently swallowed.
pub(crate) fn read_last_fence(
    runner: &dyn CommandRunner,
    ctl_path: &str,
    resource: &str,
) -> Result<Fence> {
    let trace = fence_trace_id(resource);
    let argv = [
        ctl_path.to_string(),
        "events".to_string(),
        "--trace".to_string(),
        trace,
    ];
    let out = runner.run(&argv)?;

    // A non-zero exit from `events` is a REAL fault (locked/busy DB, chain
    // corruption, env failure) — NOT "no fence events yet". The legitimate
    // empty case is an exit-0 `[]`, handled by the `events.last()` None branch
    // below. Fail CLOSED: a stale-rejection guard that defaults open under the
    // very contention it is meant to guard is no guard at all (Redlock; the
    // anti-stale token must never collapse to GENESIS on a spine fault).
    if !out.is_success() {
        return Err(DcgError::Subprocess {
            command: "orch-kernelctl events".to_string(),
            detail: format!("exit {}: {}", out.status, out.stderr.trim()),
        });
    }

    let events: Vec<serde_json::Value> =
        serde_json::from_str(out.stdout.trim()).map_err(|e| DcgError::Parse {
            source: "fence-events",
            detail: e.to_string(),
        })?;

    let Some(last_event) = events.last() else {
        // Empty array → GENESIS.
        return Ok(Fence::GENESIS);
    };

    // EventRow.payload_json is the serialised-string payload field.
    let payload_str = last_event["payload_json"].as_str().ok_or_else(|| DcgError::Parse {
        source: "fence-event-payload_json",
        detail: "missing or non-string `payload_json` field".to_string(),
    })?;

    let payload: serde_json::Value =
        serde_json::from_str(payload_str).map_err(|e| DcgError::Parse {
            source: "fence-event-payload",
            detail: e.to_string(),
        })?;

    let fence_val = payload["fence"].as_u64().ok_or_else(|| DcgError::Parse {
        source: "fence-event-payload",
        detail: "missing or non-u64 `fence` field".to_string(),
    })?;

    Ok(Fence::new(fence_val))
}

/// Reads the current live spine sequence number from the orchestrator-kernel
/// spine via `orch-kernelctl snapshot --json`.
///
/// The returned value is the spine's monotonic `last_seq` cast to `u64`. It
/// is used as the **upper bound** for any presented fence: a fence cannot
/// legitimately exceed the spine's monotonic sequence, because fences are
/// minted from spine events that have already been committed. Recording a fence
/// above this bound would permanently poison the resource floor (no future
/// fence can supersede `u64::MAX`).
///
/// Fail CLOSED: a spine read-failure surfaces as an error and must never
/// silently permit a bogus fence through.
///
/// # Errors
/// Returns [`DcgError::Subprocess`] if `orch-kernelctl snapshot` exits
/// non-zero or cannot be spawned.
/// Returns [`DcgError::Parse`] if the snapshot response is not valid JSON or
/// lacks the `last_seq` field.
/// Returns [`DcgError::OutOfRange`] if `last_seq` is negative (corrupt spine).
pub(crate) fn read_current_seq(runner: &dyn CommandRunner, ctl_path: &str) -> Result<u64> {
    let argv = [
        ctl_path.to_string(),
        "snapshot".to_string(),
        "--json".to_string(),
    ];
    let out = runner.run(&argv)?;

    if !out.is_success() {
        return Err(DcgError::Subprocess {
            command: format!("{ctl_path} snapshot"),
            detail: format!("exit {}: {}", out.status, out.stderr.trim()),
        });
    }

    let snap: serde_json::Value =
        serde_json::from_str(out.stdout.trim()).map_err(|e| DcgError::Parse {
            source: "snapshot-response",
            detail: e.to_string(),
        })?;

    let seq = snap["last_seq"].as_i64().ok_or_else(|| DcgError::Parse {
        source: "snapshot-response",
        detail: "missing or non-integer `last_seq` field".to_string(),
    })?;

    // A negative last_seq signals a corrupt or uninitialized spine. Fail closed:
    // do not coerce a negative value into a large u64 upper-bound that would
    // permit any fence through.
    u64::try_from(seq).map_err(|_| DcgError::OutOfRange {
        field: "snapshot.last_seq",
        value: seq.to_string(),
    })
}

/// Appends a `dcg.fence.admitted` event to the spine, recording `fence` as
/// the new last-admitted floor for `resource`.
///
/// Uses `orch-kernelctl append` (append-only verb; never deletes).
fn record_fence(
    runner: &dyn CommandRunner,
    ctl_path: &str,
    resource: &str,
    fence: Fence,
) -> Result<()> {
    let trace = fence_trace_id(resource);
    let payload = serde_json::json!({
        "resource": resource,
        "fence": fence.get()
    });
    let payload_json = serde_json::to_string(&payload)?;

    let argv = [
        ctl_path.to_string(),
        "append".to_string(),
        "--kind".to_string(),
        "dcg.fence.admitted".to_string(),
        "--trace-id".to_string(),
        trace,
        "--actor".to_string(),
        "dcg-admit".to_string(),
        "--payload".to_string(),
        payload_json,
    ];

    let out = runner.run(&argv)?;

    if !out.is_success() {
        return Err(DcgError::Subprocess {
            command: format!("{ctl_path} append"),
            detail: format!("exit {}: {}", out.status, out.stderr.trim()),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::arming::ArmingState;
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

    struct FixedArming(ArmingState);
    impl ArmingReader for FixedArming {
        fn read(&self) -> crate::Result<ArmingState> {
            Ok(self.0)
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

    fn request() -> AdmitRequest {
        AdmitRequest {
            resource: "git.index.workspace".to_string(),
            owner: "orch-refactor:host".to_string(),
            presented_fence: Fence::new(5),
            kind: "recipe.execution".to_string(),
            trace_id: "t-1".to_string(),
            payload: serde_json::json!({}),
        }
    }

    /// Empty events array (no prior fence → GENESIS).
    fn events_empty() -> &'static str {
        "[]"
    }

    /// Events array with a prior fence of value `v`.
    fn events_with_fence(v: u64) -> String {
        let payload = serde_json::json!({"resource": "git.index.workspace", "fence": v});
        let payload_str = serde_json::to_string(&payload).unwrap();
        // EventRow shape: seq, event_id, trace_id, kind, actor, payload_json, hash
        let row = serde_json::json!([{
            "seq": 1,
            "event_id": "evt-prior",
            "trace_id": "dcg-fence:git.index.workspace",
            "kind": "dcg.fence.admitted",
            "actor": "dcg-admit",
            "payload_json": payload_str,
            "hash": "abc"
        }]);
        serde_json::to_string(&row).unwrap()
    }

    fn submit_ok_json() -> &'static str {
        r#"{"schema":"task.submit.v1","verdict":"ACK_DURABLE","trace_id":"t-1","event_id":"evt-42","event_hash":"abc","integration_state":"admitted","idempotency":"NEW","reason":"","request_hash":"xyz"}"#
    }

    fn snapshot_json(seq: i64) -> String {
        format!(r#"{{"last_seq":{seq},"verify_chain_ok":true}}"#)
    }

    fn append_ok_json() -> &'static str {
        r#"{"seq":10,"event_id":"evt-fence","trace_id":"dcg-fence:git.index.workspace","kind":"dcg.fence.admitted","actor":"dcg-admit","payload_json":"{}","hash":"def"}"#
    }

    /// Happy-path runner (5 calls):
    /// 0: events (read last fence — returns empty → GENESIS)
    /// 1: snapshot (upper-bound check — `current_seq` = 42)
    /// 2: submit (warrant — ok)
    /// 3: snapshot (inside `submit_actuation` for receipt seq — `last_seq` = 42)
    /// 4: append (record fence — ok)
    fn happy_runner() -> FakeRunner {
        FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
            ok(submit_ok_json()),
            ok(&snapshot_json(42)),
            ok(append_ok_json()),
        ])
    }

    // -----------------------------------------------------------------------
    // read_last_fence: fail-closed on spine fault (the Redlock guard)
    // -----------------------------------------------------------------------

    #[test]
    fn read_last_fence_fails_closed_on_nonzero_events_exit() {
        // A non-zero `events` exit is a real fault (locked/busy DB, corruption) —
        // it MUST surface as Err, never collapse to GENESIS (which would let any
        // presented fence >=1 supersede and defeat the stale-write guard).
        let runner = FakeRunner::new(vec![fail(1, "database is locked")]);
        let err = read_last_fence(&runner, "/ctl", "git.index.workspace").unwrap_err();
        assert!(
            matches!(err, DcgError::Subprocess { .. }),
            "spine fault must fail closed as Subprocess, got {err:?}"
        );
    }

    #[test]
    fn read_last_fence_empty_events_is_genesis_on_exit_zero() {
        // The legitimate "no prior fence" case is exit-0 with `[]` — that, and
        // only that, is GENESIS.
        let runner = FakeRunner::new(vec![ok(events_empty())]);
        let fence = read_last_fence(&runner, "/ctl", "git.index.workspace").unwrap();
        assert_eq!(fence, Fence::GENESIS);
    }

    // -----------------------------------------------------------------------
    // read_current_seq: fail-closed upper-bound helper
    // -----------------------------------------------------------------------

    #[test]
    fn read_current_seq_returns_last_seq_on_success() {
        let runner = FakeRunner::new(vec![ok(&snapshot_json(99))]);
        let seq = read_current_seq(&runner, "/ctl").unwrap();
        assert_eq!(seq, 99);
    }

    #[test]
    fn read_current_seq_zero_last_seq_is_valid() {
        let runner = FakeRunner::new(vec![ok(&snapshot_json(0))]);
        let seq = read_current_seq(&runner, "/ctl").unwrap();
        assert_eq!(seq, 0);
    }

    #[test]
    fn read_current_seq_nonzero_exit_fails_closed() {
        // A non-zero snapshot exit must surface as Subprocess — never silently
        // return a permissive upper bound.
        let runner = FakeRunner::new(vec![fail(1, "database locked")]);
        let err = read_current_seq(&runner, "/ctl").unwrap_err();
        assert!(
            matches!(err, DcgError::Subprocess { .. }),
            "snapshot failure must fail closed as Subprocess, got {err:?}"
        );
    }

    #[test]
    fn read_current_seq_bad_json_returns_parse_error() {
        let runner = FakeRunner::new(vec![ok("not json")]);
        let err = read_current_seq(&runner, "/ctl").unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "snapshot-response", .. }));
    }

    #[test]
    fn read_current_seq_missing_last_seq_returns_parse_error() {
        let runner = FakeRunner::new(vec![ok(r#"{"verify_chain_ok":true}"#)]);
        let err = read_current_seq(&runner, "/ctl").unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "snapshot-response", .. }));
    }

    #[test]
    fn read_current_seq_negative_last_seq_returns_out_of_range() {
        // A negative last_seq is a corrupt spine — must not be coerced to a huge
        // u64 that would grant any fence through.
        let runner = FakeRunner::new(vec![ok(r#"{"last_seq":-1,"verify_chain_ok":false}"#)]);
        let err = read_current_seq(&runner, "/ctl").unwrap_err();
        assert!(
            matches!(err, DcgError::OutOfRange { field: "snapshot.last_seq", .. }),
            "negative last_seq must be rejected as OutOfRange, got {err:?}"
        );
    }

    #[test]
    fn read_current_seq_uses_snapshot_verb() {
        let runner = FakeRunner::new(vec![ok(&snapshot_json(1))]);
        read_current_seq(&runner, "/ctl").unwrap();
        let argv = runner.argv_at(0);
        assert_eq!(argv.get(1).map(String::as_str), Some("snapshot"));
    }

    #[test]
    fn read_current_seq_uses_json_flag() {
        let runner = FakeRunner::new(vec![ok(&snapshot_json(1))]);
        read_current_seq(&runner, "/ctl").unwrap();
        let argv = runner.argv_at(0);
        assert!(argv.contains(&"--json".to_string()));
    }

    // -----------------------------------------------------------------------
    // Guard 1: Arming
    // -----------------------------------------------------------------------

    #[test]
    fn not_armed_returns_not_armed_error() {
        let runner = FakeRunner::new(vec![]);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Unarmed), "/ctl", &request()).unwrap_err();
        assert!(matches!(err, DcgError::NotArmed { .. }));
    }

    #[test]
    fn unknown_arming_state_also_returns_not_armed() {
        let runner = FakeRunner::new(vec![]);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Unknown), "/ctl", &request()).unwrap_err();
        assert!(matches!(err, DcgError::NotArmed { .. }));
    }

    #[test]
    fn not_armed_error_names_the_arming_key() {
        let runner = FakeRunner::new(vec![]);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Unarmed), "/ctl", &request()).unwrap_err();
        let DcgError::NotArmed { key } = err else { panic!("expected NotArmed") };
        assert_eq!(key, ARMING_KEY);
    }

    #[test]
    fn arming_failure_makes_no_runner_calls() {
        let runner = FakeRunner::new(vec![]);
        let _ = admit_write(&runner, &FixedArming(ArmingState::Unarmed), "/ctl", &request());
        assert_eq!(runner.call_count(), 0, "no runner calls expected on arming failure");
    }

    // -----------------------------------------------------------------------
    // Guard 2a: Fence lower-bound
    // -----------------------------------------------------------------------

    #[test]
    fn stale_fence_equal_to_last_returns_stale_fence_error() {
        // prior fence = 5, presented = 5 → equal → stale
        let runner = FakeRunner::new(vec![ok(&events_with_fence(5))]);
        let mut req = request();
        req.presented_fence = Fence::new(5);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap_err();
        assert!(matches!(err, DcgError::StaleFence { .. }));
    }

    #[test]
    fn stale_fence_less_than_last_returns_stale_fence_error() {
        let runner = FakeRunner::new(vec![ok(&events_with_fence(10))]);
        let mut req = request();
        req.presented_fence = Fence::new(5);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap_err();
        assert!(matches!(err, DcgError::StaleFence { .. }));
    }

    #[test]
    fn stale_fence_error_names_the_resource() {
        let runner = FakeRunner::new(vec![ok(&events_with_fence(10))]);
        let mut req = request();
        req.presented_fence = Fence::new(5);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap_err();
        let DcgError::StaleFence { resource, .. } = err else { panic!("expected StaleFence") };
        assert_eq!(resource, "git.index.workspace");
    }

    #[test]
    fn stale_fence_error_shows_presented_and_last_admitted() {
        let runner = FakeRunner::new(vec![ok(&events_with_fence(10))]);
        let mut req = request();
        req.presented_fence = Fence::new(3);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap_err();
        let DcgError::StaleFence { presented, last_admitted, .. } = err else {
            panic!("expected StaleFence")
        };
        assert_eq!(presented, 3);
        assert_eq!(last_admitted, 10);
    }

    #[test]
    fn stale_fence_short_circuits_warrant() {
        // runner has events response; if warrant were called, we'd need more
        let runner = FakeRunner::new(vec![ok(&events_with_fence(10))]);
        let mut req = request();
        req.presented_fence = Fence::new(5);
        let _ = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req);
        assert_eq!(runner.call_count(), 1, "only events call expected on stale fence");
    }

    #[test]
    fn genesis_fence_accepted_when_no_prior_admission() {
        // Events returns empty array → last_fence = GENESIS (0)
        // presented = 1 → 1 > 0 → lower-bound OK; current_seq = 42 → 1 ≤ 42 → upper-bound OK
        let runner = happy_runner();
        let mut req = request();
        req.presented_fence = Fence::new(1);
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap();
    }

    #[test]
    fn fresh_fence_accepted_when_strictly_greater() {
        // prior = 4, presented = 5 → 5 > 4 → lower-bound OK; current_seq = 42 → upper-bound OK
        let runner = FakeRunner::new(vec![
            ok(&events_with_fence(4)),
            ok(&snapshot_json(42)),
            ok(submit_ok_json()),
            ok(&snapshot_json(42)),
            ok(append_ok_json()),
        ]);
        let mut req = request();
        req.presented_fence = Fence::new(5);
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap();
    }

    #[test]
    fn fence_events_uses_correct_trace_id_format() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let fence_argv = runner.argv_at(0);
        let pos = fence_argv.iter().position(|a| a == "--trace").expect("--trace missing");
        assert_eq!(fence_argv[pos + 1], "dcg-fence:git.index.workspace");
    }

    #[test]
    fn fence_events_not_called_when_arming_fails() {
        let runner = FakeRunner::new(vec![]);
        let _ = admit_write(&runner, &FixedArming(ArmingState::Unarmed), "/ctl", &request());
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn read_last_fence_genesis_on_empty_events_array() {
        let runner = FakeRunner::new(vec![ok("[]")]);
        let f = read_last_fence(&runner, "/ctl", "my.resource").unwrap();
        assert_eq!(f, Fence::GENESIS);
    }

    // NOTE: the former `read_last_fence_genesis_on_nonzero_events_exit` test was
    // REMOVED — it asserted the fail-OPEN bug (non-zero events exit -> GENESIS).
    // The corrected behavior (non-zero exit -> Err, fail-CLOSED) is pinned by
    // `read_last_fence_fails_closed_on_nonzero_events_exit` above.

    #[test]
    fn read_last_fence_parses_fence_from_last_event() {
        let runner = FakeRunner::new(vec![ok(&events_with_fence(99))]);
        let f = read_last_fence(&runner, "/ctl", "git.index.workspace").unwrap();
        assert_eq!(f.get(), 99);
    }

    #[test]
    fn read_last_fence_parse_error_on_bad_json() {
        let runner = FakeRunner::new(vec![ok("not json")]);
        let err = read_last_fence(&runner, "/ctl", "r").unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "fence-events", .. }));
    }

    #[test]
    fn read_last_fence_parse_error_when_payload_json_missing() {
        let bad = r#"[{"seq":1,"event_id":"e","trace_id":"t","kind":"k","actor":"a","hash":"h"}]"#;
        let runner = FakeRunner::new(vec![ok(bad)]);
        let err = read_last_fence(&runner, "/ctl", "r").unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "fence-event-payload_json", .. }));
    }

    #[test]
    fn read_last_fence_parse_error_when_fence_field_missing() {
        let payload_str = r#"{"resource":"r"}"#;
        let escaped = payload_str.replace('"', "\\\"");
        let body = format!(r#"[{{"seq":1,"event_id":"e","trace_id":"t","kind":"k","actor":"a","payload_json":"{escaped}","hash":"h"}}]"#);
        let runner = FakeRunner::new(vec![ok(&body)]);
        let err = read_last_fence(&runner, "/ctl", "r").unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "fence-event-payload", .. }));
    }

    // -----------------------------------------------------------------------
    // Guard 2b: Fence upper-bound (floor-poison protection)
    // -----------------------------------------------------------------------

    #[test]
    fn upper_bound_fence_exceeds_current_seq_rejected() {
        // presented_fence = u64::MAX, current_seq = 42 → u64::MAX > 42 → REJECTED.
        // This is the floor-poison attack: recording u64::MAX makes the floor
        // permanent (no future fence can supersede it).
        let runner = FakeRunner::new(vec![
            ok(events_empty()),      // events: last_fence = GENESIS
            ok(&snapshot_json(42)), // snapshot: current_seq = 42
        ]);
        let mut req = request();
        req.presented_fence = Fence::new(u64::MAX);
        let err = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap_err();
        assert!(
            matches!(err, DcgError::OutOfRange { field: "presented_fence", .. }),
            "fence above current_seq must be rejected as OutOfRange, got {err:?}"
        );
    }

    #[test]
    fn upper_bound_fence_exceeds_seq_short_circuits_warrant() {
        // When the fence exceeds the spine seq, the warrant must NOT be submitted.
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
        ]);
        let mut req = request();
        req.presented_fence = Fence::new(u64::MAX);
        let _ = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req);
        // Only 2 calls: events + snapshot. Warrant must NOT be called.
        assert_eq!(runner.call_count(), 2, "warrant must not be called when fence > seq");
    }

    #[test]
    fn upper_bound_fence_equal_to_current_seq_accepted() {
        // presented_fence == current_seq is within bounds (= last committed event).
        // Guard 2a: 42 > 5 (prior fence) → OK; Guard 2b: 42 ≤ 42 → OK.
        let runner = FakeRunner::new(vec![
            ok(&events_with_fence(5)),
            ok(&snapshot_json(42)),
            ok(submit_ok_json()),
            ok(&snapshot_json(42)),
            ok(append_ok_json()),
        ]);
        let mut req = request();
        req.presented_fence = Fence::new(42);
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap();
    }

    #[test]
    fn upper_bound_fence_less_than_current_seq_accepted() {
        // presented_fence < current_seq is the normal case.
        // Guard 2a: 20 > 5 → OK; Guard 2b: 20 ≤ 42 → OK.
        let runner = FakeRunner::new(vec![
            ok(&events_with_fence(5)),
            ok(&snapshot_json(42)),
            ok(submit_ok_json()),
            ok(&snapshot_json(42)),
            ok(append_ok_json()),
        ]);
        let mut req = request();
        req.presented_fence = Fence::new(20);
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap();
    }

    #[test]
    fn upper_bound_spine_read_failure_fails_closed() {
        // If orch-kernelctl snapshot exits non-zero during the upper-bound check,
        // the admission MUST fail closed — not silently admit.
        let runner = FakeRunner::new(vec![
            ok(events_empty()),         // events: last_fence = GENESIS
            fail(1, "database locked"), // snapshot: spine-read failure
        ]);
        let err = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request())
            .unwrap_err();
        assert!(
            matches!(err, DcgError::Subprocess { .. }),
            "spine-read failure during upper-bound check must fail closed, got {err:?}"
        );
        // Exactly 2 calls: events + snapshot. Warrant must NOT be submitted.
        assert_eq!(runner.call_count(), 2);
    }

    #[test]
    fn upper_bound_out_of_range_error_names_presented_fence_field() {
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(10)),
        ]);
        let mut req = request();
        req.presented_fence = Fence::new(999);
        let err = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req).unwrap_err();
        let DcgError::OutOfRange { field, value } = err else {
            panic!("expected OutOfRange, got {err:?}")
        };
        assert_eq!(field, "presented_fence");
        assert!(value.contains("999"), "value should contain presented fence: {value}");
        assert!(value.contains("10"), "value should contain current seq: {value}");
    }

    // -----------------------------------------------------------------------
    // Guard 3: Warrant
    // -----------------------------------------------------------------------

    #[test]
    fn warrant_denied_returns_denied_error() {
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
            fail(1, "RECIPE_EXECUTION DENY"),
        ]);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
    }

    #[test]
    fn warrant_subprocess_error_propagates() {
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
            fail(127, "ctl not found"),
        ]);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap_err();
        // submit non-zero → Denied (Err-as-denial)
        assert!(matches!(err, DcgError::Denied { .. }));
    }

    #[test]
    fn warrant_not_called_when_stale_fence() {
        let runner = FakeRunner::new(vec![ok(&events_with_fence(100))]);
        let mut req = request();
        req.presented_fence = Fence::new(5);
        let _ = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req);
        // Only 1 call (events), not 3 (events + snapshot + submit)
        assert_eq!(runner.call_count(), 1);
    }

    #[test]
    fn actuation_kind_matches_request_kind() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        // Call 0=events, 1=snapshot(upper-bound), 2=submit
        let submit_argv = runner.argv_at(2);
        let json_arg = &submit_argv[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["kind"], "recipe.execution");
    }

    #[test]
    fn actuation_trace_id_matches_request_trace_id() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let submit_argv = runner.argv_at(2);
        let json_arg = &submit_argv[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["trace_id"], "t-1");
    }

    #[test]
    fn actuation_actor_is_request_owner() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let submit_argv = runner.argv_at(2);
        let json_arg = &submit_argv[3];
        let v: serde_json::Value = serde_json::from_str(json_arg).unwrap();
        assert_eq!(v["operator"], "orch-refactor:host");
    }

    // -----------------------------------------------------------------------
    // Happy path receipt
    // -----------------------------------------------------------------------

    #[test]
    fn happy_path_returns_ok_receipt() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
    }

    #[test]
    fn happy_path_receipt_seq_from_snapshot() {
        let runner = happy_runner();
        let r =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        assert_eq!(r.seq, 42);
    }

    #[test]
    fn happy_path_receipt_event_id_from_warrant() {
        let runner = happy_runner();
        let r =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        assert_eq!(r.event_id, "evt-42");
    }

    #[test]
    fn happy_path_admitted_fence_equals_presented() {
        let runner = happy_runner();
        let r =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        assert_eq!(r.admitted_fence, request().presented_fence);
    }

    #[test]
    fn happy_path_makes_five_runner_calls() {
        // events + snapshot(upper-bound) + submit + snapshot(receipt) + append(fence)
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        assert_eq!(runner.call_count(), 5);
    }

    #[test]
    fn fence_record_uses_append_verb() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        // Call index 4 is the fence record (0=events, 1=snapshot, 2=submit, 3=snapshot, 4=append)
        let append_argv = runner.argv_at(4);
        assert_eq!(append_argv.get(1).map(String::as_str), Some("append"));
    }

    #[test]
    fn fence_record_uses_correct_kind() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let append_argv = runner.argv_at(4);
        let pos = append_argv.iter().position(|a| a == "--kind").unwrap();
        assert_eq!(append_argv[pos + 1], "dcg.fence.admitted");
    }

    #[test]
    fn fence_record_payload_contains_resource() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let append_argv = runner.argv_at(4);
        let pos = append_argv.iter().position(|a| a == "--payload").unwrap();
        let v: serde_json::Value = serde_json::from_str(&append_argv[pos + 1]).unwrap();
        assert_eq!(v["resource"], "git.index.workspace");
    }

    #[test]
    fn fence_record_payload_contains_fence_value() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let append_argv = runner.argv_at(4);
        let pos = append_argv.iter().position(|a| a == "--payload").unwrap();
        let v: serde_json::Value = serde_json::from_str(&append_argv[pos + 1]).unwrap();
        assert_eq!(v["fence"], 5_u64);
    }

    #[test]
    fn fence_record_never_issues_delete() {
        let runner = happy_runner();
        admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let all = runner.all_argv_flat();
        assert!(!all.iter().any(|a| a == "delete"), "must never issue delete");
    }

    #[test]
    fn fence_record_not_called_when_warrant_denied() {
        // events + snapshot(upper-bound) + submit(denied) = 3 calls; no append.
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
            fail(1, "denied"),
        ]);
        let _ = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request());
        assert_eq!(runner.call_count(), 3);
    }

    // -----------------------------------------------------------------------
    // Partial failure + saga compensation
    // -----------------------------------------------------------------------

    #[test]
    fn compensation_invoked_when_fence_record_fails() {
        // 0:events=empty 1:snapshot(upper-bound) 2:submit ok 3:snapshot(receipt)
        // 4:fence-append fails 5:compensate-append ok
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
            ok(submit_ok_json()),
            ok(&snapshot_json(42)),
            fail(1, "fence append rejected"),
            ok(r#"{"seq":11,"event_id":"evt-comp","trace_id":"t-1","kind":"dcg.compensate","actor":"dcg-admit","payload_json":"{}","hash":"ccc"}"#),
        ]);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap_err();
        // Returns the fence record error
        assert!(matches!(err, DcgError::Subprocess { .. }));
        // Compensation append was called (6th call)
        assert_eq!(runner.call_count(), 6);
    }

    #[test]
    fn compensation_append_uses_dcg_compensate_kind() {
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
            ok(submit_ok_json()),
            ok(&snapshot_json(42)),
            fail(1, "fence failed"),
            ok(r#"{"seq":11,"event_id":"e","trace_id":"t","kind":"dcg.compensate","actor":"a","payload_json":"{}","hash":"h"}"#),
        ]);
        let _ = admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request());
        // 0=events, 1=snapshot, 2=submit, 3=snapshot, 4=fence-fail, 5=compensate
        let comp_argv = runner.argv_at(5);
        let pos = comp_argv.iter().position(|a| a == "--kind").unwrap();
        assert_eq!(comp_argv[pos + 1], "dcg.compensate");
    }

    #[test]
    fn double_fault_returns_subprocess_error() {
        // fence record fails AND compensation fails
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(42)),
            ok(submit_ok_json()),
            ok(&snapshot_json(42)),
            fail(1, "fence failed"),
            fail(1, "compensation also failed"),
        ]);
        let err =
            admit_write(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap_err();
        // Double fault → Subprocess with combined message
        assert!(matches!(err, DcgError::Subprocess { .. }));
        assert!(err.to_string().contains("compensation also failed"));
    }

    // -----------------------------------------------------------------------
    // check_guards (dry-run path)
    // -----------------------------------------------------------------------

    #[test]
    fn check_guards_not_armed_returns_error() {
        let runner = FakeRunner::new(vec![]);
        let err = check_guards(&runner, &FixedArming(ArmingState::Unarmed), "/ctl", &request())
            .unwrap_err();
        assert!(matches!(err, DcgError::NotArmed { .. }));
    }

    #[test]
    fn check_guards_stale_fence_returns_stale_error() {
        let runner = FakeRunner::new(vec![ok(&events_with_fence(10))]);
        let mut req = request();
        req.presented_fence = Fence::new(3);
        let err = check_guards(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req)
            .unwrap_err();
        assert!(matches!(err, DcgError::StaleFence { .. }));
    }

    #[test]
    fn check_guards_happy_path_ok_and_makes_two_calls() {
        // events (last fence) + snapshot (upper-bound) — no warrant submit.
        let runner = FakeRunner::new(vec![ok(events_empty()), ok(&snapshot_json(42))]);
        check_guards(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        assert_eq!(runner.call_count(), 2);
    }

    #[test]
    fn check_guards_does_not_call_submit() {
        let runner = FakeRunner::new(vec![ok(events_empty()), ok(&snapshot_json(42))]);
        check_guards(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request()).unwrap();
        let all = runner.all_argv_flat();
        assert!(!all.iter().any(|a| a == "submit"), "dry-run must not submit");
    }

    #[test]
    fn check_guards_upper_bound_rejected() {
        // presented_fence > current_seq → check_guards must also reject.
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            ok(&snapshot_json(3)),
        ]);
        let mut req = request();
        req.presented_fence = Fence::new(999);
        let err = check_guards(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req)
            .unwrap_err();
        assert!(
            matches!(err, DcgError::OutOfRange { field: "presented_fence", .. }),
            "check_guards must reject fence > seq as OutOfRange, got {err:?}"
        );
    }

    #[test]
    fn check_guards_upper_bound_spine_failure_fails_closed() {
        // Spine read failure during upper-bound check must fail closed in dry-run too.
        let runner = FakeRunner::new(vec![
            ok(events_empty()),
            fail(1, "snapshot unavailable"),
        ]);
        let err = check_guards(&runner, &FixedArming(ArmingState::Armed), "/ctl", &request())
            .unwrap_err();
        assert!(
            matches!(err, DcgError::Subprocess { .. }),
            "spine fault in check_guards upper-bound must fail closed, got {err:?}"
        );
    }

    #[test]
    fn check_guards_stale_fence_short_circuits_before_snapshot() {
        // Stale fence must short-circuit before the snapshot call (only 1 runner call).
        let runner = FakeRunner::new(vec![ok(&events_with_fence(50))]);
        let mut req = request();
        req.presented_fence = Fence::new(1);
        let _ = check_guards(&runner, &FixedArming(ArmingState::Armed), "/ctl", &req);
        assert_eq!(runner.call_count(), 1, "snapshot must not be called when fence is stale");
    }

    #[test]
    fn check_guards_unknown_arming_returns_not_armed() {
        let runner = FakeRunner::new(vec![]);
        let err =
            check_guards(&runner, &FixedArming(ArmingState::Unknown), "/ctl", &request())
                .unwrap_err();
        assert!(matches!(err, DcgError::NotArmed { .. }));
    }

    // -----------------------------------------------------------------------
    // Struct coverage
    // -----------------------------------------------------------------------

    #[test]
    fn admit_request_fields_accessible() {
        let r = request();
        assert_eq!(r.resource, "git.index.workspace");
        assert_eq!(r.owner, "orch-refactor:host");
        assert_eq!(r.presented_fence.get(), 5);
        assert_eq!(r.kind, "recipe.execution");
        assert_eq!(r.trace_id, "t-1");
    }

    #[test]
    fn admit_receipt_fields_accessible() {
        let r = AdmitReceipt {
            seq: 9,
            admitted_fence: Fence::new(9),
            event_id: "evt-9".to_string(),
        };
        assert_eq!(r.seq, 9);
        assert_eq!(r.admitted_fence.get(), 9);
        assert_eq!(r.event_id, "evt-9");
    }

    #[test]
    fn admit_receipt_eq() {
        let r1 = AdmitReceipt {
            seq: 1,
            admitted_fence: Fence::new(1),
            event_id: "e".to_string(),
        };
        assert_eq!(r1.clone(), r1);
    }

    #[test]
    fn admit_receipt_debug_contains_seq() {
        let r = AdmitReceipt {
            seq: 77,
            admitted_fence: Fence::new(77),
            event_id: "e".to_string(),
        };
        assert!(format!("{r:?}").contains("77"));
    }

    #[test]
    fn fence_trace_id_contains_resource() {
        let tid = fence_trace_id("my.special.resource");
        assert!(tid.contains("my.special.resource"));
        assert!(tid.starts_with("dcg-fence:"));
    }

    #[test]
    fn fence_trace_id_different_resources_give_different_ids() {
        let tid1 = fence_trace_id("res.a");
        let tid2 = fence_trace_id("res.b");
        assert_ne!(tid1, tid2);
    }

    #[test]
    fn read_last_fence_uses_events_verb() {
        let runner = FakeRunner::new(vec![ok("[]")]);
        read_last_fence(&runner, "/ctl", "r").unwrap();
        let argv = runner.argv_at(0);
        assert_eq!(argv.get(1).map(String::as_str), Some("events"));
    }

    #[test]
    fn read_last_fence_uses_trace_flag() {
        let runner = FakeRunner::new(vec![ok("[]")]);
        read_last_fence(&runner, "/ctl", "r").unwrap();
        let argv = runner.argv_at(0);
        assert!(argv.contains(&"--trace".to_string()));
    }

    #[test]
    fn admit_request_debug_contains_resource() {
        let r = request();
        assert!(format!("{r:?}").contains("git.index.workspace"));
    }
}
