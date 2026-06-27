# Security & Admission Boundary

> Back to: [[MOC]] · in-repo [docs/SECURITY](../docs/SECURITY.md)

The security model is **fail-closed at the admission boundary**.

## Authority table

| Path | Authority |
|---|---|
| Dashboard polling | Read-only observation |
| Zellij pipe to plugin | Input handling + response formatting |
| Sidecar submit | Durable admission + proof |
| Deploy/rollback scripts | Dry-run by default; `--apply` requires explicit operator intent |

The plugin must not claim durable admission without a sidecar result. Invalid
schema and invalid sidecar responses render as **non-durable failure states**.

## Sidecar enforces

- append-before-ACK durability;
- event hash + event id in durable responses;
- idempotency replay for duplicate requests;
- conflict rejection for same key with different canonical request;
- policy-hash validation before recipe execution (two-way check — see
  [[Orchestrator Kernel Sidecar — Durable Admission Engine]]);
- fixed built-in recipe allowlist (one recipe: `verify_chain`);
- **no arbitrary shell or network recipe execution** — by construction.

## Fail-closed points (reject *before* any write)

1. Invalid pipe schema → `NACK_SCHEMA_INVALID`, `attempted: false`.
2. Policy-hash drift (hardcoded digest OR recomputed canonical hash mismatch).
3. Idempotency conflict (same key, different bytes) → `NACK`.
4. Unknown / unsupported `requested_recipe`.
5. Empty `trace_id` / `idempotency_key` / `operator`, or `kind != TASK`.

## Observe-only witnesses

The three D11 witnesses (`fiber_cockpit`, `campaign_attention`, `sphere_warden`)
are read-only self-pollers, **mechanically grep-gated to zero writes**.
`sphere_warden` specifically never issues `register`/`deregister` — auto-
registration is deferred pending Luke ratifying the sphere-id convention and
anti-burst discipline. See [[Dashboard Modules]].

## Supply chain

`cargo audit` + `cargo deny check` exit 0. Known **inherited** (non-blocking)
warnings from the Zellij dependency chain:

- `async-std` unmaintained (via `zellij-utils` / `zellij-tile`);
- `atty` and `proc-macro-error` warnings (same chain);
- duplicate-dependency warnings (transitive Zellij/terminal crates);
- broad `deny.toml` license allowlist entries (allowed but not encountered).

Treat any **new** vulnerability/source/license failure as a release blocker.

## Production guardrails

Cloning, building, or running the test suite performs **no** production arming,
promotion, rollback execution, service restart, lease claim, binary restore, or
long-running soak. Those require explicit operator authorization + separate
receipts.

## Open security item

- **`cmd_pipe` ANSI-injection — MEDIUM** (P0 G9 residual): command-injection is
  *not* present (firewalled by architecture), but ANSI-escape injection into the
  pipe path was flagged MEDIUM and routed to the (not-started) Phase 4 sanitizer.
  See [[Bugs & Known Issues]] and [[Task Status & Roadmap]].
