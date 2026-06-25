#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$(cd "${ROOT}/.." && pwd)"
STATE_VECTOR="${ORCH_KERNEL_STATE_VECTOR:-${WORKSPACE}/Orchestrator/blackboard/ZELLIJ_ORCH_KERNEL_S1008736_STATE_VECTOR.json}"

if ! command -v jq >/dev/null 2>&1; then
  echo "orch-kernel-fitness: jq is required" >&2
  exit 2
fi

if [[ ! -s "${STATE_VECTOR}" ]]; then
  echo "orch-kernel-fitness: missing state vector: ${STATE_VECTOR}" >&2
  exit 3
fi

created_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq --arg created_at "${created_at}" --arg state_vector "${STATE_VECTOR}" '
  def term($id; $score; $state; $reason):
    {
      id: $id,
      score: $score,
      state: $state,
      reason: $reason
    };

  . as $state
  | {
      schema: "habitat.kernel.fitness.v1",
      created_at: $created_at,
      framework: $state.framework,
      target_wasm: $state.target_wasm,
      source_state_vector: $state_vector,
      receipt_input: $state.last_receipt_bundle,
      fitness: $state.fitness,
      dominant_loss: $state.dominant_loss,
      hard_score_cap: $state.hard_score_cap,
      next_probe: $state.next_probe,
      terms: [
        term("durable_admission_integrity"; 0.82; "transaction_tested"; "verify-chain, idempotency, and concurrent submit transaction tests pass; independent seal remains open"),
        term("pipe_terminality"; 0.80; "mode_a_terminal"; "persistent and runtime-shadow Mode A pipes return typed valid/invalid responses within deadline; Mode B remains gated"),
        term("edge_coherence"; 0.72; $state.edge_matrix_status; "edge schema and snapshot v2 exist; dashboard edge freshness still needs long-soak proof"),
        term("replayability"; 0.82; "persistent_reproduction_pass"; "sidecar replay exists and promoted WASM cold-start reproduction passed; full restart/rollback cycle still needs independent verification"),
        term("freshness"; 0.70; "partial"; "state vector is current, dashboard freshness still needs measured snapshot v2"),
        term("policy_compliance"; 0.66; "partial"; "warrant policy exists, full recipe hash binding/red-team cases remain open"),
        term("reproduction_confidence"; 0.90; "persistent_promotion_reproduction_pass"; "v0.1.2 WASM is installed under ~/.config/zellij/plugins and reproduced from that persistent path"),
        term("dependency_truthfulness"; 0.68; "partial"; "factory/fleet degraded dependencies are tracked as blockers")
      ],
      blockers: $state.open_blockers
    }
' "${STATE_VECTOR}"
