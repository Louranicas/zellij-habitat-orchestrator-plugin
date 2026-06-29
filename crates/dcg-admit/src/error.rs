//! Error type and crate-wide [`Result`] alias for the admission gate.
//!
//! Architect-owned foundation: fully implemented so every other module compiles
//! against a stable, typed failure surface. Implementing fibers add a variant
//! only for a genuinely new failure mode; they never widen this into a
//! stringly-typed catch-all, and they never silently swallow a denial.

use std::fmt;

/// Crate-wide result alias bound to [`DcgError`].
pub type Result<T> = std::result::Result<T, DcgError>;

/// Failures produced while admitting (or refusing) a delegated write.
#[derive(Debug)]
#[non_exhaustive]
pub enum DcgError {
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
    /// The presented fence did not strictly supersede the last-admitted fence —
    /// the stale-write rejection (the Redlock guard). This is an explicit
    /// refusal, never silently swallowed.
    StaleFence {
        /// The resource whose fenced write was refused.
        resource: String,
        /// The fence the caller presented.
        presented: u64,
        /// The fence already admitted for the resource.
        last_admitted: u64,
    },
    /// The arming key did not read `armed`; consequential actuation is refused.
    NotArmed {
        /// The arming key that was consulted.
        key: String,
    },
    /// The warrant/policy organ refused the actuation (Err-as-denial — a
    /// non-zero `submit` exit, NOT a parsed `Nack` verdict). See build-plan §8
    /// A2.
    Denied {
        /// Human-readable reason / trail extracted from the refusal.
        reason: String,
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

impl fmt::Display for DcgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { field, value } => {
                write!(f, "value out of range for `{field}`: {value}")
            }
            Self::Empty { field } => write!(f, "required field `{field}` was empty"),
            Self::StaleFence {
                resource,
                presented,
                last_admitted,
            } => write!(
                f,
                "stale fence for `{resource}`: presented {presented} does not supersede last-admitted {last_admitted}"
            ),
            Self::NotArmed { key } => {
                write!(f, "not armed: `{key}` is not `armed` — actuation refused")
            }
            Self::Denied { reason } => write!(f, "actuation denied by warrant policy: {reason}"),
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

impl std::error::Error for DcgError {}

impl From<serde_json::Error> for DcgError {
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
        let err = DcgError::OutOfRange {
            field: "fence.seq",
            value: "-1".to_string(),
        };
        let s = err.to_string();
        assert!(s.contains("fence.seq"), "display should mention field name");
        assert!(s.contains("-1"), "display should mention rejected value");
    }

    #[test]
    fn display_empty() {
        let err = DcgError::Empty { field: "ctl_path" };
        assert!(err.to_string().contains("ctl_path"));
    }

    #[test]
    fn display_stale_fence_mentions_both_fences() {
        let err = DcgError::StaleFence {
            resource: "demo.fence".to_string(),
            presented: 5,
            last_admitted: 9,
        };
        let s = err.to_string();
        assert!(s.contains("demo.fence"));
        assert!(s.contains('5'));
        assert!(s.contains('9'));
    }

    #[test]
    fn display_not_armed_mentions_key() {
        let err = DcgError::NotArmed {
            key: "factory.authorize.ultimate-zellij-orchestrator".to_string(),
        };
        assert!(err.to_string().contains("ultimate-zellij-orchestrator"));
    }

    #[test]
    fn display_denied_mentions_reason() {
        let err = DcgError::Denied {
            reason: "RECIPE_EXECUTION default DENY".to_string(),
        };
        assert!(err.to_string().contains("RECIPE_EXECUTION default DENY"));
    }

    #[test]
    fn display_subprocess() {
        let err = DcgError::Subprocess {
            command: "orch-kernelctl".to_string(),
            detail: "exit 1".to_string(),
        };
        let s = err.to_string();
        assert!(s.contains("orch-kernelctl"));
        assert!(s.contains("exit 1"));
    }

    #[test]
    fn display_parse() {
        let err = DcgError::Parse {
            source: "submit-receipt",
            detail: "expected JSON".to_string(),
        };
        let s = err.to_string();
        assert!(s.contains("submit-receipt"));
        assert!(s.contains("expected JSON"));
    }

    #[test]
    fn display_json() {
        let err = DcgError::Json("trailing comma".to_string());
        assert!(err.to_string().contains("trailing comma"));
    }

    #[test]
    fn display_not_implemented() {
        let err = DcgError::NotImplemented { unit: "admit::admit_write" };
        assert!(err.to_string().contains("admit::admit_write"));
    }

    // -----------------------------------------------------------------------
    // From<serde_json::Error> conversion
    // -----------------------------------------------------------------------

    #[test]
    fn from_serde_json_error_wraps_as_json_variant() {
        let raw_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err: DcgError = DcgError::from(raw_err);
        assert!(matches!(err, DcgError::Json(_)));
    }

    #[test]
    fn question_mark_operator_converts_serde_json_error() {
        fn try_parse() -> Result<()> {
            let _: serde_json::Value = serde_json::from_str("not json")?;
            Ok(())
        }
        let err = try_parse().unwrap_err();
        assert!(matches!(err, DcgError::Json(_)));
    }

    // -----------------------------------------------------------------------
    // std::error::Error implementation
    // -----------------------------------------------------------------------

    #[test]
    fn dcg_error_implements_std_error() {
        fn takes_dyn_error(_: &dyn std::error::Error) {}
        let err = DcgError::Empty { field: "x" };
        takes_dyn_error(&err);
    }

    // -----------------------------------------------------------------------
    // Variant distinctness guard
    // -----------------------------------------------------------------------

    #[test]
    fn all_variants_produce_distinct_nonempty_messages() {
        let variants: &[DcgError] = &[
            DcgError::OutOfRange {
                field: "f",
                value: "v".to_string(),
            },
            DcgError::Empty { field: "f" },
            DcgError::StaleFence {
                resource: "r".to_string(),
                presented: 1,
                last_admitted: 2,
            },
            DcgError::NotArmed {
                key: "k".to_string(),
            },
            DcgError::Denied {
                reason: "d".to_string(),
            },
            DcgError::Subprocess {
                command: "c".to_string(),
                detail: "d".to_string(),
            },
            DcgError::Parse {
                source: "s",
                detail: "d".to_string(),
            },
            DcgError::Json("j".to_string()),
            DcgError::NotImplemented { unit: "u" },
        ];
        let messages: Vec<String> = variants.iter().map(ToString::to_string).collect();
        assert!(messages.iter().all(|m| !m.is_empty()));
        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(unique.len(), messages.len(), "duplicate error messages");
    }
}
