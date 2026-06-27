# Soak Testing & Monitoring

> Back to: [[MOC]] · [[Diagnostics]] · [[notes/Score & Fitness Framework]]
> Source: `scripts/orch-kernel-soak.sh` · `scripts/orch-kernel-soak-selftest.sh`
> `scripts/orch-kernel-monitor.sh` · `scripts/orch-kernel-deep-trace.sh`

Four scripts form the soak/monitoring stack. They range from a 15-second
selftest (always safe) to a 2-hour deep-trace under stress.

---

## `orch-kernel-soak-selftest.sh` — 15s functional selftest

The most important correctness proof. Run in a fresh `mktemp -d` state dir;
always cleaned up on exit.

```bash
scripts/orch-kernel-soak-selftest.sh
# Output: orch-kernel-soak-selftest: PASS state=<tmpdir>
```

**What it verifies (8 assertions):**

1. `REQ_A` → `ACK_DURABLE` + `event_hash` sha256 + `integration_state=INTEGRATED` + `run_id`
2. `REQ_A_REORDERED` (canonical-JSON-equivalent) → `idempotency=REPLAY` (same canonical hash)
3. `REQ_B` (different payload, same idempotency_key) → `NACK` + `idempotency=CONFLICT`
4. `verify-chain` exits 0 on the state above
5. Snapshot contains `warrant_to_run` edge
6. Snapshot contains `run_to_result` edge
7. Snapshot contains `result_to_replay_dashboard` edge

This proves the full ACK_DURABLE → chain edge → replay flow in under 15s.

**Key design:** `REQ_A` and `REQ_A_REORDERED` have identical semantic content
but different JSON key order. The REPLAY verdict proves that canonical JSON
(recursive key-sorting) is the idempotency key's hash input, not raw bytes.

---

## `orch-kernel-soak.sh` — sustained submit loop

```bash
scripts/orch-kernel-soak.sh --profile stress_quick --duration 90
```

Submits `verify_chain` tasks in a tight loop for `DURATION` seconds. Each
task gets a unique `idempotency_key` (profile + count), so every submission
is a fresh `ACK_DURABLE`. On success the loop increments the replay counter
and re-submits the same key to verify idempotency.

**Profiles:** `stress_quick` (90s default), custom via `--profile` flag.

**Purpose:** validates that the SQLite WAL-mode event log, hash-chain append,
and idempotency table hold up under rapid repeated writes.

---

## `orch-kernel-monitor.sh` — multi-level live monitor

Designed to run **in a dedicated Zellij pane** alongside tests.

```bash
scripts/orch-kernel-monitor.sh [duration_secs=7200] [interval_secs=10]
```

**Outputs per run (timestamped):**
- `receipts/zellij-orchestrator-kernel-monitor-<stamp>.tsv` — tab-separated samples
- `receipts/zellij-orchestrator-kernel-monitor-<stamp>.jsonl` — structured events
- `receipts/zellij-orchestrator-kernel-monitor-<stamp>.summary.md` — human summary
- `receipts/zellij-orchestrator-kernel-monitor-<stamp>.artifacts/` — raw captures

**Checks per interval:**
- Sidecar health (`snapshot --json`)
- Chain integrity (`verify-chain`)
- Zellij log delta (new error lines since start)
- Process census (orch-kernelctl, zellij procs)
- Pipe latency (optional: `CHECK_PIPE=1`)
- DB tail (last few events from SQLite)

**Plugin config for orchestrator-kernel mode:**
```
modules=orchestrator_kernel,bridge_health,coherence_gauge
role=orchestrator_kernel
sidecar_cli=/home/louranicas/.local/bin/orch-kernelctl
kernel_poll=5
```

---

## `orch-kernel-deep-trace.sh` — 2h stress trace

```bash
scripts/orch-kernel-deep-trace.sh [duration_secs=7200] [interval_secs=5]
```

More intensive than `monitor.sh`. Adds:
- **Stress-watch TSV** — tracks heavy operations concurrently
- **Pipe latency TSV** — times every Zellij pipe command round-trip
- **Process TSV** — `etime/cpu/mem/args` per orch-kernelctl process
- **DB tail TSV** — last N rows of the event log per interval
- **Zellij log delta** — new lines from `zellij.log` per interval

**Output directory:** `receipts/orch-kernel-deep-trace-<stamp>/`

**When to use:** before a production readiness claim above 90, or after a
binary deploy to verify no regression under sustained load.

---

## `orch-kernel-v012-live-pipe-proof.sh` — Mode A terminal proof

Proves that valid kernel pipe JSON returns `NACK_USE_SIDECAR_SUBMIT` without
attempting synchronous sidecar CLI execution inside the Zellij `CliPipe`
window.

**What it does:**
1. Spawns a Zellij session with `habitat-plugin-v0.1.2.wasm` loaded
2. Sends a valid `habitat.kernel.submit.request.v1` payload via `zellij pipe`
3. Captures `cli_pipe_output` from Zellij log
4. Verifies `verdict=NACK_USE_SIDECAR_SUBMIT, attempted=false`

**Receipt schema:** `habitat.kernel.v012.live_pipe_proof.v1`

**v0.1.2 result:** `PASS` · sha `79470cfe…`

---

## `plugin-v011-direct-health.sh` / `plugin-v011-one-hour-telemetry.sh`

Historical scripts for v0.1.1 smoke testing. Not part of the v0.1.2 gate but
retained for wave-history reference (Wave-16 S1005032, S1007594 D11).

---

## `plugin-v012-3h-ultratest-suite.sh`

3-hour full-suite stress test combining soak + monitor + pipe proof in sequence.
Rarely run (cost: 3 hours). Use `orch-kernel-soak-selftest.sh` for routine
gates.

---

## See also

- [[Diagnostics]] — which scripts are required gate items vs optional
- [[notes/Score & Fitness Framework]] — how soak results feed the score
- [[notes/Build Deploy Rollback Pipeline]] — deploying before soaking
- [[Orchestrator Kernel Sidecar — Durable Admission Engine]] — the target under test
