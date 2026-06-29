//! The single external-process injection seam.
//!
//! Every module that shells out to a host command (`zellij`, `curl`, `just`,
//! `kv-lease`, `hopf-anchor`, `orch-kernelctl`) does so through the
//! [`CommandRunner`] trait. Production code uses [`SystemRunner`]; tests inject a
//! recording fake. This keeps the assembler deterministic and densely testable
//! without a live habitat.

use std::process::Command;

use crate::error::PerceiveError;
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

/// Abstraction over running an external command with explicit argv.
///
/// Implementors MUST treat `argv[0]` as an absolute executable path and MUST NOT
/// route through a shell (no argument splitting, no glob or variable expansion).
pub trait CommandRunner {
    /// Runs `argv` and captures its output.
    ///
    /// # Errors
    /// Returns [`PerceiveError::Subprocess`] if the process cannot be spawned or
    /// its output cannot be captured.
    fn run(&self, argv: &[String]) -> Result<CommandOutput>;
}

/// Production [`CommandRunner`] backed by [`std::process::Command`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    /// Runs the command at `argv[0]` with the remaining elements as arguments.
    ///
    /// `argv[0]` is interpreted as an absolute path; no shell is invoked.
    ///
    /// # Errors
    /// Returns [`PerceiveError::Subprocess`] when the process cannot be spawned
    /// (for example if the executable does not exist or has no execute permission)
    /// or when the captured output is not valid UTF-8.
    fn run(&self, argv: &[String]) -> Result<CommandOutput> {
        let (program, rest) = argv.split_first().ok_or_else(|| PerceiveError::Subprocess {
            command: String::new(),
            detail: "empty argv".to_string(),
        })?;

        let output = Command::new(program)
            .args(rest)
            .output()
            .map_err(|err| PerceiveError::Subprocess {
                command: program.clone(),
                detail: err.to_string(),
            })?;

        let stdout =
            String::from_utf8(output.stdout).map_err(|err| PerceiveError::Subprocess {
                command: program.clone(),
                detail: format!("stdout is not UTF-8: {err}"),
            })?;
        let stderr =
            String::from_utf8(output.stderr).map_err(|err| PerceiveError::Subprocess {
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

    /// Minimal fake that always returns what you hand it.
    pub struct FakeRunner {
        pub responses: Vec<CommandOutput>,
        inner: std::cell::RefCell<usize>,
    }

    impl FakeRunner {
        pub fn new(responses: Vec<CommandOutput>) -> Self {
            Self {
                responses,
                inner: std::cell::RefCell::new(0),
            }
        }

        pub fn single(output: CommandOutput) -> Self {
            Self::new(vec![output])
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, _argv: &[String]) -> Result<CommandOutput> {
            let mut idx = self.inner.borrow_mut();
            let out = self
                .responses
                .get(*idx)
                .cloned()
                .ok_or_else(|| PerceiveError::Subprocess {
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
    fn fake_runner_returns_error_beyond_responses() {
        let runner = FakeRunner::single(ok("x"));
        runner.run(&["a".to_string()]).unwrap();
        let err = runner.run(&["b".to_string()]).unwrap_err();
        assert!(matches!(err, PerceiveError::Subprocess { .. }));
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
        assert!(matches!(err, PerceiveError::Subprocess { .. }));
    }

    #[test]
    fn system_runner_unknown_binary_returns_subprocess_error() {
        let runner = SystemRunner;
        let err = runner
            .run(&["/nonexistent/binary/zzz".to_string()])
            .unwrap_err();
        assert!(matches!(err, PerceiveError::Subprocess { .. }));
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
