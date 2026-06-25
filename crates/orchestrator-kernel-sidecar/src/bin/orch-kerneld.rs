//! Minimal daemon placeholder for the Orchestrator Kernel sidecar.
//!
//! The assimilated v1.1 plan forbids plugin autospawn. This binary therefore
//! performs explicit initialization and prints a durable snapshot; the long-lived
//! UDS server is a later P2.0 increment after the CLI substrate is sealed.

use orchestrator_kernel_sidecar::{EventLog, Result, StatePaths};

fn main() {
    if let Err(err) = run() {
        eprintln!("orch-kerneld: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let paths = StatePaths::default_from_env()?;
    let log = EventLog::open(&paths)?;
    println!("{}", serde_json::to_string_pretty(&log.snapshot()?)?);
    Ok(())
}
