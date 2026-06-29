# Zellij Habitat Orchestrator Plugin

**A terminal-native orchestrator for the Habitat: a Zellij WASM dashboard (the
*body*), a durable hash-chained admission sidecar (the *spine*), and a
perception + delegation-governance layer that lets agents and operators *see* the
field and *act* on it without ever confusing observation with actuation.**

Release name: `zellij-habitat-orchestrator-plugin`

Version: `0.1.3`

Repositories:

- GitHub: <https://github.com/Louranicas/zellij-habitat-orchestrator-plugin>
- GitLab: <https://gitlab.com/lukeomahoney/zellij-habitat-orchestrator-plugin>

This documentation follows the **Deep-Diff-Forge** pattern: state the invariant
first, show the user path early, keep the machine-verifiable gates and exact
counts near the top, give one precise reference block per binary, and split deep
operational material into linked docs. Exemplar:
<https://github.com/Louranicas/deep-diff-forge>.

---

## Table of contents

- [Why this plugin exists](#why-this-plugin-exists)
- [The first principle](#the-first-principle)
- [Three organs](#three-organs)
- [What ships in v0.1.3](#what-ships-in-v013)
- [Architecture & layer map](#architecture--layer-map)
- [Install & build](#install--build)
- [Quick start](#quick-start)
- [Crates & modules](#crates--modules)
- [Command reference](#command-reference)
  - [`orch-kernelctl` — durable admission & read-only witness](#orch-kernelctl--durable-admission--read-only-witness)
  - [`orchestrator-perceive` — L1 perception assembler](#orchestrator-perceive--l1-perception-assembler)
  - [`dcg-admit` — L2/L3 consent, fence & delegation governor](#dcg-admit--l2l3-consent-fence--delegation-governor)
  - [Plugin pipes — dashboard command surface](#plugin-pipes--dashboard-command-surface)
- [Output formats & schemas](#output-formats--schemas)
- [Exit codes & verdicts](#exit-codes--verdicts)
- [Quality & security gates](#quality--security-gates)
- [Documentation map](#documentation-map)
- [Release status](#release-status)
- [License](#license)

---

## Why this plugin exists

The Habitat has many live surfaces — ORAC, PV2, service bridges, campaign/fiber
state, kv-leases, governance proposals — and a growing population of *agents*
(Claude Code instances, looms, fleets) that read and act on them concurrently. A
single operator, or a single agent, needs to inspect that field without turning
observation into accidental actuation, and to delegate work without two actors
colliding on the same resource.

`zellij-habitat-orchestrator-plugin` keeps those concerns together but strictly
separated: render and *perceive* state cheaply and reversibly; route *actuation*
through one durable, policy-bound, hash-chained boundary that returns proof. The
result is usable as a dashboard, testable as ordinary host Rust, and governed
enough to coordinate many actors without claiming more authority than it has.

## The first principle

> **Observation must be cheap, attributed, and reversible; durable action must
> cross the sidecar boundary and return proof; a denial is an error, not a
> success with a sad payload.**

Concretely:

- **Perception** (`orchestrator-perceive`, the plugin's witness modules) appends
  to the event log with `orch-kernelctl append` — no consent required, no
  durability invented. Reads can use a genuinely non-mutating `--read-only` open.
- **Actuation** (`orch-kernelctl submit`, `dcg-admit`) is warrant-gated. The
  sidecar earns `ACK_DURABLE` only *after* event append, hash-chain update,
  idempotency resolution, and policy-bound warrant checks.
- **Denials are `Err`, not `Nack`.** A refused admission (not armed, stale fence,
  warrant rejected) returns a non-zero exit and a typed error — it is never a
  zero-exit "success" carrying a refusal in the body. Fail-closed, every guard.

This invariant is carried structurally: `cmd_pipe` stays rate-limited and
operator-visible; kernel-pipe handling fails closed on invalid schema; built-in
recipes are fixed allowlist / no-shell paths; the read-only open physically
cannot write (`SQLITE_OPEN_READ_ONLY`, no WAL checkpoint); promotion/rollback
scripts default to dry-run. See [docs/SECURITY.md](docs/SECURITY.md).

## Three organs

The orchestrator is three loosely-coupled organs that meet only through the
medium (the hash-chain + kv-leases), never through shared memory:

| Organ | What it is | Where it lives |
| --- | --- | --- |
| **Body** | The Zellij WASM dashboard — renders the live field, hosts the witness modules, forwards rate-limited pipes. | `habitat-plugin`, `habitat-modules`, `habitat-core`, `habitat-bridge-client` |
| **Spine** | The durable admission sidecar — append-only hash-chained event log, replay, verify-chain, idempotency, policy-bound warrants. | `orchestrator-kernel-sidecar` (`orch-kernelctl`, `orch-kerneld`) |
| **Perception + governance** | Assembles a typed snapshot of the field (panes/engines/leases/fibers/catalog) and governs delegation (consent, fence, fair-share width). | `orchestrator-perceive`, `dcg-admit` |

## What ships in v0.1.3

`v0.1.3` adds the perception and delegation-governance organs on top of the
v0.1.2 dashboard + sidecar, and makes the sidecar a non-mutating-read superset.

| Area | Included |
| --- | --- |
| **Perception (new)** | `orchestrator-perceive` — assembles `perceive.snapshot` from panes, engines, kv-leases, hopf fibers, and the workflow/agent catalog; emits it via `orch-kernelctl append`. Dual input: self-assembled, or `--emit-from-body` reading the plugin-written body snapshot. |
| **Delegation governor (new)** | `dcg-admit` — the Delegation-Capacity Governor: a 4-guard admission (arming → fence lower-bound → fence upper-bound → warrant), append-only saga compensation, a fair FIFO semaphore with transparent retry / AIMD / circuit-breaker / budget, and a `width = min(semaphore, model-tier, budget, antichain)` ceiling. |
| **Witness (new)** | `orchestrator_witness` dashboard module — read-only governance panel rendering perceive / kernel / width / arming / route state with STALE detection; wired into the plugin's `orchestrator` role surface. |
| **Sidecar superset** | `orch-kernelctl` gains `--read-only` (a genuinely non-mutating open for the 5 read commands) **and** P1 surfacing (`latest_event_of_kind`, `latest_perceive`, dotted-namespace event kinds). |
| **Dashboard modules** | `fleet_view`, `bridge_health`, `coherence_gauge`, `event_feed`, `na_panel`, `session_timer`, `cmd_pipe`, `campaign_attention`, `fiber_cockpit`, `sphere_warden`, `orchestrator_kernel`, `orchestrator_witness`. |
| **Zellij layouts** | Full-fleet, compact, minimal, and factory-witness layouts. |
| **Test coverage** | **1134 host tests**, `--all-targets`, pedantic-clean, `forbid(unsafe_code)`. |
| **Release tag** | `v0.1.3` on GitHub and GitLab. |

## Architecture & layer map

```text
zellij-habitat-orchestrator-plugin/
├── Cargo.toml                          # workspace, version 0.1.3 (inherited by all members)
├── crates/
│   ├── habitat-core/                   # HabitatModule trait, events, config, render, responses
│   ├── habitat-bridge-client/          # run_command(curl) transport + command_sources self-poll
│   ├── habitat-modules/                # 12 dashboard modules (incl. orchestrator_witness)
│   ├── habitat-plugin/                 # WASM entrypoint + kernel_pipe handling
│   ├── orchestrator-kernel-sidecar/    # durable admission, replay, verify-chain (+ --read-only)
│   ├── orchestrator-perceive/          # L1 perception assembler → perceive.snapshot
│   └── dcg-admit/                      # L2/L3 consent + fence + delegation governor + width
├── layouts/                            # Zellij launch layouts
├── scripts/                            # proof, deploy, rollback, monitor, soak harnesses
├── docs/                               # architecture, operations, security, release docs
└── build.sh                            # build + deploy habitat-plugin.wasm
```

The organs realize a layered control plane (the medium couples them; each layer
adds exactly one capability):

| Layer | Capability | Realized by |
| --- | --- | --- |
| **L0 Spine** | Durable append-only hash-chained truth | `orchestrator-kernel-sidecar` |
| **L1 Perception** | Cheap, attributed, reversible field snapshot | `orchestrator-perceive` + witness modules |
| **L2 Consent + fence** | Warrant gate + kv-lease fence (no two actors collide) | `dcg-admit` (arming, fence, warrant) |
| **L3 Router + DCG** | Triviality gate (D0) → two-stage routing (D3) → fan-out width | `dcg-admit` (governor, width) |
| **L4 Catalog-as-organ** | The workflow/agent catalog as perceived state | `orchestrator-perceive::catalog` |
| **L5 Governor + witness** | Fair-share scheduling + observe-only governance panel | `dcg-admit::governor` + `orchestrator_witness` |

Full data-flow map: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Install & build

Requires Rust with the `wasm32-wasip1` target and Zellij `0.43.x` plugin-API
compatibility.

```bash
git clone https://github.com/Louranicas/zellij-habitat-orchestrator-plugin.git
cd zellij-habitat-orchestrator-plugin

# Host crates (everything except the wasm-only plugin crate):
cargo test -p habitat-core -p habitat-modules -p habitat-bridge-client \
           -p orchestrator-kernel-sidecar -p orchestrator-perceive -p dcg-admit

# Build + install the WASM plugin to ~/.config/zellij/plugins/habitat-plugin.wasm
bash build.sh
```

> **Why not `cargo test --workspace`?** `habitat-plugin` depends on `zellij_tile`,
> which compiles only for `wasm32-wasip1` — it cannot build for the host target.
> Tests live in the six host crates; the plugin crate is verified by `build.sh`
> (a clean `wasm32-wasip1` release build). This is the load-bearing constraint of
> the repo.

## Quick start

```bash
# Launch the full dashboard
zellij --layout ./layouts/habitat-fleet.kdl
# …or a smaller footprint
zellij --layout ./layouts/habitat-compact.kdl
zellij --layout ./layouts/habitat-minimal.kdl

# Initialise the sidecar event log, then read it without mutating it
orch-kernelctl init
orch-kernelctl snapshot --read-only --json

# Assemble + emit one perception snapshot (dry-run prints, no append)
orchestrator-perceive --dry-run

# Read the current fan-out width ceiling
dcg-admit width
```

## Crates & modules

| Crate | Binary | Role | Tests |
| --- | --- | --- | --- |
| `habitat-core` | — | `HabitatModule` trait, `CommandSource`, events, config (`ConfigWarning`), render, responses | 74 |
| `habitat-bridge-client` | — | `run_command(curl)` transport + `command_sources()` self-poll fan-out | 24 |
| `habitat-modules` | — | 12 dashboard modules with isolated state | 362 |
| `habitat-plugin` | `habitat_plugin` (wasm) | `ZellijPlugin` impl, pipe + kernel-pipe handling | (wasm build) |
| `orchestrator-kernel-sidecar` | `orch-kernelctl`, `orch-kerneld` | durable admission, replay, verify-chain, `--read-only` | 42 |
| `orchestrator-perceive` | `orchestrator-perceive` | L1 perception assembler → `perceive.snapshot` | 204 |
| `dcg-admit` | `dcg-admit` | L2/L3 consent + fence + delegation governor + width | 428 |

Dashboard modules (`habitat-modules`): `fleet_view`, `bridge_health`,
`coherence_gauge`, `event_feed`, `cmd_pipe`, `na_panel`, `session_timer`,
`fiber_cockpit`, `campaign_attention`, `sphere_warden`, `orchestrator_kernel`,
`orchestrator_witness`. The three D11 witnesses and `orchestrator_witness` are
read-only self-pollers — they render, never register or write. Module-level
detail: [CLAUDE.md](CLAUDE.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Command reference

### `orch-kernelctl` — durable admission & read-only witness

The control surface for the spine. State dir from `ORCH_KERNEL_STATE_DIR`.

| Command | Mutating? | Purpose |
| --- | --- | --- |
| `init` | yes | Create / initialise the event log (schema + genesis). |
| `append --kind <K> --trace-id <T> [--actor A] [--json P]` | yes | Append an event (perception path). |
| `submit --json <REQUEST>` | yes | **Warrant-gated** durable task admission. |
| `snapshot [--json]` | no | Current head, event count, chain status. |
| `snapshot-v2 [--json]` | no | Snapshot + fitness/sidecar blocks. |
| `verify-chain` | no | Recompute the hash chain; report integrity. |
| `replay [--since SEQ]` | no | Replay events from a sequence. |
| `events [--trace-id T]` | no | Events, optionally filtered by trace. |

`--read-only` is valid **only** for the five read commands
(`snapshot`, `snapshot-v2`, `verify-chain`, `replay`, `events`) — it opens the
log `SQLITE_OPEN_READ_ONLY` (no DDL, no WAL checkpoint, does not create a missing
DB). Passing it to `init`/`append`/`submit` is rejected with a non-zero exit
*before* the log is opened (anchored allowlist, fail-closed):

```bash
orch-kernelctl snapshot --read-only --json          # ok — non-mutating read
orch-kernelctl append --kind X --read-only          # error: --read-only is only valid for read commands
```

Submit smoke test:

```bash
ORCH_KERNEL_STATE_DIR="$(mktemp -d)" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- \
  submit --json '{"schema":"habitat.kernel.submit.request.v1","trace_id":"smoke","idempotency_key":"smoke-1","kind":"TASK","operator":"manual","payload":{"goal":"smoke"}}'
```

### `orchestrator-perceive` — L1 perception assembler

Assembles a typed `perceive.snapshot` from the live field (panes, engines,
kv-leases, hopf fibers, the workflow/agent catalog) and, unless dry-run, emits it
via `orch-kernelctl append --kind perceive.snapshot`. Perception is the cheap,
attributed, reversible half of the first principle — no consent, no durability
invented.

| Flag | Effect |
| --- | --- |
| *(none)* | Self-assemble the snapshot and append it to the event log. |
| `--dry-run` | Assemble but do not emit; print the JSON snapshot to stdout. |
| `--emit-from-body` | Use the body-written snapshot (`PaneInput::BodySnapshot`) instead of self-assembling. |
| `--body-snapshot-path <path>` | Override the body snapshot path (default `$PERCEIVE_BODY_SNAPSHOT`). |

```bash
orchestrator-perceive --dry-run                      # inspect what would be emitted
orchestrator-perceive                                # assemble + append perceive.snapshot
orchestrator-perceive --emit-from-body               # emit the plugin-assembled snapshot
```

### `dcg-admit` — L2/L3 consent, fence & delegation governor

The Delegation-Capacity Governor. Default invocation is an **admission decision**:
it gates a write behind four guards in order — *arming* (the `factory.authorize.*`
key must be `armed`), *fence lower-bound*, *fence upper-bound* (the kv-lease fence
minted from the spine sequence), then *warrant*. Any guard failing returns a
non-zero exit with a typed error (`Err`, never a zero-exit refusal). On the happy
path it admits and records an append-only saga step for compensation.

| Subcommand | Purpose |
| --- | --- |
| *(default)* `admit` | Run the 4-guard admission for a delegated write. |
| `width` | Print the current fan-out ceiling: `min(semaphore[HARD], model-tier, budget[soft], antichain[SPECULATIVE])`. |
| `govern` | Print the resolved governor config (fair-semaphore / retry / AIMD / breaker / budget). |

```bash
dcg-admit width                                      # the fan-out ceiling, computed live
dcg-admit govern                                     # resolved governor tunables
dcg-admit                                            # admission decision (fail-closed)
```

### Plugin pipes — dashboard command surface

```bash
P="file:$HOME/.config/zellij/plugins/habitat-plugin.wasm"
zellij pipe -p "$P" -n snapshot                      # context snapshot (rate-limited by cmd_pipe)
zellij pipe -p "$P" -n query -- "sphere-alpha"       # sphere detail query
zellij pipe -p "$P" -n status                        # status dump
```

In-pane keybinds: `r` force refresh · `q`/`Esc` close · per-module keys (e.g.
`coherence_gauge` `c`, `event_feed`/`fiber_cockpit` `j`/`k`).

## Output formats & schemas

- **Submit request:** `habitat.kernel.submit.request.v1` — `{schema, trace_id, idempotency_key, kind, operator, payload}`.
- **Submit response:** `habitat.kernel.submit.response.v1` — `{verdict, event_id, event_hash, integration_state, …}`.
- **Perception snapshot:** event kind `perceive.snapshot` (dotted-namespace) — the typed field manifest emitted by `orchestrator-perceive`.
- **Event hashes:** `sha256:` prefixed; the chain is recomputed by `verify-chain`.

All `--json` outputs are machine-readable and stable for agents / CI.

## Exit codes & verdicts

| Exit | Meaning |
| --- | --- |
| `0` | Success — `snapshot`/`verify-chain` ok, `submit` returned `ACK_DURABLE`, admission granted. |
| non-zero | A typed error — invalid input, schema-invalid pipe, **denied admission** (not armed / stale fence / warrant rejected), or chain failure. |

Sidecar `submit` verdicts: `ACK_DURABLE` (admitted + durable), `NACK_*`
(schema/policy refusal returned by the sidecar protocol). A `dcg-admit` denial is
reported as a process error (`Err`), not a zero-exit Nack — the observe/act and
denial-is-error invariants made executable.

## Quality & security gates

```bash
# Host crates — the authoritative gate (note --all-targets, not scoped --lib)
cargo fmt --all --check
cargo check  -p habitat-core -p habitat-modules -p habitat-bridge-client \
             -p orchestrator-kernel-sidecar -p orchestrator-perceive -p dcg-admit --all-targets
cargo clippy -p habitat-core -p habitat-modules -p habitat-bridge-client \
             -p orchestrator-kernel-sidecar -p orchestrator-perceive -p dcg-admit \
             --all-targets -- -D warnings -W clippy::pedantic
cargo test   -p habitat-core -p habitat-modules -p habitat-bridge-client \
             -p orchestrator-kernel-sidecar -p orchestrator-perceive -p dcg-admit --all-targets
bash build.sh                 # wasm32-wasip1 release build of habitat-plugin
cargo audit && cargo deny check
```

Standards (the Habitat gold bar, exemplified by Deep-Diff-Forge):
`#![forbid(unsafe_code)]`; no `unwrap`/`expect` outside `#[cfg(test)]`; ≥50
meaningful tests per module (no fitted/tautological tests); zero clippy warnings
at `-D warnings -W clippy::pedantic` on `--all-targets`. `cargo audit` /
`cargo deny` exit clean with documented inherited Zellij-chain warnings — see
[docs/SECURITY.md](docs/SECURITY.md).

Deep operational verification:

```bash
scripts/orch-kernel-v012-live-pipe-proof.sh
scripts/orch-kernel-v012-zero-touch-verify.sh
scripts/orch-kernel-soak-selftest.sh
scripts/orch-kernel-rollback.sh --dry-run
```

## Documentation map

| Document | Purpose |
| --- | --- |
| [docs/INDEX.md](docs/INDEX.md) | Documentation hub + bidirectional link index. |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Organ / layer / transport / data-flow architecture. |
| [docs/OPERATIONS.md](docs/OPERATIONS.md) | Build, launch, pipe, deploy, rollback, proof workflows. |
| [docs/TESTING.md](docs/TESTING.md) | Gate matrix and expected verification outputs. |
| [docs/SECURITY.md](docs/SECURITY.md) | Admission boundary, policy gates, supply-chain notes. |
| [docs/RELEASE.md](docs/RELEASE.md) | Release identity, remotes, tag, publish checklist. |
| [docs/TASK_STATUS.md](docs/TASK_STATUS.md) | Plan reconciliation, completed work, gated tasks. |
| [CLAUDE.md](CLAUDE.md) | Module table, constraints, build/test discipline. |
| [TESTING.md](TESTING.md) · [NOTES.md](NOTES.md) | Local runbook · durable lessons. |

Every file in `docs/` links back to this README and to
[docs/INDEX.md](docs/INDEX.md).

## Release status

- Version: `0.1.3`
- Tag: `v0.1.3` (GitHub + GitLab)
- Gate: `--all-targets` check / clippy `-D` / pedantic / test green; **1134 host
  tests**; `habitat-plugin` wasm build clean.

The repo is release-ready as a source distribution. Production arming, promotion,
service restart, rollback execution, and long-running soak remain operator-gated
workflows — never implicit side effects of cloning or building this repository.

## License

Workspace package license: `MIT OR Apache-2.0`.
