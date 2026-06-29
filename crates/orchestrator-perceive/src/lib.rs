//! `orchestrator-perceive` is the L1 perception manifest assembler for the
//! Ultimate Zellij Orchestrator.
//!
//! It observes the live habitat (panes, sessions, engine health, the callable
//! catalog, leases, and fibers), folds those observations into a single typed
//! [`manifest::PerceiveSnapshot`] (schema [`SCHEMA`]), and appends that snapshot
//! to the orchestrator-kernel spine as a warrant-free `perceive.snapshot`
//! observation event. The cortex then routes from the perceived state instead of
//! a memorized catalog.
//!
//! # Design seams
//!
//! Every interaction with an external process (`zellij`, `curl`, `just`,
//! `kv-lease`, `hopf-anchor`, `orch-kernelctl`) flows through the single
//! [`exec::CommandRunner`] trait. Production code uses [`exec::SystemRunner`];
//! tests inject a deterministic fake. This keeps the assembler pure and densely
//! testable without a live habitat.
//!
//! # Scaffold convention (read before implementing)
//!
//! This crate is an architect-authored skeleton. The data contract is complete:
//! [`error`] and [`manifest`] (types and bounded newtypes) are foundation and
//! must not be reshaped without an architecture change. Every behavioural
//! function returns [`error::PerceiveError::NotImplemented`] and carries
//! underscore-prefixed parameters; an implementing fiber removes the underscore,
//! writes the real body, and adds the test suite. No function uses `todo!()`,
//! `unwrap()`, `expect()`, or `unsafe`.
//!
//! # Gold-standard bar (enforced by the gate, judged outside the build loop)
//!
//! `#![forbid(unsafe_code)]`; no `unwrap`/`expect` outside `#[cfg(test)]`;
//! `# Errors` on every public fallible function; bounded newtypes over
//! primitives; pedantic-clean. Test modules opt out of the unwrap/expect denials
//! with `#![allow(clippy::unwrap_used, clippy::expect_used)]` at the top of the
//! `#[cfg(test)] mod tests` block.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![warn(missing_docs)]
// `error::PerceiveError` deliberately echoes its module name; this is the
// conventional Rust error-module shape. Justified architect allow (S1008937);
// remove if the error type is relocated out of `error`.
#![allow(clippy::module_name_repetitions)]

pub mod assemble;
pub mod catalog;
pub mod cli;
pub mod emit;
pub mod engines;
pub mod error;
pub mod exec;
pub mod fibers;
pub mod leases;
pub mod manifest;
pub mod panes;

pub use error::{PerceiveError, Result};
pub use manifest::{PerceiveSnapshot, SCHEMA};
