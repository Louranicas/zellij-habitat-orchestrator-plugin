#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/habitat-zellij-target}"
STATE_DIR="$(mktemp -d)"
trap 'rm -rf "${STATE_DIR}"' EXIT

cd "${ROOT}"

submit() {
  ORCH_KERNEL_STATE_DIR="${STATE_DIR}" CARGO_TARGET_DIR="${TARGET_DIR}" \
    cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- submit --json "$1"
}

REQ_A='{"schema":"habitat.kernel.submit.request.v1","trace_id":"selftest-a","idempotency_key":"selftest-key","kind":"TASK","operator":"selftest","requested_recipe":"verify_chain","payload":{"alpha":1,"beta":2}}'
REQ_A_REORDERED='{"payload":{"beta":2,"alpha":1},"requested_recipe":"verify_chain","operator":"selftest","kind":"TASK","idempotency_key":"selftest-key","trace_id":"selftest-a","schema":"habitat.kernel.submit.request.v1"}'
REQ_B='{"schema":"habitat.kernel.submit.request.v1","trace_id":"selftest-a","idempotency_key":"selftest-key","kind":"TASK","operator":"selftest","payload":{"alpha":2}}'

FIRST="$(submit "${REQ_A}")"
REPLAY="$(submit "${REQ_A_REORDERED}")"
CONFLICT="$(submit "${REQ_B}")"

printf '%s\n' "${FIRST}" | grep -q '"verdict": "ACK_DURABLE"'
printf '%s\n' "${FIRST}" | grep -q '"event_hash": "sha256:'
printf '%s\n' "${FIRST}" | grep -q '"integration_state": "INTEGRATED"'
printf '%s\n' "${FIRST}" | grep -q '"run_id": "run_'
printf '%s\n' "${FIRST}" | grep -q '"result_event_id": "evt_'
printf '%s\n' "${REPLAY}" | grep -q '"idempotency": "REPLAY"'
printf '%s\n' "${CONFLICT}" | grep -q '"verdict": "NACK"'
printf '%s\n' "${CONFLICT}" | grep -q '"idempotency": "CONFLICT"'

ORCH_KERNEL_STATE_DIR="${STATE_DIR}" CARGO_TARGET_DIR="${TARGET_DIR}" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- verify-chain >/dev/null

SNAPSHOT="$(ORCH_KERNEL_STATE_DIR="${STATE_DIR}" CARGO_TARGET_DIR="${TARGET_DIR}" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- snapshot --json)"
printf '%s\n' "${SNAPSHOT}" | grep -q '"edge": "warrant_to_run"'
printf '%s\n' "${SNAPSHOT}" | grep -q '"edge": "run_to_result"'
printf '%s\n' "${SNAPSHOT}" | grep -q '"edge": "result_to_replay_dashboard"'

echo "orch-kernel-soak-selftest: PASS state=${STATE_DIR}"
