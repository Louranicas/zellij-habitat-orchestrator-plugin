//! The single external-process injection seam.
//!
//! Every module that shells out to a host command (`orch-kernelctl`, `atuin`,
//! `kv-lease`) does so through the [`CommandRunner`] trait. Production code uses
//! [`SystemRunner`]; tests inject a recording fake. This keeps admission
//! deterministic and densely testable without a live habitat, the live arming
//! key, or a running sidecar.
//!
//! Architect-owned foundation: fully implemented.

use std::process::Command;

use crate::error::DcgError;
use crate::Result;

/// Captured result of running an external command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    /// Process exit status code (`0` on success).
    pub status: i32,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl CommandOutput {
    /// Returns `true` iff the process exited successfully (status `0`).
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.status == 0
    }
}

/// Abstraction over running an external command with explicit argv.
///
/// Implementors MUST treat `argv[0]` as the executable and MUST NOT route
/// through a shell (no argument splitting, no glob or variable expansion). For
/// the WASM body path callers MUST pass an absolute `argv[0]` (D11); host bins
/// may rely on `PATH`.
pub trait CommandRunner {
    /// Runs `argv` and captures its output.
    ///
    /// # Errors
    /// Returns [`DcgError::Subprocess`] if the process cannot be spawned or its
    /// output cannot be captured.
    fn run(&self, argv: &[String]) -> Result<CommandOutput>;
}

/// Production [`CommandRunner`] backed by [`std::process::Command`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    /// Runs the command at `argv[0]` with the remaining elements as arguments.
    ///
    /// No shell is invoked.
    ///
    /// # Errors
    /// Returns [`DcgError::Subprocess`] when the process cannot be spawned (for
    /// example if the executable does not exist or has no execute permission) or
    /// when the captured output is not valid UTF-8.
    fn run(&self, argv: &[String]) -> Result<CommandOutput> {
        let (program, rest) = argv.split_first().ok_or_else(|| DcgError::Subprocess {
            command: String::new(),
            detail: "empty argv".to_string(),
        })?;

        let output = Command::new(program)
            .args(rest)
            .output()
            .map_err(|err| DcgError::Subprocess {
                command: program.clone(),
                detail: err.to_string(),
            })?;

        let stdout = String::from_utf8(output.stdout).map_err(|err| DcgError::Subprocess {
            command: program.clone(),
            detail: format!("stdout is not UTF-8: {err}"),
        })?;
        let stderr = String::from_utf8(output.stderr).map_err(|err| DcgError::Subprocess {
            command: program.clone(),
            detail: format!("stderr is not UTF-8: {err}"),
        })?;

        let status = output.status.code().unwrap_or(-1);

        Ok(CommandOutput {
            status,
            stdout,
            stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Records every argv it is handed and replays canned responses in order.
    pub struct FakeRunner {
        pub responses: Vec<CommandOutput>,
        pub calls: std::cell::RefCell<Vec<Vec<String>>>,
        cursor: std::cell::RefCell<usize>,
    }

    impl FakeRunner {
        pub fn new(responses: Vec<CommandOutput>) -> Self {
            Self {
                responses,
                calls: std::cell::RefCell::new(Vec::new()),
                cursor: std::cell::RefCell::new(0),
            }
        }

        pub fn single(output: CommandOutput) -> Self {
            Self::new(vec![output])
        }

        pub fn last_argv(&self) -> Vec<String> {
            self.calls.borrow().last().cloned().unwrap_or_default()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput> {
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

    #[test]
    fn command_output_is_success_true_on_zero() {
        assert!(ok("x").is_success());
    }

    #[test]
    fn command_output_is_success_false_on_nonzero() {
        assert!(!fail(1, "e").is_success());
    }

    #[test]
    fn fake_runner_returns_single_response() {
        let runner = FakeRunner::single(ok("hello"));
        let out = runner.run(&["echo".to_string()]).unwrap();
        assert_eq!(out.stdout, "hello");
        assert_eq!(out.status, 0);
    }

    #[test]
    fn fake_runner_cycles_through_multiple_responses() {
        let runner = FakeRunner::new(vec![ok("first"), ok("second")]);
        let a = runner.run(&["a".to_string()]).unwrap();
        let b = runner.run(&["b".to_string()]).unwrap();
        assert_eq!(a.stdout, "first");
        assert_eq!(b.stdout, "second");
    }

    #[test]
    fn fake_runner_records_argv() {
        let runner = FakeRunner::single(ok("x"));
        runner
            .run(&["orch-kernelctl".to_string(), "submit".to_string()])
            .unwrap();
        assert_eq!(runner.last_argv(), vec!["orch-kernelctl", "submit"]);
    }

    #[test]
    fn fake_runner_returns_error_beyond_responses() {
        let runner = FakeRunner::single(ok("x"));
        runner.run(&["a".to_string()]).unwrap();
        let err = runner.run(&["b".to_string()]).unwrap_err();
        assert!(matches!(err, DcgError::Subprocess { .. }));
    }

    #[test]
    fn fake_runner_preserves_nonzero_status() {
        let runner = FakeRunner::single(fail(42, "error detail"));
        let out = runner.run(&["cmd".to_string()]).unwrap();
        assert_eq!(out.status, 42);
        assert_eq!(out.stderr, "error detail");
    }

    #[test]
    fn system_runner_empty_argv_returns_subprocess_error() {
        let runner = SystemRunner;
        let err = runner.run(&[]).unwrap_err();
        assert!(matches!(err, DcgError::Subprocess { .. }));
    }

    #[test]
    fn system_runner_unknown_binary_returns_subprocess_error() {
        let runner = SystemRunner;
        let err = runner
            .run(&["/nonexistent/binary/zzz".to_string()])
            .unwrap_err();
        assert!(matches!(err, DcgError::Subprocess { .. }));
    }

    #[test]
    fn system_runner_true_returns_zero() {
        let runner = SystemRunner;
        let out = runner.run(&["/usr/bin/true".to_string()]).unwrap();
        assert_eq!(out.status, 0);
    }

    #[test]
    fn system_runner_false_returns_nonzero() {
        let runner = SystemRunner;
        let out = runner.run(&["/usr/bin/false".to_string()]).unwrap();
        assert_ne!(out.status, 0);
    }

    #[test]
    fn system_runner_echo_captures_stdout() {
        let runner = SystemRunner;
        let out = runner
            .run(&["/bin/echo".to_string(), "hello world".to_string()])
            .unwrap();
        assert_eq!(out.stdout.trim(), "hello world");
    }
}
