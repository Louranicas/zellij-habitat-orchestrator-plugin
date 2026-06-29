//! Typed representation of the `perceive.snapshot.v1` manifest.
//!
//! These types are the shared write-path contract: the source modules populate
//! them, [`crate::assemble`] composes them into a [`PerceiveSnapshot`], and
//! [`crate::emit`] serializes that snapshot onto the orchestrator-kernel spine.
//! The data definitions and bounded newtypes are architect-owned foundation;
//! the population logic lives in the source modules.

use serde::Serialize;

use crate::error::PerceiveError;
use crate::Result;

/// Schema identifier embedded in every emitted manifest.
pub const SCHEMA: &str = "perceive.snapshot.v1";

/// A TCP port, constrained to the non-zero range `1..=65535`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Port(u16);

impl Port {
    /// Creates a [`Port`], rejecting `0`.
    ///
    /// # Errors
    /// Returns [`PerceiveError::OutOfRange`] when `port` is zero.
    pub fn new(port: u16) -> Result<Self> {
        if port == 0 {
            return Err(PerceiveError::OutOfRange {
                field: "port",
                value: port.to_string(),
            });
        }
        Ok(Self(port))
    }

    /// Returns the underlying port number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// An HTTP-style health status code, constrained to `100..=599`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HealthCode(u16);

impl HealthCode {
    /// Creates a [`HealthCode`], rejecting values outside `100..=599`.
    ///
    /// # Errors
    /// Returns [`PerceiveError::OutOfRange`] when `code` is not a valid status.
    pub fn new(code: u16) -> Result<Self> {
        if (100..=599).contains(&code) {
            Ok(Self(code))
        } else {
            Err(PerceiveError::OutOfRange {
                field: "health_code",
                value: code.to_string(),
            })
        }
    }

    /// Returns the underlying status code.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A Unix epoch timestamp in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TimestampMs(u64);

impl TimestampMs {
    /// Wraps a millisecond epoch value.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Returns the underlying millisecond value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A Zellij pane identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PaneId(u32);

impl PaneId {
    /// Wraps a pane identifier.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns the underlying identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A zero-based tab index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TabIndex(u32);

impl TabIndex {
    /// Wraps a tab index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the underlying index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// An operating-system process identifier, constrained to non-zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Pid(u32);

impl Pid {
    /// Creates a [`Pid`], rejecting `0`.
    ///
    /// # Errors
    /// Returns [`PerceiveError::OutOfRange`] when `pid` is zero.
    pub fn new(pid: u32) -> Result<Self> {
        if pid == 0 {
            return Err(PerceiveError::OutOfRange {
                field: "pid",
                value: pid.to_string(),
            });
        }
        Ok(Self(pid))
    }

    /// Returns the underlying process identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A process exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ExitCode(i32);

impl ExitCode {
    /// Wraps a process exit code.
    #[must_use]
    pub const fn new(code: i32) -> Self {
        Self(code)
    }

