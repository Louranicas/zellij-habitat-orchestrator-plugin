//! Command-line surface for the `dcg-admit` binary.
//!
//! Parses arguments, builds an [`crate::admit::AdmitRequest`], constructs the
//! production seams ([`crate::exec::SystemRunner`] +
//! [`crate::arming::AtuinArmingReader`]), calls [`crate::admit::admit_write`],
//! and maps the typed result to a process exit code.
//!
//! A refusal (not armed, stale fence, or warrant denial) exits **non-zero** so
//! callers (cortex / workflows) treat Err-as-denial without parsing stdout.
//!
//! ## Flags
//!
//! | Flag | Required | Default | Effect |
//! |------|----------|---------|--------|
//! | `--resource <key>` | yes | — | Fenced resource being written |
//! | `--owner <id>` | yes | — | Lease owner presenting the write |
//! | `--fence <u64>` | yes | — | Fence the caller holds |
//! | `--trace-id <id>` | yes | — | Trace id for the spine trail |
//! | `--kind <kind>` | no | `recipe.execution` | Warrant kind |
//! | `--payload <json>` | no | `{}` | Actuation payload JSON |
//! | `--ctl-path <path>` | no | `orch-kernelctl` | `orch-kernelctl` binary path |
//! | `--atuin-path <path>` | no | `/usr/local/bin/atuin` | Absolute `atuin` path |
//! | `--dry-run` | no | false | Check guards only; do not submit |
//!
//! ## Testability
//!
//! `run` delegates argument parsing to the testable [`parse_args`] helper and
//! admission to the testable [`run_with`] helper, both of which are
//! `pub(crate)`. Tests use these entry points directly to inject fakes.

use crate::admit::{AdmitRequest, admit_write, check_guards};
use crate::arming::AtuinArmingReader;
use crate::error::DcgError;
use crate::exec::SystemRunner;
use crate::fence::Fence;
use crate::Result;

#[cfg(test)]
use crate::arming::ArmingReader;
#[cfg(test)]
use crate::exec::CommandRunner;

/// Default warrant kind when `--kind` is not supplied.
const DEFAULT_KIND: &str = "recipe.execution";
/// Default `orch-kernelctl` path when `--ctl-path` is not supplied.
const DEFAULT_CTL_PATH: &str = "orch-kernelctl";
/// Default `atuin` binary path when `--atuin-path` is not supplied.
const DEFAULT_ATUIN_PATH: &str = "/usr/local/bin/atuin";

/// Parsed and validated CLI arguments.
#[derive(Debug, PartialEq)]
pub(crate) struct CliArgs {
    pub(crate) resource: String,
    pub(crate) owner: String,
    pub(crate) fence: Fence,
    pub(crate) trace_id: String,
    pub(crate) kind: String,
    pub(crate) payload: serde_json::Value,
    pub(crate) ctl_path: String,
    pub(crate) atuin_path: String,
    pub(crate) dry_run: bool,
}

