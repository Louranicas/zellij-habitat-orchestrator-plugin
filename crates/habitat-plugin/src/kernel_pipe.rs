use serde_json::{json, Value};

pub const PIPE_RESPONSE_SCHEMA: &str = "habitat.kernel.pipe.response.v1";
pub const PIPE_DEADLINE_MS: i64 = 1000;
pub const PIPE_MODE_FAIL_CLOSED: &str = "A_FAIL_CLOSED";
pub const PIPE_MODE_SEALED_SYNC: &str = "B_SEALED_SYNC";

#[derive(Clone, Copy)]
pub struct PipeResponse<'a> {
    pub mode: &'a str,
    pub trace_id: &'a str,
    pub verdict: &'a str,
    pub reason: &'a str,
    pub attempted: bool,
    pub event_id: Option<&'a str>,
    pub event_hash: Option<&'a str>,
    pub request_hash: Option<&'a str>,
}

#[must_use]
pub fn response_from_sidecar(trace_id: &str, sidecar: &Value) -> Value {
    let verdict = sidecar
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("NACK");
    let event_id = sidecar.get("event_id").and_then(Value::as_str);
    let event_hash = sidecar.get("event_hash").and_then(Value::as_str);
    let request_hash = sidecar.get("request_hash").and_then(Value::as_str);
    let reason = sidecar
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("SIDECAR_RESPONSE");

    if verdict == "ACK_DURABLE" {
        response(PipeResponse {
            mode: PIPE_MODE_SEALED_SYNC,
            trace_id,
            verdict: "ACK_DURABLE",
            reason,
            attempted: true,
            event_id,
            event_hash,
            request_hash,
        })
    } else {
        response(PipeResponse {
            mode: PIPE_MODE_SEALED_SYNC,
            trace_id,
            verdict: "NACK_USE_SIDECAR_SUBMIT",
            reason,
            attempted: true,
            event_id,
            event_hash,
            request_hash,
        })
    }
}

#[must_use]
pub fn schema_invalid_response(reason: &str) -> Value {
    response(PipeResponse {
        mode: PIPE_MODE_FAIL_CLOSED,
        trace_id: "kernel-pipe",
        verdict: "NACK_SCHEMA_INVALID",
        reason,
        attempted: false,
        event_id: None,
        event_hash: None,
        request_hash: None,
    })
}

#[must_use]
pub fn use_sidecar_submit_response(trace_id: &str) -> Value {
    response(PipeResponse {
        mode: PIPE_MODE_FAIL_CLOSED,
        trace_id,
        verdict: "NACK_USE_SIDECAR_SUBMIT",
        reason: "PLUGIN_PIPE_FAIL_CLOSED_USE_ORCH_KERNELCTL_SUBMIT",
        attempted: false,
        event_id: None,
        event_hash: None,
        request_hash: None,
    })
}

#[must_use]
pub fn sidecar_invalid_response(trace_id: &str) -> Value {
    response(PipeResponse {
        mode: PIPE_MODE_SEALED_SYNC,
        trace_id,
        verdict: "DEGRADED_SIDECAR_BUSY",
        reason: "SIDECAR_RESPONSE_INVALID",
        attempted: true,
        event_id: None,
        event_hash: None,
        request_hash: None,
    })
}

#[must_use]
pub fn sidecar_submit_failed_response(trace_id: &str, stderr: &str) -> Value {
    response(PipeResponse {
        mode: PIPE_MODE_SEALED_SYNC,
        trace_id,
        verdict: "DEGRADED_SIDECAR_BUSY",
        reason: &format!("SIDECAR_SUBMIT_FAILED: {stderr}"),
        attempted: true,
        event_id: None,
        event_hash: None,
        request_hash: None,
    })
}

