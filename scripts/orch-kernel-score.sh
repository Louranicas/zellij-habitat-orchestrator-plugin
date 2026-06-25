#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$(cd "${ROOT}/.." && pwd)"
SCORE=0
CAP=100

require_file() {
  test -s "${WORKSPACE}/$1"
}

require_file "schemas/habitat.kernel.submit.request.v1.schema.json" && SCORE="$((SCORE + 12))"
require_file "schemas/habitat.kernel.submit.response.v1.schema.json" && SCORE="$((SCORE + 12))"
require_file "schemas/habitat.kernel.identity.bundle.v1.schema.json" && require_file "habitat-zellij/scripts/orch-kernel-identity.sh" && SCORE="$((SCORE + 8))"
require_file "schemas/habitat.kernel.state.vector.v1.schema.json" && require_file "Orchestrator/blackboard/ZELLIJ_ORCH_KERNEL_S1008736_STATE_VECTOR.json" && SCORE="$((SCORE + 8))"
require_file "schemas/habitat.kernel.pipe.response.v1.schema.json" && require_file "schemas/habitat.kernel.snapshot.v2.schema.json" && require_file "schemas/habitat.kernel.stress.receipt.v1.schema.json" && SCORE="$((SCORE + 8))"
require_file "config/zellij-orchestrator-kernel-fitness.v1.toml" && require_file "habitat-zellij/scripts/orch-kernel-fitness.sh" && SCORE="$((SCORE + 8))"
require_file "config/zellij-orchestrator-kernel-warrants.v2.json" && SCORE="$((SCORE + 12))"
require_file "config/zellij-orchestrator-kernel-scorecard.toml" && SCORE="$((SCORE + 10))"
require_file "habitat-zellij/scripts/orch-kernel-soak.sh" && SCORE="$((SCORE + 10))"
require_file "habitat-zellij/scripts/orch-kernel-deploy.sh" && SCORE="$((SCORE + 6))"
require_file "habitat-zellij/scripts/orch-kernel-rollback.sh" && SCORE="$((SCORE + 6))"
grep -q '"submit"' "${ROOT}/crates/habitat-plugin/src/main.rs" && grep -q "kernel_pipe_id" "${ROOT}/crates/habitat-plugin/src/main.rs" && SCORE="$((SCORE + 10))"
grep -q "AckDurable" "${ROOT}/crates/orchestrator-kernel-sidecar/src/lib.rs" && SCORE="$((SCORE + 12))"
grep -q "submit_replays_same_idempotency_key" "${ROOT}/crates/orchestrator-kernel-sidecar/src/lib.rs" && SCORE="$((SCORE + 10))"

STATE_VECTOR="${WORKSPACE}/Orchestrator/blackboard/ZELLIJ_ORCH_KERNEL_S1008736_STATE_VECTOR.json"
if [[ -s "${STATE_VECTOR}" ]] && command -v jq >/dev/null 2>&1; then
  GATE0_STATUS="$(jq -r '.gate0_identity_status // "missing"' "${STATE_VECTOR}")"
  STATE_CAP="$(jq -r '.do_not_claim_above // 100' "${STATE_VECTOR}")"
  if [[ "${STATE_CAP}" =~ ^[0-9]+$ && "${STATE_CAP}" -lt "${CAP}" ]]; then
    CAP="${STATE_CAP}"
  fi
  if [[ "${GATE0_STATUS}" != "pass" && "${CAP}" -gt 74 ]]; then
    CAP=74
  fi
else
  CAP=74
fi

RAW_SCORE="${SCORE}"
if [[ "${SCORE}" -gt "${CAP}" ]]; then
  SCORE="${CAP}"
fi

VERDICT="REQUEST_CHANGES"
if [[ "${SCORE}" -ge 90 && "${CAP}" -ge 90 ]]; then
  VERDICT="READY_FOR_INDEPENDENT_VERIFY"
fi

if [[ "${SCORE}" -gt 100 ]]; then
  SCORE=100
fi

printf '{"schema":"habitat.kernel.score.v1","framework":"ai_docs/ZELLIJ_ORCHESTRATOR_KERNEL_DEPLOYMENT_FRAMEWORK_S1008736.md","target_wasm":"habitat-plugin-v0.1.2.wasm","raw_score":%s,"score":%s,"hard_score_cap":%s,"verdict":"%s","independent_verifier_required":true}\n' "${RAW_SCORE}" "${SCORE}" "${CAP}" "${VERDICT}"
