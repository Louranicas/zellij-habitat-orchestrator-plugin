# Score & Fitness Framework

> Back to: [[MOC]] · [[Diagnostics]] · [[notes/Build Deploy Rollback Pipeline]]
> Source: `scripts/orch-kernel-score.sh` · `scripts/orch-kernel-fitness.sh`
> `scripts/orch-kernel-identity.sh`

The scoring framework answers: **"how complete is the orchestrator kernel
deployment?"** It assigns a numeric score (0–100) against a required-artifact
checklist, applies an evidence-based cap, and gates certain scores on gate
status from the state vector.

---

## `orch-kernel-score.sh` — artifact scoring

Runs a series of `require_file` / `grep` checks. Each adds to `SCORE`.

```
SCORE starts at 0; CAP starts at 100.
Each check adds its weight on success.
Final answer: min(SCORE, CAP)
```

| Artifact / check | Weight |
|---|---|
| `schemas/habitat.kernel.submit.request.v1.schema.json` | +12 |
| `schemas/habitat.kernel.submit.response.v1.schema.json` | +12 |
| `schemas/habitat.kernel.identity.bundle.v1.schema.json` + `orch-kernel-identity.sh` | +8 |
| `schemas/habitat.kernel.state.vector.v1.schema.json` + state vector | +8 |
| Pipe response + snapshot v2 + stress receipt schemas (3 files) | +8 |
| Fitness config `zellij-orchestrator-kernel-fitness.v1.toml` + `orch-kernel-fitness.sh` | +8 |
| Policy warrants `zellij-orchestrator-kernel-warrants.v2.json` | +12 |
| Scorecard `zellij-orchestrator-kernel-scorecard.toml` | +10 |
| `orch-kernel-soak.sh` present | +10 |
| `orch-kernel-deploy.sh` present | +6 |
| `orch-kernel-rollback.sh` present | +6 |
| `AckDurable` found in `orchestrator-kernel-sidecar/src/lib.rs` | +12 |
| `submit_replays_same_idempotency_key` test found in lib.rs | +10 |
| `"submit"` + `"kernel_pipe_id"` both found in `habitat-plugin/src/main.rs` | +10 |

**Maximum raw score: ~132** (some weights not listed above). Cap brings to 100.

### Gate0 cap logic

```bash
# Read the state vector
GATE0_STATUS="$(jq -r '.gate0_identity_status // "missing"' state_vector.json)"
STATE_CAP="$(jq -r '.do_not_claim_above // 100' state_vector.json)"

# If state vector has a hard cap, apply it
if [[ "${STATE_CAP}" -lt "${CAP}" ]]; then CAP="${STATE_CAP}"; fi

# If gate0 identity fails, cap at 74
if [[ "${GATE0_STATUS}" != "pass" && "${CAP}" -gt 74 ]]; then CAP=74; fi
```

**v0.1.2 result:** `score=90 cap=90` — gate0 passes, `do_not_claim_above=90`.
This is the "governed cap" — the state vector sets the ceiling, not just raw
artifacts.

---

## `orch-kernel-fitness.sh` — fitness report

Reads the state vector and emits a `habitat.kernel.fitness.v1` JSON document.
Used by the zero-touch verifier and the monitoring infrastructure.

**Inputs:**
```
ORCH_KERNEL_STATE_VECTOR (env) or
  workspace/Orchestrator/blackboard/ZELLIJ_ORCH_KERNEL_S1008736_STATE_VECTOR.json
```

**Output fields:**
```json
{
  "schema": "habitat.kernel.fitness.v1",
  "created_at": "...",
  "framework": "...",
  "target_wasm": "...",
  "fitness": <from state vector>,
  "dominant_loss": "...",
  "hard_score_cap": 90,
  "next_probe": "...",
  "receipt_input": "..."
}
```

The fitness value comes directly from the state vector — it is not recomputed
here. The script wraps it in a dated envelope for receipt chaining.

**Edge coherence terms** (from state vector, mapped to fitness):
- `submit_to_event → event_to_warrant → warrant_to_run → run_to_result → result_to_replay_dashboard`
- Scored 0.0 / 0.74 / 0.80 at various stages of the hardening arc

---

## `orch-kernel-identity.sh` — identity bundle

Produces a `habitat.kernel.identity.bundle.v1` JSON document: WASM sha,
SQLite DB stats, Git ref, OS info.

**Expected WASM name:** `habitat-plugin-v0.1.2.wasm` — the script checks for
this exact name.

**Gate0 identity check:** the zero-touch verifier requires `gate0_identity_status=pass`
before awarding score above 74.

---

## `orch-kernel-policy-hash.sh` — policy hash verification

```bash
scripts/orch-kernel-policy-hash.sh [policy.json]
# Default: workspace/config/zellij-orchestrator-kernel-warrants.v2.json
```

Recomputes `sha256(canonical_json(policy with policy_hash=null))` and
compares against the stored `policy_hash` field. Returns `PASS` / `FAIL`.

```python
canonical = json.dumps(policy_without_hash_field, sort_keys=True, separators=(",", ":"))
expected = "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()
status = "PASS" if stored == expected else "FAIL"
```

This is the operator-side verification of the same two-way hash-check that
the sidecar runs at `resolve_policy_decision()`. See
[[Orchestrator Kernel Sidecar — Durable Admission Engine]] §Policy warrants.

---

## `orch-kernel-v012-zero-touch-verify.sh` — integration verifier

Orchestrates all verifiers into a single pass (12 gates):

```
G1  schema files present
G2  state vector readable
G3  identity bundle passes gate0
G4  fitness from state vector
G5  policy hash verifies
G6  soak selftest PASS
G7  live pipe proof present
G8  monitor JSONL present (if available)
G9  cargo test passes
G10 cargo clippy passes
G11 cargo fmt check passes
G12 build.sh compiles WASM
```

**v0.1.2 result:** `PASS` · score=90 · cap=90 ·
sha `ca7cee840071412cac354ec1fc668299ef182009ecde6a13f55acdd7ae5994e6`

---

## See also

- [[Diagnostics]] — complete gate matrix and proof commands
- [[Orchestrator Kernel Sidecar — Durable Admission Engine]] — policy hash origin
- [[notes/Soak Testing & Monitoring]] — the soak test the score checks for
- [[Task Status & Roadmap]] — current score history and operator-gated next steps
