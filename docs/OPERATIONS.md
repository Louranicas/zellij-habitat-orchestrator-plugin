# Operations

Back to [README](../README.md) · [Docs index](INDEX.md)

This runbook covers local build, launch, proof, dry-run deployment, and rollback
operations. Production mutation remains operator-gated.

## Build

```bash
cargo check --workspace
cargo test --workspace
bash build.sh
```

`build.sh` installs the WASM artifact at:

```text
~/.config/zellij/plugins/habitat-plugin.wasm
```

## Launch

```bash
zellij --layout ./layouts/habitat-fleet.kdl
zellij --layout ./layouts/habitat-compact.kdl
zellij --layout ./layouts/habitat-minimal.kdl
```

## Pipe Commands

```bash
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n snapshot
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n query -- "sphere-alpha"
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n status
```

## Sidecar Smoke

```bash
ORCH_KERNEL_STATE_DIR="$(mktemp -d)" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- \
  submit --json '{"schema":"habitat.kernel.submit.request.v1","trace_id":"manual-smoke","idempotency_key":"manual-smoke-1","kind":"TASK","operator":"manual","payload":{"goal":"smoke"}}'
```

Expected: `ACK_DURABLE`, non-null `event_id`, and `sha256:` event hash.

## Proof Scripts

```bash
scripts/orch-kernel-v012-live-pipe-proof.sh
scripts/orch-kernel-v012-zero-touch-verify.sh
scripts/orch-kernel-deep-trace.sh
scripts/orch-kernel-soak-selftest.sh
```

## Deploy And Rollback Boundaries

Dry-run first:

```bash
scripts/orch-kernel-deploy.sh --dry-run
scripts/orch-kernel-rollback.sh --dry-run
```

Use `--apply` only after:

- Rust gates pass;
- live pipe proof passes;
- zero-touch verifier passes;
- rollback readiness exists;
- operator arming is explicit;
- production status is not degraded.

For release-level context, see [Release](RELEASE.md).