/// Parses CLI arguments into [`CliArgs`].
///
/// # Errors
/// Returns [`DcgError::Empty`] if a required argument is missing.
/// Returns [`DcgError::Parse`] if `--fence` is not a valid `u64` or
/// `--payload` is not valid JSON.
pub(crate) fn parse_args(args: &[String]) -> Result<CliArgs> {
    let mut resource: Option<String> = None;
    let mut owner: Option<String> = None;
    let mut fence_raw: Option<String> = None;
    let mut trace_id: Option<String> = None;
    let mut kind = DEFAULT_KIND.to_string();
    let mut payload_raw = "{}".to_string();
    let mut ctl_path = DEFAULT_CTL_PATH.to_string();
    let mut atuin_path = DEFAULT_ATUIN_PATH.to_string();
    let mut dry_run = false;

    let mut i = 0_usize;
    while i < args.len() {
        match args[i].as_str() {
            "--resource" => {
                resource = Some(require_value(args, i, "--resource")?);
                i += 2;
            }
            "--owner" => {
                owner = Some(require_value(args, i, "--owner")?);
                i += 2;
            }
            "--fence" => {
                fence_raw = Some(require_value(args, i, "--fence")?);
                i += 2;
            }
            "--trace-id" => {
                trace_id = Some(require_value(args, i, "--trace-id")?);
                i += 2;
            }
            "--kind" => {
                kind = require_value(args, i, "--kind")?;
                i += 2;
            }
            "--payload" => {
                payload_raw = require_value(args, i, "--payload")?;
                i += 2;
            }
            "--ctl-path" => {
                ctl_path = require_value(args, i, "--ctl-path")?;
                i += 2;
            }
            "--atuin-path" => {
                atuin_path = require_value(args, i, "--atuin-path")?;
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            other => {
                return Err(DcgError::Parse {
                    source: "cli-args",
                    detail: format!("unknown argument: {other:?}"),
                });
            }
        }
    }

    let resource = resource.ok_or(DcgError::Empty { field: "--resource" })?;
    let owner = owner.ok_or(DcgError::Empty { field: "--owner" })?;
    let trace_id = trace_id.ok_or(DcgError::Empty { field: "--trace-id" })?;
    let fence_str = fence_raw.ok_or(DcgError::Empty { field: "--fence" })?;

    let fence_val: u64 = fence_str.parse().map_err(|_| DcgError::Parse {
        source: "cli-args",
        detail: format!("--fence must be a non-negative integer, got: {fence_str:?}"),
    })?;
    let fence = Fence::new(fence_val);

    let payload: serde_json::Value =
        serde_json::from_str(&payload_raw).map_err(|e| DcgError::Parse {
            source: "cli-args",
            detail: format!("--payload is not valid JSON: {e}"),
        })?;

    Ok(CliArgs {
        resource,
        owner,
        fence,
        trace_id,
        kind,
        payload,
        ctl_path,
        atuin_path,
        dry_run,
    })
}

/// Runs admission with injected seams. The `pub(crate)` entry point for tests.
///
/// # Errors
/// Returns a [`DcgError`] if any guard or transport fails. Callers map this
/// to a non-zero exit code.
#[cfg(test)]
pub(crate) fn run_with(
    args: &[String],
    runner: &dyn CommandRunner,
    arming: &dyn ArmingReader,
) -> Result<i32> {
    let cli = parse_args(args)?;
    let req = AdmitRequest {
        resource: cli.resource,
        owner: cli.owner,
        presented_fence: cli.fence,
        kind: cli.kind,
        trace_id: cli.trace_id,
        payload: cli.payload,
    };

    if cli.dry_run {
        check_guards(runner, arming, &cli.ctl_path, &req)?;
        println!("dry-run: admission would be granted for `{}`", req.resource);
        return Ok(0);
    }

    admit_write(runner, arming, &cli.ctl_path, &req)?;
    Ok(0)
}

/// Parses CLI arguments and runs the admission gate, returning the process exit
/// code (`0` admitted; non-zero refused or faulted).
///
/// Dispatches the `width` subcommand (`dcg-admit width [flags]`) to
/// [`crate::width::run_width_command`] before the admit-gate logic runs.
///
/// Uses [`SystemRunner`] and [`AtuinArmingReader`] as the production seams.
///
/// # Errors
/// Returns a [`DcgError`] if argument parsing fails, if a required argument is
/// missing, or if admission faults at the transport level. Guard refusals
/// (not-armed / stale-fence / denied) are also surfaced as `Err` so `main`
/// can map them to a non-zero exit.
pub fn run(args: &[String]) -> Result<i32> {
    // Dispatch the `width` subcommand before parsing admit flags.
    if args.first().is_some_and(|a| a == "width") {
        return crate::width::run_width_command(args.get(1..).unwrap_or(&[]));
    }

    // Dispatch the `govern` subcommand (stub — prints config and exits 0).
    if args.first().is_some_and(|a| a == "govern") {
        return run_govern_stub(args.get(1..).unwrap_or(&[]));
    }

    let cli = parse_args(args)?;
    let runner = SystemRunner;
    let arming = AtuinArmingReader::new(SystemRunner, cli.atuin_path.clone());
    let req = AdmitRequest {
        resource: cli.resource,
        owner: cli.owner,
        presented_fence: cli.fence,
        kind: cli.kind,
        trace_id: cli.trace_id,
        payload: cli.payload,
    };

    if cli.dry_run {
        check_guards(&runner, &arming, &cli.ctl_path, &req)?;
        println!("dry-run: admission would be granted for `{}`", req.resource);
        return Ok(0);
    }

    admit_write(&runner, &arming, &cli.ctl_path, &req)?;
    Ok(0)
}

/// Runs the `dcg-admit govern` subcommand.
///
/// Prints the measure-first default governor configuration as JSON and exits
/// `0`. This stub gives operators a discoverable entry point for the D6
/// governance tier before a full CLI surface is wired. The full implementation
/// would accept `--agent-id`, `--cost`, and a command to wrap.
///
/// # Errors
/// Returns [`DcgError`] if the default configuration cannot be built (should
/// not occur — this is a logic-bug guard).
fn run_govern_stub(_args: &[String]) -> Result<i32> {
    let _cfg = crate::governor::GovernorConfig::default_config()?;
    println!(
        "{}",
        serde_json::json!({
            "subcommand": "govern",
            "status": "stub",
            "tier_a": {
                "fair_semaphore": "max_inflight=4 (measure-first)",
                "transparent_retry": "max_retries=3 (measure-first)"
            },
            "tier_b": {
                "aimd": "increase=1 decrease_factor=0.5 (measure-first)",
                "circuit_breaker": "threshold=5 half_open_timeout_ms=10000 (measure-first)",
                "agent_budget": "limit=1000000 (measure-first)",
                "congestion": "threshold=10 (measure-first)"
            }
        })
    );
    Ok(0)
}

/// Returns the value that follows `flag` in `args`, or an error if missing.
fn require_value(args: &[String], index: usize, flag: &str) -> Result<String> {
    args.get(index + 1).cloned().ok_or(DcgError::Empty {
        field: "flag value",
    })
    .and_then(|v| {
        if v.starts_with("--") {
            Err(DcgError::Parse {
                source: "cli-args",
                detail: format!("{flag} requires a value, got next flag: {v:?}"),
            })
        } else {
            Ok(v)
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::arming::ArmingState;
    use crate::exec::{CommandOutput, CommandRunner};

    // -----------------------------------------------------------------------
    // Test infrastructure
    // -----------------------------------------------------------------------

    struct FakeRunner {
        responses: Vec<CommandOutput>,
        calls: std::cell::RefCell<Vec<Vec<String>>>,
        cursor: std::cell::RefCell<usize>,
    }

    impl FakeRunner {
        fn new(responses: Vec<CommandOutput>) -> Self {
            Self {
                responses,
                calls: std::cell::RefCell::new(Vec::new()),
                cursor: std::cell::RefCell::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, argv: &[String]) -> crate::Result<CommandOutput> {
            self.calls.borrow_mut().push(argv.to_vec());
            let mut idx = self.cursor.borrow_mut();
            let out = self
                .responses
                .get(*idx)
                .cloned()
                .ok_or_else(|| DcgError::Subprocess {
                    command: "fake".to_string(),
                    detail: format!("no response at index {idx}"),
                })?;
            *idx += 1;
            Ok(out)
        }
    }

    struct FixedArming(ArmingState);
    impl ArmingReader for FixedArming {
        fn read(&self) -> crate::Result<ArmingState> {
            Ok(self.0)
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn fail(status: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    fn minimal_args() -> Vec<String> {
        ["--resource", "my.res", "--owner", "me", "--fence", "1", "--trace-id", "t-1"]
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn events_empty() -> &'static str { "[]" }
    fn submit_ok_json() -> &'static str {
        r#"{"schema":"task.submit.v1","verdict":"ACK_DURABLE","trace_id":"t-1","event_id":"evt-1","event_hash":"abc","integration_state":"admitted","idempotency":"NEW","reason":"","request_hash":"xyz"}"#
    }
    fn snapshot_json() -> &'static str { r#"{"last_seq":1,"verify_chain_ok":true}"# }
    fn append_ok_json() -> &'static str {
        r#"{"seq":2,"event_id":"evt-fence","trace_id":"dcg-fence:my.res","kind":"dcg.fence.admitted","actor":"dcg-admit","payload_json":"{}","hash":"def"}"#
    }

    fn happy_runner() -> FakeRunner {
        // 0: events (read last fence)
        // 1: snapshot (upper-bound check — new guard 2b)
        // 2: submit (warrant)
        // 3: snapshot (inside submit_actuation, for receipt seq)
        // 4: append (record fence)
        FakeRunner::new(vec![
            ok(events_empty()),
            ok(snapshot_json()),
            ok(submit_ok_json()),
            ok(snapshot_json()),
            ok(append_ok_json()),
        ])
    }

    // -----------------------------------------------------------------------
    // parse_args — required fields
    // -----------------------------------------------------------------------

    #[test]
    fn missing_resource_returns_empty_error() {
        let args: Vec<String> =
            ["--owner", "me", "--fence", "1", "--trace-id", "t"]
                .iter().map(ToString::to_string).collect();
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Empty { field: "--resource" }));
    }

    #[test]
    fn missing_owner_returns_empty_error() {
        let args: Vec<String> =
            ["--resource", "r", "--fence", "1", "--trace-id", "t"]
                .iter().map(ToString::to_string).collect();
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Empty { field: "--owner" }));
    }

    #[test]
    fn missing_fence_returns_empty_error() {
        let args: Vec<String> =
            ["--resource", "r", "--owner", "me", "--trace-id", "t"]
                .iter().map(ToString::to_string).collect();
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Empty { field: "--fence" }));
    }

    #[test]
    fn missing_trace_id_returns_empty_error() {
        let args: Vec<String> =
            ["--resource", "r", "--owner", "me", "--fence", "1"]
                .iter().map(ToString::to_string).collect();
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Empty { field: "--trace-id" }));
    }

    #[test]
    fn minimal_valid_args_parse_successfully() {
        parse_args(&minimal_args()).unwrap();
    }

    #[test]
    fn parsed_resource_matches_input() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert_eq!(cli.resource, "my.res");
    }

    #[test]
    fn parsed_owner_matches_input() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert_eq!(cli.owner, "me");
    }

    #[test]
    fn parsed_fence_matches_input() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert_eq!(cli.fence.get(), 1);
    }

    #[test]
    fn parsed_trace_id_matches_input() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert_eq!(cli.trace_id, "t-1");
    }

    // -----------------------------------------------------------------------
    // parse_args — defaults
    // -----------------------------------------------------------------------

    #[test]
    fn kind_defaults_to_recipe_execution() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert_eq!(cli.kind, "recipe.execution");
    }

    #[test]
    fn payload_defaults_to_empty_object() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert_eq!(cli.payload, serde_json::json!({}));
    }

    #[test]
    fn ctl_path_defaults_to_orch_kernelctl() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert_eq!(cli.ctl_path, "orch-kernelctl");
    }

    #[test]
    fn atuin_path_has_default() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert!(!cli.atuin_path.is_empty());
    }

    #[test]
    fn dry_run_defaults_to_false() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert!(!cli.dry_run);
    }

    // -----------------------------------------------------------------------
    // parse_args — optional flags
    // -----------------------------------------------------------------------

    #[test]
    fn kind_flag_overrides_default() {
        let mut args = minimal_args();
        args.extend(["--kind".to_string(), "custom.kind".to_string()]);
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.kind, "custom.kind");
    }

    #[test]
    fn payload_flag_overrides_default() {
        let mut args = minimal_args();
        args.extend(["--payload".to_string(), r#"{"x":1}"#.to_string()]);
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.payload["x"], 1);
    }

    #[test]
    fn ctl_path_flag_overrides_default() {
        let mut args = minimal_args();
        args.extend(["--ctl-path".to_string(), "/custom/ctl".to_string()]);
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.ctl_path, "/custom/ctl");
    }

    #[test]
    fn atuin_path_flag_overrides_default() {
        let mut args = minimal_args();
        args.extend(["--atuin-path".to_string(), "/custom/atuin".to_string()]);
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.atuin_path, "/custom/atuin");
    }

    #[test]
    fn dry_run_flag_sets_true() {
        let mut args = minimal_args();
        args.push("--dry-run".to_string());
        let cli = parse_args(&args).unwrap();
        assert!(cli.dry_run);
    }

    // -----------------------------------------------------------------------
    // parse_args — error cases
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_fence_not_a_number_returns_parse_error() {
        let args: Vec<String> =
            ["--resource", "r", "--owner", "me", "--fence", "not-a-number", "--trace-id", "t"]
                .iter().map(ToString::to_string).collect();
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "cli-args", .. }));
    }

    #[test]
    fn invalid_fence_negative_returns_parse_error() {
        let args: Vec<String> =
            ["--resource", "r", "--owner", "me", "--fence", "-1", "--trace-id", "t"]
                .iter().map(ToString::to_string).collect();
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "cli-args", .. }));
    }

    #[test]
    fn invalid_payload_not_json_returns_parse_error() {
        let mut args = minimal_args();
        args.extend(["--payload".to_string(), "not-json".to_string()]);
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "cli-args", .. }));
    }

    #[test]
    fn unknown_flag_returns_parse_error() {
        let mut args = minimal_args();
        args.push("--unknown-flag".to_string());
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "cli-args", .. }));
    }

    #[test]
    fn flag_without_value_returns_error() {
        let args: Vec<String> = vec!["--resource".to_string()];
        let err = parse_args(&args).unwrap_err();
        assert!(matches!(err, DcgError::Empty { .. }));
    }

    #[test]
    fn fence_zero_is_valid() {
        let args: Vec<String> =
            ["--resource", "r", "--owner", "me", "--fence", "0", "--trace-id", "t"]
                .iter().map(ToString::to_string).collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.fence.get(), 0);
    }

    #[test]
    fn fence_large_value_is_valid() {
        let val = u64::MAX.to_string();
        let args: Vec<String> =
            vec!["--resource".to_string(), "r".to_string(), "--owner".to_string(), "me".to_string(),
                 "--fence".to_string(), val, "--trace-id".to_string(), "t".to_string()];
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.fence.get(), u64::MAX);
    }

    // -----------------------------------------------------------------------
    // run_with — admission integration
    // -----------------------------------------------------------------------

    #[test]
    fn run_with_happy_path_returns_zero() {
        let runner = happy_runner();
        let code =
            run_with(&minimal_args(), &runner, &FixedArming(ArmingState::Armed)).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn run_with_not_armed_returns_error() {
        let runner = FakeRunner::new(vec![]);
        let err =
            run_with(&minimal_args(), &runner, &FixedArming(ArmingState::Unarmed)).unwrap_err();
        assert!(matches!(err, DcgError::NotArmed { .. }));
    }

    #[test]
    fn run_with_stale_fence_returns_stale_error() {
        // prior fence = 10, presented = 1
        let payload_str = r#"{"resource":"my.res","fence":10}"#;
        let escaped = payload_str.replace('"', "\\\"");
        let body = format!(
            r#"[{{"seq":1,"event_id":"e","trace_id":"dcg-fence:my.res","kind":"dcg.fence.admitted","actor":"dcg-admit","payload_json":"{escaped}","hash":"h"}}]"#
        );
        let runner = FakeRunner::new(vec![ok(&body)]);
        let err =
            run_with(&minimal_args(), &runner, &FixedArming(ArmingState::Armed)).unwrap_err();
        assert!(matches!(err, DcgError::StaleFence { .. }));
    }

    #[test]
    fn run_with_warrant_denied_returns_denied_error() {
        // events + snapshot(upper-bound) + submit(denied)
        let runner = FakeRunner::new(vec![ok(events_empty()), ok(snapshot_json()), fail(1, "denied")]);
        let err =
            run_with(&minimal_args(), &runner, &FixedArming(ArmingState::Armed)).unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
    }

    #[test]
    fn run_with_parse_error_in_args_propagates() {
        let bad_args: Vec<String> =
            ["--resource", "r", "--owner", "me", "--fence", "bad", "--trace-id", "t"]
                .iter().map(ToString::to_string).collect();
        let runner = FakeRunner::new(vec![]);
        let err =
            run_with(&bad_args, &runner, &FixedArming(ArmingState::Armed)).unwrap_err();
        assert!(matches!(err, DcgError::Parse { .. }));
    }

    // -----------------------------------------------------------------------
    // run_with — dry-run
    // -----------------------------------------------------------------------

    #[test]
    fn dry_run_returns_zero_without_submitting() {
        let mut args = minimal_args();
        args.push("--dry-run".to_string());
        // 2 responses: events (fence read) + snapshot (upper-bound check).
        // No submit/append — dry-run does not actuate.
        let runner = FakeRunner::new(vec![ok(events_empty()), ok(snapshot_json())]);
        let code =
            run_with(&args, &runner, &FixedArming(ArmingState::Armed)).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn dry_run_makes_only_guard_calls() {
        // Dry-run calls events (fence read) + snapshot (upper-bound) = 2.
        // It must NOT call submit or append.
        let mut args = minimal_args();
        args.push("--dry-run".to_string());
        let runner = FakeRunner::new(vec![ok(events_empty()), ok(snapshot_json())]);
        run_with(&args, &runner, &FixedArming(ArmingState::Armed)).unwrap();
        assert_eq!(runner.call_count(), 2, "dry-run must make exactly 2 calls (events + snapshot)");
    }

    #[test]
    fn dry_run_not_armed_returns_error() {
        let mut args = minimal_args();
        args.push("--dry-run".to_string());
        let runner = FakeRunner::new(vec![]);
        let err =
            run_with(&args, &runner, &FixedArming(ArmingState::Unarmed)).unwrap_err();
        assert!(matches!(err, DcgError::NotArmed { .. }));
    }

    #[test]
    fn dry_run_stale_fence_returns_stale_error() {
        let mut args = minimal_args();
        args.push("--dry-run".to_string());
        let payload_str = r#"{"resource":"my.res","fence":99}"#;
        let escaped = payload_str.replace('"', "\\\"");
        let body = format!(
            r#"[{{"seq":1,"event_id":"e","trace_id":"dcg-fence:my.res","kind":"dcg.fence.admitted","actor":"dcg-admit","payload_json":"{escaped}","hash":"h"}}]"#
        );
        let runner = FakeRunner::new(vec![ok(&body)]);
        let err =
            run_with(&args, &runner, &FixedArming(ArmingState::Armed)).unwrap_err();
        assert!(matches!(err, DcgError::StaleFence { .. }));
    }

    // -----------------------------------------------------------------------
    // CliArgs struct coverage
    // -----------------------------------------------------------------------

    #[test]
    fn cli_args_debug_contains_resource() {
        let cli = parse_args(&minimal_args()).unwrap();
        assert!(format!("{cli:?}").contains("my.res"));
    }

    #[test]
    fn cli_args_partial_eq_same() {
        let a = parse_args(&minimal_args()).unwrap();
        let b = parse_args(&minimal_args()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn cli_args_partial_eq_different_kind() {
        let a = parse_args(&minimal_args()).unwrap();
        let mut args2 = minimal_args();
        args2.extend(["--kind".to_string(), "other".to_string()]);
        let b = parse_args(&args2).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn multiple_flags_in_any_order_parsed() {
        let args: Vec<String> = [
            "--trace-id", "t-2",
            "--fence", "10",
            "--owner", "bob",
            "--resource", "res.two",
            "--kind", "custom",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.resource, "res.two");
        assert_eq!(cli.owner, "bob");
        assert_eq!(cli.fence.get(), 10);
        assert_eq!(cli.trace_id, "t-2");
        assert_eq!(cli.kind, "custom");
    }

    #[test]
    fn payload_with_nested_json_parsed_correctly() {
        let mut args = minimal_args();
        args.extend(["--payload".to_string(), r#"{"a":{"b":2}}"#.to_string()]);
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.payload["a"]["b"], 2);
    }

    #[test]
    fn require_value_missing_next_arg_returns_error() {
        let args: Vec<String> = vec!["--resource".to_string()];
        let err = require_value(&args, 0, "--resource").unwrap_err();
        assert!(matches!(err, DcgError::Empty { .. }));
    }

    #[test]
    fn require_value_next_is_another_flag_returns_parse_error() {
        let args: Vec<String> = vec!["--resource".to_string(), "--owner".to_string()];
        let err = require_value(&args, 0, "--resource").unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "cli-args", .. }));
    }

    #[test]
    fn parse_args_duplicate_resource_uses_last() {
        let args: Vec<String> = [
            "--resource", "first", "--resource", "second",
            "--owner", "me", "--fence", "1", "--trace-id", "t",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.resource, "second");
    }

    #[test]
    fn parse_args_empty_args_returns_missing_resource() {
        let err = parse_args(&[]).unwrap_err();
        assert!(matches!(err, DcgError::Empty { .. }));
    }

    #[test]
    fn parse_args_fence_u64_max_valid() {
        let args: Vec<String> = [
            "--resource", "r", "--owner", "me",
            "--fence", &u64::MAX.to_string(), "--trace-id", "t",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let cli = parse_args(&args).unwrap();
        assert_eq!(cli.fence.get(), u64::MAX);
    }

    #[test]
    fn parse_args_unknown_flag_error_mentions_flag_name() {
        let mut args = minimal_args();
        args.push("--bogus-flag".to_string());
        let err = parse_args(&args).unwrap_err();
        assert!(err.to_string().contains("bogus-flag"));
    }

    #[test]
    fn run_with_uses_ctl_path_from_args() {
        let mut args = minimal_args();
        args.extend(["--ctl-path".to_string(), "/custom/ctl".to_string()]);
        let runner = happy_runner();
        run_with(&args, &runner, &FixedArming(ArmingState::Armed)).unwrap();
        // First runner call (events) should use the custom ctl path
        let recorded = runner.calls.borrow();
        assert_eq!(recorded[0][0], "/custom/ctl");
    }

    #[test]
    fn parse_args_payload_string_value_rejected() {
        let mut args = minimal_args();
        args.extend(["--payload".to_string(), "bare-string".to_string()]);
        let err = parse_args(&args).unwrap_err();
        // "bare-string" is not valid JSON object
        assert!(matches!(err, DcgError::Parse { source: "cli-args", .. }));
    }

    #[test]
    fn run_with_happy_path_makes_no_error() {
        let runner = happy_runner();
        let result = run_with(&minimal_args(), &runner, &FixedArming(ArmingState::Armed));
        assert!(result.is_ok());
    }

    #[test]
    fn dry_run_armed_no_prior_fence_returns_ok() {
        let mut args = minimal_args();
        args.push("--dry-run".to_string());
        // events (GENESIS) + snapshot (upper-bound OK)
        let runner = FakeRunner::new(vec![ok(events_empty()), ok(snapshot_json())]);
        assert!(run_with(&args, &runner, &FixedArming(ArmingState::Armed)).is_ok());
    }
}
