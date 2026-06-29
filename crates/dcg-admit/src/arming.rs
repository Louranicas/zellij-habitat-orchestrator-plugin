//! The arming-key read seam (read-never-write).
//!
//! Consequential actuation is refused unless the campaign arming key
//! ([`ARMING_KEY`]) reads `armed`. Arming enforcement lives in this crate, not in
//! the sidecar (build-plan §8 A3); the sidecar's deny-by-default policy is the
//! backstop. The arming key is owned by Luke @ node 0.A — this crate **reads it
//! and never writes it**: [`AtuinArmingReader`] only ever issues `atuin kv get`.
//!
//! The read flows through the [`ArmingReader`] trait so admission can be tested
//! without the live key. Architect-owned foundation: fully implemented.

use crate::error::DcgError;
use crate::exec::CommandRunner;
use crate::Result;

/// The campaign arming key consulted before any consequential actuation.
pub const ARMING_KEY: &str = "factory.authorize.ultimate-zellij-orchestrator";

/// The observed arming state of the campaign key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmingState {
    /// The key reads exactly `armed`.
    Armed,
    /// The key is present but holds some value other than `armed`.
    Unarmed,
    /// The key is absent, unreadable, or the read failed — treated as not armed.
    Unknown,
}

impl ArmingState {
    /// Returns `true` only for [`ArmingState::Armed`]. `Unarmed` and `Unknown`
    /// both fail closed (deny).
    #[must_use]
    pub const fn is_armed(self) -> bool {
        matches!(self, Self::Armed)
    }
}

/// Reads the campaign arming state. The single seam admission consults; tests
/// inject a deterministic fake instead of touching the live key.
pub trait ArmingReader {
    /// Reads the current [`ArmingState`].
    ///
    /// # Errors
    /// Implementations SHOULD fail closed: a failed read maps to
    /// [`ArmingState::Unknown`] (`Ok`), not an `Err`. An `Err` is reserved for a
    /// programming-level fault (for example a malformed reader configuration);
    /// when returned it must be a [`DcgError`].
    fn read(&self) -> Result<ArmingState>;
}

/// Production [`ArmingReader`] that reads the key via `atuin kv get` through an
/// injected [`CommandRunner`]. Read-only by construction — it issues no `set` or
/// `delete`.
#[derive(Clone, Debug)]
pub struct AtuinArmingReader<R: CommandRunner> {
    runner: R,
    atuin_path: String,
    key: String,
}

impl<R: CommandRunner> AtuinArmingReader<R> {
    /// Creates a reader for [`ARMING_KEY`] using the given absolute `atuin_path`.
    ///
    /// The caller supplies the host-verified absolute path to the `atuin` binary;
    /// no path is guessed here.
    #[must_use]
    pub fn new(runner: R, atuin_path: impl Into<String>) -> Self {
        Self {
            runner,
            atuin_path: atuin_path.into(),
            key: ARMING_KEY.to_string(),
        }
    }

    /// Creates a reader for a non-default key (used by tests and alternate
    /// campaigns).
    #[must_use]
    pub fn with_key(runner: R, atuin_path: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            runner,
            atuin_path: atuin_path.into(),
            key: key.into(),
        }
    }

    /// Returns the key this reader consults.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns a reference to the injected runner (used by tests to inspect the
    /// issued argv and prove the read-never-write invariant).
    #[must_use]
    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> ArmingReader for AtuinArmingReader<R> {
    /// Reads the key via `atuin kv get <key>` and maps the result to an
    /// [`ArmingState`], failing closed on any error or empty value.
    ///
    /// # Errors
    /// Returns [`DcgError::Empty`] if this reader was constructed with an empty
    /// `atuin_path` (a configuration fault). Subprocess failures fail closed to
    /// [`ArmingState::Unknown`] rather than erroring.
    fn read(&self) -> Result<ArmingState> {
        if self.atuin_path.is_empty() {
            return Err(DcgError::Empty {
                field: "atuin_path",
            });
        }

        // Fail closed: an unreachable atuin is "not armed", not a crash.
        let Ok(out) = self.runner.run(&[
            self.atuin_path.clone(),
            "kv".to_string(),
            "get".to_string(),
            self.key.clone(),
        ]) else {
            return Ok(ArmingState::Unknown);
        };

        Ok(classify(out.status, &out.stdout))
    }
}