#[must_use]
pub fn response(response: PipeResponse<'_>) -> Value {
    json!({
        "schema": PIPE_RESPONSE_SCHEMA,
        "trace_id": response.trace_id,
        "mode": response.mode,
        "verdict": response.verdict,
        "reason": response.reason,
        "deadline_ms": PIPE_DEADLINE_MS,
        "response_ms": 0,
        "sidecar_submission": {
            "attempted": response.attempted,
            "event_id": response.event_id,
            "event_hash": response.event_hash,
            "request_hash": response.request_hash
        },
        "instance": {
            "instance_id": "habitat-plugin-v0.1.2",
            "load_kind": "unknown"
        },
        "circuit": {
            "state": "closed",
            "previous_state": "unknown",
            "next_probe": "orch-kernelctl submit"
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_durable_sidecar_response_preserves_receipt_fields() {
        let sidecar = json!({
            "verdict": "ACK_DURABLE",
            "reason": "INGESTED",
            "event_id": "evt_1",
            "event_hash": "sha256:event",
            "request_hash": "sha256:request"
        });

        let actual = response_from_sidecar("trace-1", &sidecar);

        assert_eq!(actual["schema"], PIPE_RESPONSE_SCHEMA);
        assert_eq!(actual["trace_id"], "trace-1");
        assert_eq!(actual["mode"], PIPE_MODE_SEALED_SYNC);
        assert_eq!(actual["verdict"], "ACK_DURABLE");
        assert_eq!(actual["reason"], "INGESTED");
        assert_eq!(actual["sidecar_submission"]["attempted"], true);
        assert_eq!(actual["sidecar_submission"]["event_id"], "evt_1");
        assert_eq!(actual["sidecar_submission"]["event_hash"], "sha256:event");
        assert_eq!(
            actual["sidecar_submission"]["request_hash"],
            "sha256:request"
        );
    }

    #[test]
    fn non_ack_sidecar_response_remains_sealed_but_not_durable() {
        let sidecar = json!({
            "verdict": "NACK",
            "reason": "DUPLICATE_IDEMPOTENCY_CONFLICT",
            "request_hash": "sha256:request"
        });

        let actual = response_from_sidecar("trace-2", &sidecar);

        assert_eq!(actual["mode"], PIPE_MODE_SEALED_SYNC);
        assert_eq!(actual["verdict"], "NACK_USE_SIDECAR_SUBMIT");
        assert_eq!(actual["reason"], "DUPLICATE_IDEMPOTENCY_CONFLICT");
        assert_eq!(actual["sidecar_submission"]["attempted"], true);
        assert_eq!(actual["sidecar_submission"]["event_id"], Value::Null);
        assert_eq!(
            actual["sidecar_submission"]["request_hash"],
            "sha256:request"
        );
    }

    #[test]
    fn invalid_schema_response_is_fail_closed_without_submission() {
        let actual = schema_invalid_response("SCHEMA_INVALID: expected value");

        assert_eq!(actual["mode"], PIPE_MODE_FAIL_CLOSED);
        assert_eq!(actual["verdict"], "NACK_SCHEMA_INVALID");
        assert_eq!(actual["reason"], "SCHEMA_INVALID: expected value");
        assert_eq!(actual["sidecar_submission"]["attempted"], false);
        assert_eq!(actual["sidecar_submission"]["event_id"], Value::Null);
    }

    #[test]
    fn valid_payload_in_mode_a_returns_terminal_sidecar_instruction() {
        let actual = use_sidecar_submit_response("trace-mode-a");

        assert_eq!(actual["mode"], PIPE_MODE_FAIL_CLOSED);
        assert_eq!(actual["trace_id"], "trace-mode-a");
        assert_eq!(actual["verdict"], "NACK_USE_SIDECAR_SUBMIT");
        assert_eq!(
            actual["reason"],
            "PLUGIN_PIPE_FAIL_CLOSED_USE_ORCH_KERNELCTL_SUBMIT"
        );
        assert_eq!(actual["sidecar_submission"]["attempted"], false);
    }

    #[test]
    fn sidecar_submit_failure_reports_degraded_attempt() {
        let actual = sidecar_submit_failed_response("trace-3", "database is locked");

        assert_eq!(actual["trace_id"], "trace-3");
        assert_eq!(actual["mode"], PIPE_MODE_SEALED_SYNC);
        assert_eq!(actual["verdict"], "DEGRADED_SIDECAR_BUSY");
        assert_eq!(
            actual["reason"],
            "SIDECAR_SUBMIT_FAILED: database is locked"
        );
        assert_eq!(actual["sidecar_submission"]["attempted"], true);
    }
}
