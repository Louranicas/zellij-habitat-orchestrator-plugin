# Diagnostics

> Back to: [[MOC]] · in-repo [docs/TESTING](../docs/TESTING.md) · [TESTING.md](../TESTING.md)

## Gate matrix (release-local)

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace --exclude habitat-plugin   # 1134 host tests (habitat-plugin is wasm-only)
cargo audit                     # exits 0 (inherited warnings documented)
cargo deny check                # exits 0
```

### Project quality gate (host crates only — the WASM line)

```bash
cargo check  -p habitat-core -p habitat-modules -p habitat-bridge-client && \
cargo clippy -p habitat-core -p habitat-modules -p habitat-bridge-client -- -D warnings && \
cargo test --lib -p habitat-core -p habitat-modules -p habitat-bridge-client && \
./build.sh
```

`habitat-plugin` cannot run host `cargo test` (depends on `zellij_tile`,
wasm32-wasip1 only) — `build.sh` is its verification.

## Build & deploy

```bash
./build.sh    # compiles habitat-plugin for wasm32-wasip1,
              # installs ~/.config/zellij/plugins/habitat-plugin.wasm (~1.2 MB),
              # asks Zellij to hot-reload if a session is live
```

Manual: `CARGO_TARGET_DIR=/tmp/habitat-zellij-target cargo build --target
wasm32-wasip1 --release -p habitat-plugin`.

## Proof scripts (deep verification)

```bash
scripts/orch-kernel-v012-live-pipe-proof.sh      # live pipe ACK/NACK proof
scripts/orch-kernel-v012-zero-touch-verify.sh    # read-only release proof bundle
scripts/orch-kernel-deep-trace.sh                # event/edge trace
scripts/orch-kernel-soak-selftest.sh             # soak self-test
scripts/orch-kernel-rollback.sh --dry-run        # rollback readiness (dry-run)
```

## Sidecar health

```bash
orch-kernelctl snapshot --json      # status, last_seq, last_hash, event_count, verify_chain_ok
orch-kernelctl snapshot-v2 --json   # fitness score + dominant_loss + edges
orch-kernelctl verify-chain         # {"verify_chain_ok": true} or chain violation error
```

## Expected pass signals

- Submit smoke → `ACK_DURABLE`, non-null `event_id`, `sha256:` event hash.
- Pipe proof → valid `NACK_USE_SIDECAR_SUBMIT` on non-ACK, `NACK_SCHEMA_INVALID`
  on bad schema (fail-closed, `attempted: false`).
- Zero-touch verify → `PASS`, 12/12 gates (per CLAUDE.local readiness receipts).
- `snapshot_v2` fitness `0.80` nominal / `0.74` edges-missing / `0.0` chain-broken.

## Historical receipts (v0.1.2 readiness, 2026-06-26)

- `receipts/orch-kernel-v012-live-pipe-proof-*/summary.json` → PASS
- `receipts/orch-kernel-v012-zero-touch-verify-*/summary.json` → PASS, 12/12, score 90 cap 90
- `receipts/production-readiness/*.json` → `ready_for_explicit_approval`, blockers=[]

## See also

- [[Problem Solving]] — what to do when a gate is red
- [[Bugs & Known Issues]] — known non-blocking issues
