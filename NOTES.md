# habitat-zellij Durable Lessons

## 2026-06-25 - Orchestrator Kernel Admission Boundary

Lesson: durable task admission belongs in `orchestrator-kernel-sidecar`; Zellij pipe handling must not claim durability or append through an async bridge path.

Evidence: `crates/orchestrator-kernel-sidecar/src/lib.rs` owns `SubmitRequest`/`SubmitResponse` and idempotency; `crates/habitat-plugin/src/main.rs` returns `NACK` with `USE_SIDECAR_SUBMIT` for valid kernel pipe JSON.

Affected files/symbols/commands: `EventLog::submit`, `HabitatDashboard::handle_kernel_pipe`, `cargo test --lib -p habitat-modules -p orchestrator-kernel-sidecar`.

Status: active.
