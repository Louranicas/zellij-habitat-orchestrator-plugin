# Testing

Back to [README](../README.md) · [Docs index](INDEX.md)

The v0.1.3 standalone repository was verified with the following local gates.

## Standard Gates

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

Expected release-local result:

- `cargo fmt --all --check`: pass;
- `cargo check --workspace`: pass;
- `cargo test --workspace`: pass;
- 1134 Rust host tests pass across core, bridge client, modules, sidecar, perceive, and dcg-admit
  (428 dcg-admit + 204 orchestrator-perceive + 362 modules + 74 core + 42 sidecar + 24 bridge-client);
  the `habitat-plugin` crate is wasm-only and is verified by `build.sh`, not host `cargo test`.

## Security And Supply Chain

```bash
cargo audit
cargo deny check
```

Expected release-local result:

- both commands exit 0;
- inherited audit warnings are from the `zellij-tile` dependency chain;
- duplicate dependency and license allowlist warnings are non-blocking policy
  findings, not release blockers.

See [Security](SECURITY.md).

## Kernel-Focused Gates

```bash
cargo test -p orchestrator-kernel-sidecar
cargo test --lib -p habitat-modules -p orchestrator-kernel-sidecar
```

The sidecar tests cover:

- canonical JSON;
- policy hash matching;
- unknown/network/shell recipe rejection;
- idempotency replay;
- durable ACK event fields;
- snapshot contract shape;
- concurrent appends/submits preserving hash-chain validity.

## Live Proof Gates

```bash
scripts/orch-kernel-v012-live-pipe-proof.sh
scripts/orch-kernel-v012-zero-touch-verify.sh
```

Expected live pipe proof:

- valid pipe returns `ACK_DURABLE`;
- invalid JSON returns `NACK_SCHEMA_INVALID`;
- summary verdict is `PASS`.

Expected zero-touch verifier:

- schema is `habitat.kernel.v012.zero_touch_verify.v1`;
- `PASS` is reserved for all required gates passing;
- `ENV_BLOCKED`, `COVERAGE_GAP`, and `PASS_WITH_DEGRADED` are not collapsed into
  production readiness.

For the original local runbook, see [../TESTING.md](../TESTING.md).
