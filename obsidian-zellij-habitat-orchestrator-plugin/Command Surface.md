# Command Surface

> Back to: [[MOC]] · in-repo [docs/OPERATIONS](../docs/OPERATIONS.md) · source `crates/orchestrator-kernel-sidecar/src/bin/orch-kernelctl.rs`

Three command planes: the sidecar **CLI**, the Zellij **pipe** protocol, and the
**scripts/layouts**.

## CLI — `orch-kernelctl` (8 subcommands, all emit pretty JSON)

| Command | Purpose |
|---|---|
| `init` | Initialize state + print snapshot |
| `submit --json REQUEST` | Durable task admission (core write path) |
| `append --kind K [--trace-id][--parent-id][--actor][--payload]` | Append a raw event |
| `snapshot [--json]` | Health snapshot + chain status |
| `snapshot-v2 [--json]` | Contract-shaped dashboard-truth projection (fitness/pipe/edges) |
| `verify-chain` | Walk + validate the entire hash chain |
| `replay [--since SEQ]` | Replay events after a sequence |
| `events --trace TRACE_ID` | All events for one trace |

Env: `ORCH_KERNEL_STATE_DIR` (state dir), `ORCH_KERNEL_POLICY_PATH` (policy file).

### Submit smoke test

```bash
ORCH_KERNEL_STATE_DIR="$(mktemp -d)" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- \
  submit --json '{"schema":"habitat.kernel.submit.request.v1","trace_id":"manual-smoke","idempotency_key":"manual-smoke-1","kind":"TASK","operator":"manual","payload":{"goal":"smoke"}}'
```

Expected: `verdict = ACK_DURABLE`, non-null `event_id`, `event_hash` starts
`sha256:`, `integration_state` is `INGESTED` or stronger.

`orch-kerneld` (daemon) is a **stub** — initializes state + prints a snapshot; the
long-lived UDS server is a later increment.

## Zellij pipe protocol (into the WASM plugin)

```bash
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n snapshot
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n query -- "sphere-alpha"
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n status
```

Pipe is rate-limited by the `cmd_pipe` module. Responses are a typed protocol
(`kernel_pipe.rs`, schema `habitat.kernel.pipe.response.v1`, `deadline_ms = 1000`):

| Field | Values |
|---|---|
| `mode` | `A_FAIL_CLOSED` · `B_SEALED_SYNC` |
| `verdict` | `ACK_DURABLE` · `NACK_USE_SIDECAR_SUBMIT` · `NACK_SCHEMA_INVALID` · `DEGRADED_SIDECAR_BUSY` |

**Invalid schema fails closed without attempting submission** (`attempted: false`)
— the proof bundle's key assertion. Durable admission is never claimed by the
plugin alone; it only echoes a real sidecar receipt.

## Layouts (4)

| Layout | Footprint |
|---|---|
| `habitat-fleet.kdl` | Full dashboard tab |
| `habitat-compact.kdl` | Smaller pane |
| `habitat-minimal.kdl` | Minimal pane |
| `factory-witness.kdl` | Witness pane |

```bash
zellij --layout ./layouts/habitat-fleet.kdl
```

## Scripts (20)

- **Proof:** `orch-kernel-v012-live-pipe-proof`, `…-zero-touch-verify`,
  `orch-kernel-deep-trace`, `orch-kernel-visual-proof`, `plugin-v012-3h-ultratest-suite`
- **Ops (dry-run default):** `orch-kernel-deploy`, `orch-kernel-rollback`,
  `orch-kernel-promote-persistent`, `orch-kernel-rollback-persistent`,
  `orch-kernel-monitor`, `orch-kernel-soak`, `orch-kernel-soak-selftest`
- **Scoring:** `orch-kernel-fitness`, `orch-kernel-score`, `orch-kernel-identity`,
  `orch-kernel-policy-hash`
- **Fixtures / legacy:** `capture-fixtures`, `plugin-v011-direct-health`,
  `plugin-v011-one-hour-telemetry`

Mutating ops scripts default to dry-run; `--apply` requires explicit operator
intent (see [[Security & Admission Boundary]]).

## See also

- [[Orchestrator Kernel Sidecar — Durable Admission Engine]] — what `submit`/`verify-chain` actually do
- [[Diagnostics]] — which scripts gate a release