/// Maps a raw `atuin kv get` result to an [`ArmingState`]. A non-zero exit or an
/// empty value is [`ArmingState::Unknown`]; an exact `armed` is
/// [`ArmingState::Armed`]; anything else is [`ArmingState::Unarmed`].
fn classify(status: i32, stdout: &str) -> ArmingState {
    if status != 0 {
        return ArmingState::Unknown;
    }
    match stdout.trim() {
        "" => ArmingState::Unknown,
        "armed" => ArmingState::Armed,
        _ => ArmingState::Unarmed,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};

    /// Canned runner that records argv and returns one fixed response.
    struct CannedRunner {
        status: i32,
        stdout: String,
        argv: std::cell::RefCell<Vec<String>>,
    }

    impl CannedRunner {
        fn new(status: i32, stdout: &str) -> Self {
            Self {
                status,
                stdout: stdout.to_string(),
                argv: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn last_argv(&self) -> Vec<String> {
            self.argv.borrow().clone()
        }
    }

    impl CommandRunner for CannedRunner {
        fn run(&self, argv: &[String]) -> crate::Result<CommandOutput> {
            *self.argv.borrow_mut() = argv.to_vec();
            Ok(CommandOutput {
                status: self.status,
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    struct ErrRunner;
    impl CommandRunner for ErrRunner {
        fn run(&self, _: &[String]) -> crate::Result<CommandOutput> {
            Err(DcgError::Subprocess {
                command: "atuin".to_string(),
                detail: "not found".to_string(),
            })
        }
    }

    /// A test-only fixed-state reader (the fake admission injects).
    struct FakeArmingReader(ArmingState);
    impl ArmingReader for FakeArmingReader {
        fn read(&self) -> crate::Result<ArmingState> {
            Ok(self.0)
        }
    }

    // -----------------------------------------------------------------------
    // ArmingState semantics
    // -----------------------------------------------------------------------

    #[test]
    fn armed_is_armed() {
        assert!(ArmingState::Armed.is_armed());
    }

    #[test]
    fn unarmed_is_not_armed() {
        assert!(!ArmingState::Unarmed.is_armed());
    }

    #[test]
    fn unknown_fails_closed() {
        assert!(!ArmingState::Unknown.is_armed());
    }

    // -----------------------------------------------------------------------
    // classify()
    // -----------------------------------------------------------------------

    #[test]
    fn classify_exact_armed() {
        assert_eq!(classify(0, "armed"), ArmingState::Armed);
    }

    #[test]
    fn classify_armed_with_surrounding_whitespace() {
        assert_eq!(classify(0, "  armed\n"), ArmingState::Armed);
    }

    #[test]
    fn classify_other_value_is_unarmed() {
        assert_eq!(classify(0, "disarmed"), ArmingState::Unarmed);
    }

    #[test]
    fn classify_empty_is_unknown() {
        assert_eq!(classify(0, ""), ArmingState::Unknown);
    }

    #[test]
    fn classify_nonzero_exit_is_unknown() {
        assert_eq!(classify(1, "armed"), ArmingState::Unknown);
    }

    #[test]
    fn classify_armed_substring_is_not_armed() {
        // Guards against a loose `contains` check: only an exact match arms.
        assert_eq!(classify(0, "rearmed"), ArmingState::Unarmed);
    }

    // -----------------------------------------------------------------------
    // AtuinArmingReader — read path + read-never-write invariant
    // -----------------------------------------------------------------------

    #[test]
    fn reader_armed_key_reads_armed() {
        let reader = AtuinArmingReader::new(CannedRunner::new(0, "armed"), "/usr/bin/atuin");
        assert_eq!(reader.read().unwrap(), ArmingState::Armed);
    }

    #[test]
    fn reader_unarmed_key_reads_unarmed() {
        let reader = AtuinArmingReader::new(CannedRunner::new(0, "pending"), "/usr/bin/atuin");
        assert_eq!(reader.read().unwrap(), ArmingState::Unarmed);
    }

    #[test]
    fn reader_missing_key_reads_unknown() {
        let reader = AtuinArmingReader::new(CannedRunner::new(1, ""), "/usr/bin/atuin");
        assert_eq!(reader.read().unwrap(), ArmingState::Unknown);
    }

    #[test]
    fn reader_subprocess_error_fails_closed_to_unknown() {
        let reader = AtuinArmingReader::new(ErrRunner, "/usr/bin/atuin");
        assert_eq!(reader.read().unwrap(), ArmingState::Unknown);
    }

    #[test]
    fn reader_empty_atuin_path_is_config_error() {
        let reader = AtuinArmingReader::new(CannedRunner::new(0, "armed"), "");
        assert!(matches!(reader.read().unwrap_err(), DcgError::Empty { .. }));
    }

    #[test]
    fn reader_issues_only_kv_get_never_set_or_delete() {
        let reader = AtuinArmingReader::new(CannedRunner::new(0, "armed"), "/usr/bin/atuin");
        let _ = reader.read().unwrap();
        let argv = reader.runner().last_argv();
        assert_eq!(argv.first().map(String::as_str), Some("/usr/bin/atuin"));
        assert!(argv.contains(&"kv".to_string()));
        assert!(argv.contains(&"get".to_string()));
        assert!(argv.contains(&ARMING_KEY.to_string()));
        assert!(!argv.contains(&"set".to_string()), "must never write");
        assert!(!argv.contains(&"delete".to_string()), "must never delete");
    }

    #[test]
    fn reader_default_key_is_the_arming_key() {
        let reader = AtuinArmingReader::new(CannedRunner::new(0, "armed"), "/usr/bin/atuin");
        assert_eq!(reader.key(), ARMING_KEY);
    }

    #[test]
    fn reader_with_key_uses_custom_key() {
        let reader =
            AtuinArmingReader::with_key(CannedRunner::new(0, "armed"), "/usr/bin/atuin", "x.y.z");
        assert_eq!(reader.key(), "x.y.z");
    }

    #[test]
    fn arming_key_constant_value() {
        assert_eq!(ARMING_KEY, "factory.authorize.ultimate-zellij-orchestrator");
    }

    // -----------------------------------------------------------------------
    // Fake reader (the seam admission will inject)
    // -----------------------------------------------------------------------

    #[test]
    fn fake_reader_returns_injected_state() {
        let fake = FakeArmingReader(ArmingState::Armed);
        assert_eq!(fake.read().unwrap(), ArmingState::Armed);
    }
}
