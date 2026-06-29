//! Error type and crate-wide [`Result`] alias for the perception assembler.
//!
//! Architect-owned foundation: fully implemented so every other module compiles
//! against a stable, typed failure surface. Implementing fibers add a variant
//! only for a genuinely new failure mode; they never widen this into a
//! stringly-typed catch-all.

use std::fmt;

/// Crate-wide result alias bound to [`PerceiveError`].
pub type Result<T> = std::result::Result<T, PerceiveError>;

/// Failures produced while assembling or emitting a perception manifest.
#[derive(Debug)]
#[non_exhaustive]
pub enum PerceiveError {
    /// A bounded newtype received a value outside its permitted range.
    OutOfRange {
        /// Name of the field that failed validation.
        field: &'static str,
        /// The rejected value, rendered for diagnostics.
        value: String,
    },
    /// A required input string was unexpectedly empty.
    Empty {
        /// Name of the field that was empty.
        field: &'static str,
    },
    /// An external command failed to spawn or returned a non-success status.
    Subprocess {
        /// The command that was invoked, for diagnostics.
        command: String,
        /// Detail about why it failed.
        detail: String,
    },
    /// The output of an external command could not be parsed.
    Parse {
        /// The source whose output could not be parsed.
        source: &'static str,
        /// Detail about the parse failure.
        detail: String,
    },
    /// JSON serialization or deserialization failed.
    Json(String),
    /// A capability is scaffolded but not yet implemented by its owning fiber.
    NotImplemented {
        /// The unit of work that remains to be implemented.
        unit: &'static str,
    },
}

impl fmt::Display for PerceiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { field, value } => {
                write!(f, "value out of range for `{field}`: {value}")
            }
            Self::Empty { field } => write!(f, "required field `{field}` was empty"),
            Self::Subprocess { command, detail } => {
                write!(f, "subprocess `{command}` failed: {detail}")
            }
            Self::Parse { source, detail } => {
                write!(f, "failed to parse output from `{source}`: {detail}")
            }
            Self::Json(detail) => write!(f, "json error: {detail}"),
            Self::NotImplemented { unit } => write!(f, "not yet implemented: {unit}"),
        }
    }
}

impl std::error::Error for PerceiveError {}

impl From<serde_json::Error> for PerceiveError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // -----------------------------------------------------------------------
    // Display formatting tests
    // -----------------------------------------------------------------------

    #[test]
    fn display_out_of_range() {
        let err = PerceiveError::OutOfRange {
            field: "port",
            value: "0".to_string(),
        };
        let s = err.to_string();
        assert!(s.contains("port"), "display should mention field name");
        assert!(s.contains('0'), "display should mention rejected value");
    }

    #[test]
    fn display_empty() {
        let err = PerceiveError::Empty { field: "ctl_path" };
        assert!(err.to_string().contains("ctl_path"));
    }

    #[test]
    fn display_subprocess() {
        let err = PerceiveError::Subprocess {
            command: "curl".to_string(),
            detail: "connection refused".to_string(),
        };
        let s = err.to_string();
        assert!(s.contains("curl"));
        assert!(s.contains("connection refused"));
    }

    #[test]
    fn display_parse() {
        let err = PerceiveError::Parse {
            source: "kv-lease",
            detail: "expected JSON".to_string(),
        };
        let s = err.to_string();
        assert!(s.contains("kv-lease"));
        assert!(s.contains("expected JSON"));
    }

    #[test]
    fn display_json() {
        let err = PerceiveError::Json("trailing comma".to_string());
        assert!(err.to_string().contains("trailing comma"));
    }

    #[test]
    fn display_not_implemented() {
        let err = PerceiveError::NotImplemented { unit: "foo::bar" };
        let s = err.to_string();
        assert!(s.contains("foo::bar"));
    }

    // -----------------------------------------------------------------------
    // From<serde_json::Error> conversion
    // -----------------------------------------------------------------------

    #[test]
    fn from_serde_json_error_wraps_as_json_variant() {
        let raw_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err: PerceiveError = PerceiveError::from(raw_err);
        assert!(matches!(err, PerceiveError::Json(_)));
    }

    #[test]
    fn question_mark_operator_converts_serde_json_error() {
        fn try_parse() -> Result<()> {
            let _: serde_json::Value = serde_json::from_str("not json")?;
            Ok(())
        }
        let err = try_parse().unwrap_err();
        assert!(matches!(err, PerceiveError::Json(_)));
    }

    // -----------------------------------------------------------------------
    // std::error::Error implementation
    // -----------------------------------------------------------------------

    #[test]
    fn perceive_error_implements_std_error() {
        fn takes_dyn_error(_: &dyn std::error::Error) {}
        let err = PerceiveError::Empty { field: "x" };
        takes_dyn_error(&err);
    }

    // -----------------------------------------------------------------------
    // non_exhaustive guard — adding a match ensures the compiler would warn
    // if a new variant were added without a catch-all.
    // -----------------------------------------------------------------------

    #[test]
    fn error_is_non_exhaustive_pattern_requires_catch_all() {
        // This test exists to document the non_exhaustive attribute and ensure
        // all current variants produce distinct messages.
        let variants: &[PerceiveError] = &[
            PerceiveError::OutOfRange {
                field: "f",
                value: "v".to_string(),
            },
            PerceiveError::Empty { field: "f" },
            PerceiveError::Subprocess {
                command: "c".to_string(),
                detail: "d".to_string(),
            },
            PerceiveError::Parse {
                source: "s",
                detail: "d".to_string(),
            },
            PerceiveError::Json("j".to_string()),
            PerceiveError::NotImplemented { unit: "u" },
        ];
        let messages: Vec<String> = variants.iter().map(ToString::to_string).collect();
        // All messages must be non-empty and unique
        assert!(messages.iter().all(|m| !m.is_empty()));
        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(unique.len(), messages.len(), "duplicate error messages");
    }
}
