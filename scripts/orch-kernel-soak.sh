#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/habitat-zellij-target}"
STATE_DIR="${ORCH_KERNEL_STATE_DIR:-$(mktemp -d)}"
PROFILE="stress_quick"
DURATION="90"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    --duration)
      DURATION="$2"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

cd "${ROOT}"
START="$(date +%s)"
END="$((START + DURATION))"
COUNT=0
REPLAYS=0

submit() {
  ORCH_KERNEL_STATE_DIR="${STATE_DIR}" CARGO_TARGET_DIR="${TARGET_DIR}" \
    cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- submit --json "$1"
}

while [[ "$(date +%s)" -lt "${END}" ]]; do
  KEY="${PROFILE}-${COUNT}"
  REQ="{\"schema\":\"habitat.kernel.submit.request.v1\",\"trace_id\":\"soak-${PROFILE}\",\"idempotency_key\":\"${KEY}\",\"kind\":\"TASK\",\"operator\":\"orch-kernel-soak\",\"requested_recipe\":\"verify_chain\",\"payload\":{\"profile\":\"${PROFILE}\",\"count\":${COUNT}}}"
  OUT="$(submit "${REQ}")"
  printf '%s\n' "${OUT}" | grep -q '"verdict": "ACK_DURABLE"'
  printf '%s\n' "${OUT}" | grep -q '"event_hash": "sha256:'
  printf '%s\n' "${OUT}" | grep -q '"integration_state": "INTEGRATED"'

  if [[ $((COUNT % 5)) -eq 0 ]]; then
    REPLAY_OUT="$(submit "${REQ}")"
    printf '%s\n' "${REPLAY_OUT}" | grep -q '"idempotency": "REPLAY"'
    REPLAYS="$((REPLAYS + 1))"
  fi

  COUNT="$((COUNT + 1))"
done

ORCH_KERNEL_STATE_DIR="${STATE_DIR}" CARGO_TARGET_DIR="${TARGET_DIR}" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- verify-chain >/dev/null

SNAPSHOT="$(ORCH_KERNEL_STATE_DIR="${STATE_DIR}" CARGO_TARGET_DIR="${TARGET_DIR}" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- snapshot --json)"

echo "orch-kernel-soak: PASS profile=${PROFILE} duration=${DURATION}s submissions=${COUNT} replays=${REPLAYS} state=${STATE_DIR}"
printf '%s\n' "${SNAPSHOT}"
