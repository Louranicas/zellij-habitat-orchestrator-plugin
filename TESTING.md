# Orchestrator Kernel Testing

This file is the local runbook for the Zellij Orchestrator Kernel implementation described in `../ai_docs/ZELLIJ_ORCHESTRATOR_KERNEL_CODEBASE_DEPLOYMENT_FRAMEWORK_S1008735.md`.

## Focused Rust Gates

```bash
cd /home/louranicas/claude-code-workspace/habitat-zellij
CARGO_TARGET_DIR=/tmp/habitat-zellij-target cargo test -p orchestrator-kernel-sidecar
CARGO_TARGET_DIR=/tmp/habitat-zellij-target cargo test --lib -p habitat-modules -p orchestrator-kernel-sidecar
```

## Submit Contract Smoke Test

```bash
cd /home/louranicas/claude-code-workspace/habitat-zellij
ORCH_KERNEL_STATE_DIR="$(mktemp -d)" \
  CARGO_TARGET_DIR=/tmp/habitat-zellij-target \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- \
  submit --json '{"schema":"habitat.kernel.submit.request.v1","trace_id":"manual-smoke","idempotency_key":"manual-smoke-1","kind":"TASK","operator":"manual","payload":{"goal":"smoke"}}'
```

Expected response:

- `verdict` is `ACK_DURABLE`.
- `event_id` is non-null.
- `event_hash` starts with `sha256:`.
- `integration_state` is `INGESTED`.

Full built-in E2E response:

```bash
cd /home/louranicas/claude-code-workspace/habitat-zellij
ORCH_KERNEL_STATE_DIR="$(mktemp -d)" \
  CARGO_TARGET_DIR=/tmp/habitat-zellij-target \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- \
  submit --json '{"schema":"habitat.kernel.submit.request.v1","trace_id":"manual-e2e","idempotency_key":"manual-e2e-1","kind":"TASK","operator":"manual","requested_recipe":"verify_chain","payload":{"goal":"full-e2e"}}'
```

Expected response:

- `verdict` is `ACK_DURABLE`.
- `integration_state` is `INTEGRATED`.
- `run_id` is non-null.
- `result_event_id` is non-null.

## Soak Harness

```bash
cd /home/louranicas/claude-code-workspace/habitat-zellij
scripts/orch-kernel-soak-selftest.sh
scripts/orch-kernel-soak.sh --profile stress_quick --duration 60
```

The harness submits through `orch-kernelctl submit`, verifies the hash chain, and prints a final snapshot. It does not execute recipes.

## Live Zellij Pipe Proof

Use this after the WASM and sidecar CLI are installed. It opens a disposable Zellij session, loads `habitat-plugin-v0.1.3.wasm`, sends a valid `kernel` pipe and an invalid JSON pipe, captures both responses, then deletes the session.

```bash
cd /home/louranicas/claude-code-workspace/habitat-zellij
scripts/orch-kernel-v012-live-pipe-proof.sh
```

Expected response:

- `valid.json` contains `verdict=ACK_DURABLE` and `mode=B_SEALED_SYNC`.
- `invalid.json` contains `verdict=NACK_SCHEMA_INVALID` and `mode=A_FAIL_CLOSED`.
- `summary.json` contains `verdict=PASS`.

## Zero-Touch Verification Bundle

Use this to join the read-only proof surfaces into one machine-verifiable receipt. It does not promote, rollback, restart services, arm grants, or create a new Zellij proof session by default; it reads existing live-pipe/randomized/monitor receipts and runs read-only score, factory-readiness, sidecar snapshot, sidecar verify-chain, and Deep-Diff-Forge self-test probes.

```bash
cd /home/louranicas/claude-code-workspace/habitat-zellij
scripts/orch-kernel-v012-zero-touch-verify.sh
```

Expected response:

- `summary.json` uses schema `habitat.kernel.v012.zero_touch_verify.v1`.
- `verdict=PASS` only when all required gates pass.
- `ENV_BLOCKED`, `COVERAGE_GAP`, and `PASS_WITH_DEGRADED` are preserved as non-production-ready outcomes rather than being collapsed into `PASS`.

## Security Boundary Checks

- `ACK_DURABLE` is only produced by sidecar submit after event append.
- Plugin pipe calls sidecar submit and returns the sidecar response; raw append is not task admission.
- Duplicate delivery with the same idempotency key and same canonical request replays the first event.
- Same idempotency key with different canonical request returns `NACK` and no event id/hash.
- Arbitrary recipe execution is denied by `../config/zellij-orchestrator-kernel-warrants.v2.json`; only the built-in no-shell `verify_chain` recipe is allowed.
- Sidecar submit resolves the warrant policy before durable admission: `policy_ref`, `policy_version`, and the canonical policy hash must match `config/zellij-orchestrator-kernel-warrants.v2.json`, and the requested recipe must match the fixed built-in allowlist.

Verify the warrant policy hash whenever the policy changes:

```bash
cd /home/louranicas/claude-code-workspace/habitat-zellij
scripts/orch-kernel-policy-hash.sh
```

Expected response:

- `status=PASS`.
- `stored_policy_hash` equals `expected_policy_hash`.
- The hash rule is `sha256(canonical_json(policy with policy_hash=null))`.

## Deployment

Deployment scripts default to dry-run:

```bash
scripts/orch-kernel-deploy.sh --dry-run
scripts/orch-kernel-rollback.sh --dry-run
```

Use `--apply` only after the Rust gates, soak harness, score script, and operator arming are complete.
