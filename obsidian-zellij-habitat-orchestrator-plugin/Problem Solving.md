# Problem Solving

> Back to: [[MOC]] · [[Diagnostics]] · [[Bugs & Known Issues]]

Runbook for common build / launch / sidecar failure modes.

## Build fails: `habitat-plugin` won't `cargo test`

Expected — it depends on `zellij_tile` (wasm32-wasip1 only). Test the **host
crates** instead and use `build.sh` for the plugin:

```bash
cargo test --lib -p habitat-core -p habitat-modules -p habitat-bridge-client
./build.sh
```

## Plugin renders but data is stale / blank

1. Confirm services are up: `cc-health` (path-map aware — not a hand-rolled curl loop).
2. The bridge polls via `run_command(curl …)`; if curl isn't on PATH inside the
   Zellij host env, all `DataSource`s go blank. Check the pane's environment.
3. Force refresh in-pane with `r`.

## Sidecar `submit` returns NACK

| reason | cause | fix |
|---|---|---|
| `IDEMPOTENCY_CONFLICT` | same `idempotency_key`, different canonical bytes | use a fresh key, or send identical bytes for a replay |
| schema error | `schema != habitat.kernel.submit.request.v1` or `kind != TASK` | fix the request envelope |
| policy error | `policy_hash` drift or unreadable policy file | restore `config/zellij-orchestrator-kernel-warrants.v2.json`; check `ORCH_KERNEL_POLICY_PATH` |
| `unsupported requested_recipe` | recipe other than `verify_chain` | only `verify_chain` is allowed |

## `verify-chain` reports a chain violation

The event log is append-only and hash-linked; a violation means a row was
mutated or rows are out of sequence. Do **not** edit the SQLite file directly.
Inspect with `orch-kernelctl replay --since 0` and `events --trace <id>`;
preserve the DB and capture it before any reset (forensics-first).

## Host CPU spikes after launching witness panes

This is the [[Bugs & Known Issues]] CPU-saturation storm. Triage per the RCA:

```bash
free -h
/usr/bin/ps aux | grep '[f]iber-cockpit-snapshot' | wc -l   # overlapping polls
```

Fix path: ensure the flock guard is active, lease cap is set, and the poll
cadence is 30s (not 5s). Emergency reset per `ai_docs/CPU_SATURATION_RCA_S1008517.md`.

## Zellij crashed / SIGABRT on launch

The patched binary `a4c68619` cures the 0.44.3 PTY double-panic. On recurrence:

```bash
free -h
/usr/bin/ps aux | grep '[z]ellij --server'
zellij delete-session --force <hog>
```

**Never** `cargo install zellij` (D7) — reverts the patch. Rollback binary:
`~/.cargo/bin/zellij.4dfa6d57-classC-pre.rollback`.

## Deploy / rollback

Always dry-run first; `--apply` only after gates + live-pipe proof + zero-touch
verify + rollback readiness + explicit operator arming + non-degraded status.
See [[Command Surface]] and [[Security & Admission Boundary]].
