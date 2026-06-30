# Release

Back to [README](../README.md) · [Docs index](INDEX.md)

## v0.1.3 Identity (current)

| Field | Value |
| --- | --- |
| Name | Zellij Habitat Orchestrator Plugin |
| Repo slug | `zellij-habitat-orchestrator-plugin` |
| Version | `0.1.3` |
| Tag | `v0.1.3` |
| Release commit | `831182e` — *"release: v0.1.3 — Ultimate Orchestrator perception/governance organs + witness panel (S1008937)"* |
| Synced HEAD (both remotes) | `834625f` |
| WASM sha | `c5b9cce69d1a39525efbc0ee73d2c34549b1e2cc4de7da6d02f1cf42c44f9789` |
| GitHub | <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin> |
| GitLab | <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin> |

**v0.1.3 adds**: perception organ `orchestrator-perceive`, Delegation-Capacity Governor
`dcg-admit`, read-only `orchestrator_witness` panel, `orch-kernelctl --read-only` superset.
7 crates · 12 dashboard modules · 1134 host tests · no `unsafe` code · pedantic-clean.

### v0.1.2 Identity (prior release)

| Field | Value |
| --- | --- |
| Version | `0.1.2` |
| Tag | `v0.1.2` |
| Initial commit | `2a32442d51d20f262b58f993bd2c1cddd2acdcf1` |
| WASM sha | `4dcd8c60eede6545ab2c22a4fbf5ec6c6063f9cf662aae568642a8d716db6bc7` |

## Release Checklist

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo audit
cargo deny check
scripts/orch-kernel-v012-zero-touch-verify.sh
```

For WASM artifact readiness:

```bash
bash build.sh
```

For live pipe readiness:

```bash
scripts/orch-kernel-v012-live-pipe-proof.sh
```

## Publish Steps

```bash
git tag -a v0.1.3 -m "Zellij Habitat Orchestrator Plugin v0.1.3"
git push github main v0.1.3
git push gitlab main v0.1.3
```

Do not treat publishing as deployment. Deployment, rollback execution, and
production soak remain separate operator-gated workflows. See
[Operations](OPERATIONS.md), [Security](SECURITY.md), and
[Task Status](TASK_STATUS.md).
