# Zellij Habitat Orchestrator Plugin

**A Zellij WASM dashboard and command surface for the Habitat orchestrator: field
state, fleet health, campaign attention, fiber cockpit, sphere warden, and
durable orchestrator-kernel admission in one terminal-native plugin.**

Release name: `zellij-habitat-orchestrator-plugin`

Version: `0.1.2`

Repositories:

- GitHub: <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin>
- GitLab: <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin>

This documentation follows the Deep-Diff-Forge pattern: state the invariant,
show the user path early, keep the machine-verifiable gates close to the top, and
split deep operational material into linked docs. See the exemplar at
<https://github.com/Louranicas/deep-diff-forge>.

---

## Table Of Contents

- [Why This Plugin Exists](#why-this-plugin-exists)
- [First Principle](#first-principle)
- [What Ships In v0.1.2](#what-ships-in-v012)
- [Install And Build](#install-and-build)
- [Quick Start](#quick-start)
- [Command Surface](#command-surface)
- [Architecture](#architecture)
- [Quality And Security Gates](#quality-and-security-gates)
- [Documentation Map](#documentation-map)
- [Task Status](#task-status)
- [Release Status](#release-status)
- [License](#license)

---

## Why This Plugin Exists

Habitat has many live surfaces: ORAC, PV2, service bridges, campaign/fiber state,
governance proposals, and the v0.1.2 orchestrator-kernel sidecar. A terminal
operator needs to inspect that field without turning observation into accidental
actuation.

`zellij-habitat-orchestrator-plugin` keeps those concerns together but separated:

| Surface | Role |
| --- | --- |
| Zellij WASM plugin | Read and render live state inside a pane. |
| `habitat-core` | Shared event, config, render, and response contracts. |
| `habitat-modules` | Built-in dashboard modules with isolated state and tests. |
| `habitat-bridge-client` | Tagged `run_command` transport and snapshot fan-out. |
| `orchestrator-kernel-sidecar` | Durable admission, hash-chain replay, policy-bound warrant checks. |
| Scripts and layouts | Operator runbooks, proof harnesses, and Zellij layouts. |

The result is a plugin that is usable as a dashboard, testable as normal Rust,
and governed enough to participate in a production-oriented orchestrator without
claiming more authority than it has.

## First Principle

> **Observation must be cheap, attributed, and reversible; durable action must
> cross the sidecar boundary and return proof.**

The plugin may render, poll, serialize pane state, and forward pipe messages. It
does not invent durability. `ACK_DURABLE` belongs to the sidecar submit path only,
after event append, hash-chain update, idempotency resolution, and policy-bound
warrant checks.

This invariant is carried through:

- `cmd_pipe` remains rate-limited and operator-visible.
- Kernel pipe handling fails closed for invalid schema.
- Durable task admission is owned by `orchestrator-kernel-sidecar`.
- Built-in recipes are fixed allowlist/no-shell paths.
- Rollback and promotion scripts default to dry-run.

See [docs/SECURITY.md](docs/SECURITY.md) for the boundary details.

## What Ships In v0.1.2

| Area | Included |
| --- | --- |
| Dashboard modules | `fleet_view`, `coherence_gauge`, `bridge_health`, `event_feed`, `na_panel`, `session_timer`, `cmd_pipe`, `campaign_attention`, `fiber_cockpit`, `sphere_warden`, `orchestrator_kernel`. |
| Zellij layouts | Full fleet, compact, minimal, and factory witness layouts. |
| Sidecar | `orch-kernelctl`, `orch-kerneld`, durable event log, replay, verify-chain, idempotency, policy hash checks. |
| Proof scripts | Live pipe proof, zero-touch verifier, deep trace, monitor, fitness, score, soak, rollback, deploy dry-runs. |
| Test coverage | 365 Rust tests in the standalone export at release time. |
| Release tag | `v0.1.2` on GitHub and GitLab. |

## Install And Build

Requires Rust with `wasm32-wasip1` target support and Zellij `0.43.x` plugin API
compatibility.

```bash
git clone https://github.com/Louranicas/zellij-habitat-orchestrator-plugin.git
cd zellij-habitat-orchestrator-plugin

cargo check --workspace
cargo test --workspace

# Build and install the WASM plugin to ~/.config/zellij/plugins/habitat-plugin.wasm
bash build.sh
```

The build script compiles `crates/habitat-plugin` for `wasm32-wasip1`, writes the
plugin artifact to `~/.config/zellij/plugins/habitat-plugin.wasm`, and asks
Zellij to reload it when a session is available.

## Quick Start

Launch a full dashboard tab:

```bash
zellij --layout ./layouts/habitat-fleet.kdl
```

Use the compact or minimal layouts for a smaller pane footprint:

```bash
zellij --layout ./layouts/habitat-compact.kdl
zellij --layout ./layouts/habitat-minimal.kdl
```

Run a read-only release proof bundle:

```bash
scripts/orch-kernel-v012-zero-touch-verify.sh
```

Run sidecar-focused tests:

```bash
cargo test -p orchestrator-kernel-sidecar
cargo test --lib -p habitat-modules -p orchestrator-kernel-sidecar
```

## Command Surface

Zellij pipe examples:

```bash
# Context snapshot, rate-limited by cmd_pipe
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n snapshot

# Sphere detail query
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n query -- "sphere-alpha"

# Status dump
zellij pipe -p "file:$HOME/.config/zellij/plugins/habitat-plugin.wasm" -n status
```

Sidecar submit smoke test:

```bash
ORCH_KERNEL_STATE_DIR="$(mktemp -d)" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- \
  submit --json '{"schema":"habitat.kernel.submit.request.v1","trace_id":"manual-smoke","idempotency_key":"manual-smoke-1","kind":"TASK","operator":"manual","payload":{"goal":"smoke"}}'
```

Expected sidecar response:

- `verdict` is `ACK_DURABLE`.
- `event_id` is present.
- `event_hash` starts with `sha256:`.
- `integration_state` is `INGESTED` or stronger.

## Architecture

```text
zellij-habitat-orchestrator-plugin/
├── Cargo.toml
├── crates/
│   ├── habitat-core/                 # contracts, config, render helpers
│   ├── habitat-bridge-client/        # run_command bridge and snapshot fan-out
│   ├── habitat-modules/              # dashboard modules
│   ├── habitat-plugin/               # WASM entrypoint and pipe handling
│   └── orchestrator-kernel-sidecar/   # durable admission and replay
├── layouts/                          # Zellij launch layouts
├── scripts/                          # proof, deploy, rollback, monitor harnesses
├── docs/                             # architecture, operations, security, release docs
└── TESTING.md                        # local testing runbook
```

The detailed map is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Quality And Security Gates

Release-local gates used for the standalone v0.1.2 repository:

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo audit
cargo deny check
```

`cargo audit` and `cargo deny check` currently exit successfully. They report
inherited warnings from the Zellij dependency chain and broad allowlist policy;
these are documented in [docs/SECURITY.md](docs/SECURITY.md).

Deep operational verification:

```bash
scripts/orch-kernel-v012-live-pipe-proof.sh
scripts/orch-kernel-v012-zero-touch-verify.sh
scripts/orch-kernel-soak-selftest.sh
scripts/orch-kernel-rollback.sh --dry-run
```

See [docs/TESTING.md](docs/TESTING.md) and [TESTING.md](TESTING.md).

## Documentation Map

| Document | Purpose |
| --- | --- |
| [docs/INDEX.md](docs/INDEX.md) | Documentation hub and bidirectional link index. |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Module, sidecar, transport, and data-flow architecture. |
| [docs/OPERATIONS.md](docs/OPERATIONS.md) | Build, launch, pipe, deploy, rollback, and proof workflows. |
| [docs/TESTING.md](docs/TESTING.md) | Gate matrix and expected verification outputs. |
| [docs/SECURITY.md](docs/SECURITY.md) | Admission boundary, policy gates, supply-chain notes. |
| [docs/RELEASE.md](docs/RELEASE.md) | v0.1.2 release identity, remotes, tag, and publish checklist. |
| [docs/TASK_STATUS.md](docs/TASK_STATUS.md) | Plan reconciliation, completed work, and remaining gated tasks. |
| [PLAN.md](PLAN.md) | Historical plan of record retained for context. |
| [TESTING.md](TESTING.md) | Original local orchestrator-kernel testing runbook. |
| [NOTES.md](NOTES.md) | Durable lessons and active reminders. |

Every file in `docs/` links back to this README and to [docs/INDEX.md](docs/INDEX.md).

## Task Status

The current task ledger is [docs/TASK_STATUS.md](docs/TASK_STATUS.md). It marks
the standalone v0.1.2 source release, documentation, link map, and Rust gates as
complete, and separates the remaining operator-gated production items from
repository work.

## Release Status

Current release:

- Commit: `2a32442d51d20f262b58f993bd2c1cddd2acdcf1`
- Tag: `v0.1.2`
- GitHub: <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin>
- GitLab: <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin>

The repo is release-ready as a source distribution. Production arming,
promotion, service restart, rollback execution, and long-running soak remain
operator-gated workflows, not implicit side effects of cloning or building this
repository.

## License

Workspace package license: `MIT OR Apache-2.0`.
