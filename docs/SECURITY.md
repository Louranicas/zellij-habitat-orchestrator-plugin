# Security

Back to [README](../README.md) · [Docs index](INDEX.md)

The security model is fail-closed at the admission boundary.

## Boundary

| Path | Authority |
| --- | --- |
| Dashboard polling | Read-only observation. |
| Zellij pipe to plugin | Input handling and response formatting. |
| Sidecar submit | Durable admission and proof. |
| Deploy/rollback scripts | Dry-run by default; `--apply` requires explicit operator intent. |

The plugin must not claim durable admission without a sidecar result. Invalid
schema and invalid sidecar responses are rendered as non-durable failure states.

## Sidecar Rules

The sidecar enforces:

- append-before-ACK durability;
- event hash and event id in durable responses;
- idempotency replay for duplicate requests;
- conflict rejection for same key with different canonical request;
- policy hash validation before recipe execution;
- fixed built-in recipe allowlist;
- no arbitrary shell or network recipe execution.

## Supply Chain Notes

Release-local commands:

```bash
cargo audit
cargo deny check
```

Current known inherited warnings:

- `async-std` unmaintained warning through `zellij-utils` / `zellij-tile`;
- `atty` and `proc-macro-error` warnings through the same Zellij dependency chain;
- duplicate dependency warnings from transitive Zellij/terminal crates;
- broad `deny.toml` license allowlist entries that are allowed but not currently
  encountered.

These commands exit 0 in the release repo. Treat new vulnerability failures,
source failures, or license failures as release blockers.

## Production Guardrails

Production arming, promotion, rollback execution, service restart, lease claim,
binary restore, and long-running production soak are not performed by cloning,
building, or running the plugin test suite. They require explicit operator
authorization and separate receipts.