    /// Returns the underlying exit code.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// The body or host helper that produced a manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Produced inside the Zellij plugin body via `command_sources()`.
    Body,
    /// Produced by the host-side `orchestrator-perceive` helper.
    HostHelper,
}

/// A single observed pane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaneObservation {
    /// Tab index hosting the pane.
    pub tab: TabIndex,
    /// Human-readable tab name.
    pub tab_name: String,
    /// Geometry / position descriptor.
    pub pos: String,
    /// Stable pane identifier.
    pub pane_id: PaneId,
    /// Pane title.
    pub title: String,
    /// Working directory, when known.
    pub cwd: String,
    /// Foreground process id, when known.
    pub pid: Option<Pid>,
    /// The command currently running in the pane.
    pub running_command: String,
    /// Whether the pane currently holds focus.
    pub is_focused: bool,
    /// Exit code if the pane's command has terminated.
    pub exit_code: Option<ExitCode>,
}

/// A single observed Zellij session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionObservation {
    /// Session name.
    pub name: String,
    /// Whether this is the current session.
    pub is_current: bool,
}

/// The observed health of one ULTRAPLATE engine endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineProbe {
    /// Short engine name (for example `WFE`).
    pub name: String,
    /// Service port.
    pub port: Port,
    /// Observed status code, or `None` when unreachable.
    pub health_code: Option<HealthCode>,
    /// When the probe was taken.
    pub probed_at_ms: TimestampMs,
}

/// The factory's callable surface at observation time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Default)]
pub struct CatalogObservation {
    /// Workflow names discovered under the workflows directory.
    pub workflows: Vec<String>,
    /// Agent names discovered under the agents directory.
    pub agents: Vec<String>,
    /// Recipe names reported by `just --list`.
    pub just_recipes: Vec<String>,
    /// Provenance describing how the catalog was assembled.
    pub source: String,
}

/// A single observed lease over a shared resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LeaseObservation {
    /// Leased resource key.
    pub resource: String,
    /// Current owner string.
    pub owner: String,
    /// Expiry timestamp.
    pub expires_ms: TimestampMs,
    /// Monotonic fence value minted elsewhere; read-only here.
    pub fence: u64,
}

/// A single observed fiber / campaign.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FiberObservation {
    /// Campaign identifier.
    pub campaign: String,
    /// Root resource or directory the campaign operates on.
    pub root: String,
    /// Active loop identifiers under the campaign.
    pub loops: Vec<String>,
}

/// The complete `perceive.snapshot.v1` manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PerceiveSnapshot {
    /// Schema identifier; always equal to [`SCHEMA`].
    pub schema: String,
    /// When the snapshot was captured.
    pub captured_at_ms: TimestampMs,
    /// Provenance of the snapshot.
    pub source: Source,
    /// Observed panes.
    pub panes: Vec<PaneObservation>,
    /// Observed sessions.
    pub sessions: Vec<SessionObservation>,
    /// Observed engine health.
    pub engines: Vec<EngineProbe>,
    /// Observed callable catalog.
    pub catalog: CatalogObservation,
    /// Observed leases.
    pub leases: Vec<LeaseObservation>,
    /// Observed fibers / campaigns.
    pub fibers: Vec<FiberObservation>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // -----------------------------------------------------------------------
    // Port newtype tests
    // -----------------------------------------------------------------------

    #[test]
    fn port_zero_rejected() {
        assert!(Port::new(0).is_err());
    }

    #[test]
    fn port_one_accepted() {
        assert_eq!(Port::new(1).unwrap().get(), 1);
    }

    #[test]
    fn port_max_accepted() {
        assert_eq!(Port::new(65535).unwrap().get(), 65535);
    }

    #[test]
    fn port_roundtrip_get() {
        for p in [80_u16, 443, 8080, 8142, 8200] {
            assert_eq!(Port::new(p).unwrap().get(), p);
        }
    }

