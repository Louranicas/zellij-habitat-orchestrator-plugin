//! Command-line control surface for the Orchestrator Kernel sidecar.

use orchestrator_kernel_sidecar::{
    parse_payload, AppendEvent, EventLog, KernelError, Result, StatePaths, SubmitRequest,
};
use serde_json::json;
use std::env;

/// Commands for which `--read-only` (a non-mutating open) is valid.
const READ_COMMANDS: [&str; 5] = ["snapshot", "snapshot-v2", "verify-chain", "replay", "events"];

fn main() {
    if let Err(err) = run() {
        eprintln!("orch-kernelctl: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return Ok(());
    };
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        print_help();
        return Ok(());
    }

    // Pull `--read-only` out of the argument list before the per-command parsers
    // see it. It opens the event log read-only (no WAL checkpoint / no writer
    // contention) and is valid ONLY for non-mutating read commands.
    let raw: Vec<String> = args.collect();
    let read_only = raw.iter().any(|arg| arg == "--read-only");
    let rest: Vec<String> = raw.into_iter().filter(|arg| arg != "--read-only").collect();

    if read_only && !READ_COMMANDS.contains(&command.as_str()) {
        return Err(KernelError::InvalidInput(format!(
            "--read-only is only valid for read commands ({}), not {command:?}",
            READ_COMMANDS.join(", ")
        )));
    }

    let paths = StatePaths::default_from_env()?;
    let log = if read_only {
        EventLog::open_read_only(&paths)?
    } else {
        EventLog::open(&paths)?
    };

    match command.as_str() {
        "init" => {
            log.initialize()?;
            println!("{}", serde_json::to_string_pretty(&log.snapshot()?)?);
        }
        "append" => {
            let request = parse_append_args(&rest)?;
            let row = log.append_event(&request)?;
            println!("{}", serde_json::to_string_pretty(&row)?);
        }
        "submit" => {
            let request = parse_submit_args(&rest)?;
            let response = log.submit(&request)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        "snapshot" => {
            ensure_json_flag(&rest)?;
            println!("{}", serde_json::to_string_pretty(&log.snapshot()?)?);
        }
        "snapshot-v2" => {
            ensure_json_flag(&rest)?;
            println!("{}", serde_json::to_string_pretty(&log.snapshot_v2()?)?);
        }
        "verify-chain" => {
            log.verify_chain()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"verify_chain_ok": true}))?
            );
        }
        "replay" => {
            let since = parse_since_arg(&rest)?;
            let rows = log.replay_since(since)?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        "events" => {
            let trace_id = parse_trace_arg(&rest)?;
            let rows = log.events_for_trace(&trace_id)?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        other => {
            return Err(KernelError::InvalidInput(format!(
                "unknown command {other:?}; run orch-kernelctl help"
            )));
        }
    }
    Ok(())
}

fn parse_submit_args(args: &[String]) -> Result<SubmitRequest> {
    if args.len() == 2 && args[0] == "--json" {
        return serde_json::from_str(&args[1]).map_err(KernelError::from);
    }
    Err(KernelError::InvalidInput(format!(
        "submit requires --json <request>, got {args:?}"
    )))
}

fn parse_append_args(args: &[String]) -> Result<AppendEvent> {
    let mut kind: Option<String> = None;
    let mut trace_id: Option<String> = None;
    let mut parent_id: Option<String> = None;
    let mut actor = "orch-kernelctl".to_string();
    let mut payload = json!({});

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--kind" => {
                kind = Some(value_after(args, index, "--kind")?);
                index += 2;
            }
            "--trace-id" => {
                trace_id = Some(value_after(args, index, "--trace-id")?);
                index += 2;
            }
            "--parent-id" => {
                parent_id = Some(value_after(args, index, "--parent-id")?);
                index += 2;
            }
            "--actor" => {
                actor = value_after(args, index, "--actor")?;
                index += 2;
            }
            "--payload" => {
                payload = parse_payload(&value_after(args, index, "--payload")?)?;
                index += 2;
            }
            other => {
                return Err(KernelError::InvalidInput(format!(
                    "unexpected append argument {other:?}"
                )));
            }
        }
    }

    let kind = kind.ok_or_else(|| KernelError::InvalidInput("append requires --kind".into()))?;
    let trace_id = trace_id.unwrap_or_else(|| format!("trace-{kind}"));

    Ok(AppendEvent {
        kind,
        trace_id,
        parent_id,
        actor,
        payload,
    })
}

fn ensure_json_flag(args: &[String]) -> Result<()> {
    if args.is_empty() || args == ["--json"] {
        Ok(())
    } else {
        Err(KernelError::InvalidInput(format!(
            "snapshot only accepts --json, got {args:?}"
        )))
    }
}

fn parse_since_arg(args: &[String]) -> Result<i64> {
    if args.is_empty() {
        return Ok(0);
    }
    if args.len() == 2 && args[0] == "--since" {
        return args[1]
            .parse::<i64>()
            .map_err(|err| KernelError::InvalidInput(format!("invalid --since value: {err}")));
    }
    Err(KernelError::InvalidInput(format!(
        "replay accepts optional --since <seq>, got {args:?}"
    )))
}

fn parse_trace_arg(args: &[String]) -> Result<String> {
    if args.len() == 2 && args[0] == "--trace" {
        Ok(args[1].clone())
    } else {
        Err(KernelError::InvalidInput(format!(
            "events requires --trace <trace_id>, got {args:?}"
        )))
    }
}

fn value_after(args: &[String], index: usize, flag: &str) -> Result<String> {
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| KernelError::InvalidInput(format!("{flag} requires a value")))
}

fn print_help() {
    println!(
        "orch-kernelctl commands:\n  init\n  submit --json REQUEST_JSON\n  append --kind KIND [--trace-id ID] [--parent-id ID] [--actor ACTOR] [--payload JSON]\n  snapshot [--json] [--read-only]\n  snapshot-v2 [--json] [--read-only]\n  verify-chain [--read-only]\n  replay [--since SEQ] [--read-only]\n  events --trace TRACE_ID [--read-only]\n\n  --read-only  open the event log read-only (no WAL checkpoint, no writer contention); read commands only"
    );
}
