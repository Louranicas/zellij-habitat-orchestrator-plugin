//! `dcg-admit` is the L2/L3 delegated-write admission gate for the Ultimate
//! Zellij Orchestrator (DCG Phase A · P2 consent-reuse + fencing).
//!
//! Both the cortex and workflow fibers call this organ to admit a delegated
//! write to shared state. Admission enforces three independent guards before any
//! actuation reaches the world:
//!
//! 1. **Arming** — the `factory.authorize.ultimate-zellij-orchestrator` key MUST
//!    read `armed`, surfaced through the injectable [`arming::ArmingReader`]
//!    (read-never-write). Arming enforcement lives here, not in the sidecar
//!    (build-plan §8 A3); the sidecar's deny-by-default policy is the backstop.
//! 2. **Fence** — the caller's presented [`fence::Fence`] MUST strictly
//!    [`fence::Fence::supersedes`] the resource's last-admitted fence, so a stale
//!    lease-holder cannot corrupt a fenced write (the Kleppmann / Azure-Redlock
//!    result; DCG §9 falsifiable item #1). Fences are minted from the spine
//!    monotonic `seq`, never wall-clock.
//! 3. **Warrant** — the actuation is routed through the existing orchestrator-
//!    kernel warrant/policy organ via `orch-kernelctl submit`. A policy denial
//!    surfaces as a **non-zero exit / `Err`**, NOT a `SubmitVerdict::Nack` — the
//!    only `Nack` is an idempotency conflict (build-plan §8 A2). This crate
//!    therefore treats **Err-as-denial**: it inspects the exit code, it does not
//!    parse for a `Nack` verdict. There is no `consent` concept; it is a
//!    policy/warrant gate.
//!
//! Multi-step writes use [`saga`] compensation: on partial failure a compensating
//! event is **appended** — chain rows are NEVER deleted (the append-only spine
//! invariant; `verify-chain` must stay green).
//!
//! # Design seams (testable without the live key or services)
//!
//! Every interaction with an external process (`orch-kernelctl`, `atuin`,
//! `kv-lease`) flows through the single [`exec::CommandRunner`] trait, and every
//! read of the arming key flows through [`arming::ArmingReader`]. Production code
//! uses [`exec::SystemRunner`] and [`arming::AtuinArmingReader`]; tests inject
//! deterministic fakes. This keeps the admission logic pure and densely testable
//! without a live habitat, the live arming key, or a running sidecar.
//!
//! # Scaffold convention (read before implementing)
//!
//! This crate is an architect-authored skeleton. The contract surface is
//! complete and must not be reshaped without an architecture change:
//!
//! * [`error`] — the typed failure surface ([`DcgError`] / [`Result`]).
//! * [`fence`] — the bounded [`Fence`] newtype and its monotonicity guard.
//! * [`exec`] — the [`exec::CommandRunner`] subprocess seam.
//! * [`arming`] — the [`arming::ArmingReader`] seam (read-only, fully wired).
//!
//! The behavioural modules — [`warrant`], [`saga`], [`admit`], and [`cli`] — are
//! skeletons: every behavioural function returns
//! [`error::DcgError::NotImplemented`] and carries underscore-prefixed
//! parameters. An implementing fiber removes the underscore, writes the real
//! body against the documented per-fiber contract, and adds the test suite. No
//! function uses `todo!()`, `unwrap()`, `expect()`, or `unsafe`.
//!
//! # Gold-standard bar (enforced by the gate, judged outside the build loop)
//!
//! `#![forbid(unsafe_code)]`; no `unwrap`/`expect` outside `#[cfg(test)]`
//! (`#![deny(clippy::unwrap_used, clippy::expect_used)]`); `# Errors` on every
//! public fallible function; bounded newtypes over primitives; pedantic-clean
//! checked with `--all-targets` (NOT scoped `--lib`); >=50 meaningful tests per
//! behavioural module. Test modules opt out of the unwrap/expect denials with
//! `#![allow(clippy::unwrap_used, clippy::expect_used)]` at the top of the
//! `#[cfg(test)] mod tests` block.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![warn(missing_docs)]
// `error::DcgError` deliberately echoes its module name; this is the conventional
// Rust error-module shape. Justified architect allow (S1008937); remove only if
// the error type is relocated out of `error`.
#![allow(clippy::module_name_repetitions)]

pub mod admit;
pub mod arming;
pub mod cli;
pub mod error;
pub mod exec;
pub mod fence;
pub mod governor;
pub mod saga;
pub mod warrant;
pub mod width;

pub use arming::{ArmingReader, ArmingState};
pub use error::{DcgError, Result};
pub use fence::Fence;
pub use width::{BoundCeiling, Width, WidthCeilings, WidthResult, compute_width};