    #[test]
    fn port_out_of_range_error_mentions_field() {
        let err = Port::new(0).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("port"));
    }

    // -----------------------------------------------------------------------
    // HealthCode newtype tests
    // -----------------------------------------------------------------------

    #[test]
    fn health_code_99_rejected() {
        assert!(HealthCode::new(99).is_err());
    }

    #[test]
    fn health_code_100_accepted() {
        assert_eq!(HealthCode::new(100).unwrap().get(), 100);
    }

    #[test]
    fn health_code_200_accepted() {
        assert_eq!(HealthCode::new(200).unwrap().get(), 200);
    }

    #[test]
    fn health_code_599_accepted() {
        assert_eq!(HealthCode::new(599).unwrap().get(), 599);
    }

    #[test]
    fn health_code_600_rejected() {
        assert!(HealthCode::new(600).is_err());
    }

    #[test]
    fn health_code_zero_rejected() {
        assert!(HealthCode::new(0).is_err());
    }

    // -----------------------------------------------------------------------
    // Pid newtype tests
    // -----------------------------------------------------------------------

    #[test]
    fn pid_zero_rejected() {
        assert!(Pid::new(0).is_err());
    }

    #[test]
    fn pid_one_accepted() {
        assert_eq!(Pid::new(1).unwrap().get(), 1);
    }

    #[test]
    fn pid_large_value_accepted() {
        assert_eq!(Pid::new(99999).unwrap().get(), 99999);
    }

    #[test]
    fn pid_out_of_range_error_mentions_field() {
        let err = Pid::new(0).unwrap_err();
        assert!(err.to_string().contains("pid"));
    }

    // -----------------------------------------------------------------------
    // TimestampMs newtype tests
    // -----------------------------------------------------------------------

    #[test]
    fn timestamp_ms_zero_allowed() {
        assert_eq!(TimestampMs::from_millis(0).get(), 0);
    }

    #[test]
    fn timestamp_ms_large_epoch_roundtrip() {
        let ts: u64 = 1_700_000_000_000;
        assert_eq!(TimestampMs::from_millis(ts).get(), ts);
    }

    // -----------------------------------------------------------------------
    // ExitCode newtype tests
    // -----------------------------------------------------------------------

    #[test]
    fn exit_code_zero_accepted() {
        assert_eq!(ExitCode::new(0).get(), 0);
    }

    #[test]
    fn exit_code_negative_accepted() {
        assert_eq!(ExitCode::new(-1).get(), -1);
    }

    #[test]
    fn exit_code_positive_accepted() {
        assert_eq!(ExitCode::new(127).get(), 127);
    }

    // -----------------------------------------------------------------------
    // Source serialization
    // -----------------------------------------------------------------------

    #[test]
    fn source_body_serializes_to_kebab_case() {
        let s = serde_json::to_string(&Source::Body).unwrap();
        assert_eq!(s, r#""body""#);
    }

    #[test]
    fn source_host_helper_serializes_to_kebab_case() {
        let s = serde_json::to_string(&Source::HostHelper).unwrap();
        assert_eq!(s, r#""host-helper""#);
    }

    // -----------------------------------------------------------------------
    // PerceiveSnapshot serde shape (contract compliance)
    // -----------------------------------------------------------------------

    fn minimal_snapshot() -> PerceiveSnapshot {
        PerceiveSnapshot {
            schema: SCHEMA.to_string(),
            captured_at_ms: TimestampMs::from_millis(0),
            source: Source::HostHelper,
            panes: Vec::new(),
            sessions: Vec::new(),
            engines: Vec::new(),
            catalog: CatalogObservation::default(),
            leases: Vec::new(),
            fibers: Vec::new(),
        }
    }

    #[test]
    fn perceive_snapshot_serializes() {
        let snap = minimal_snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"schema\""));
        assert!(json.contains("\"captured_at_ms\""));
        assert!(json.contains("\"source\""));
        assert!(json.contains("\"panes\""));
        assert!(json.contains("\"sessions\""));
        assert!(json.contains("\"engines\""));
        assert!(json.contains("\"catalog\""));
        assert!(json.contains("\"leases\""));
        assert!(json.contains("\"fibers\""));
    }

    #[test]
    fn perceive_snapshot_schema_field_value() {
        let snap = minimal_snapshot();
        let v: serde_json::Value = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["schema"].as_str().unwrap(), SCHEMA);
    }

    #[test]
    fn perceive_snapshot_source_field_is_kebab_case() {
        let snap = minimal_snapshot();
        let v: serde_json::Value = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["source"].as_str().unwrap(), "host-helper");
    }

    #[test]
    fn perceive_snapshot_arrays_are_json_arrays() {
        let snap = minimal_snapshot();
        let v: serde_json::Value = serde_json::to_value(&snap).unwrap();
        assert!(v["panes"].is_array());
        assert!(v["sessions"].is_array());
        assert!(v["engines"].is_array());
        assert!(v["leases"].is_array());
        assert!(v["fibers"].is_array());
    }

    #[test]
    fn schema_constant_value() {
        assert_eq!(SCHEMA, "perceive.snapshot.v1");
    }

    #[test]
    fn catalog_observation_default_has_empty_vectors() {
        let c = CatalogObservation::default();
        assert!(c.workflows.is_empty());
        assert!(c.agents.is_empty());
        assert!(c.just_recipes.is_empty());
    }
}
