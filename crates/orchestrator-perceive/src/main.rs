//! Binary entry point for `orchestrator-perceive`.
//!
//! Thin wrapper: collects argv, delegates to [`orchestrator_perceive::cli::run`],
//! and maps the typed result onto a process exit code. All real behaviour lives
//! in the library so it can be tested without spawning the binary.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::process;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match orchestrator_perceive::cli::run(&args) {
        Ok(code) => process::exit(code),
        Err(err) => {
            eprintln!("orchestrator-perceive: {err}");
            process::exit(1);
        }
    }
}
