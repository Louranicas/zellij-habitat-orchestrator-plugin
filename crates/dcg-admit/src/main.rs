//! Binary entry point for `dcg-admit`.
//!
//! Thin wrapper: collects argv, delegates to [`dcg_admit::cli::run`], and maps
//! the typed result onto a process exit code. All real behaviour lives in the
//! library so it can be tested without spawning the binary. A non-zero exit is
//! the canonical signal of a refused or faulted admission (Err-as-denial).

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::process;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dcg_admit::cli::run(&args) {
        Ok(code) => process::exit(code),
        Err(err) => {
            eprintln!("dcg-admit: {err}");
            process::exit(1);
        }
    }
}
