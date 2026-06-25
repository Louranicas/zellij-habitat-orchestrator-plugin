# Release

Back to [README](../README.md) · [Docs index](INDEX.md)

## v0.1.2 Identity

| Field | Value |
| --- | --- |
| Name | Zellij Habitat Orchestrator Plugin |
| Repo slug | `zellij-habitat-orchestrator-plugin` |
| Version | `0.1.2` |
| Tag | `v0.1.2` |
| Initial commit | `2a32442d51d20f262b58f993bd2c1cddd2acdcf1` |
| GitHub | <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin> |
| GitLab | <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin> |

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
git tag -a v0.1.2 -m "Zellij Habitat Orchestrator Plugin v0.1.2"
git push github main v0.1.2
git push gitlab main v0.1.2
```

Do not treat publishing as deployment. Deployment, rollback execution, and
production soak remain separate operator-gated workflows. See
[Operations](OPERATIONS.md), [Security](SECURITY.md), and
[Task Status](TASK_STATUS.md).
