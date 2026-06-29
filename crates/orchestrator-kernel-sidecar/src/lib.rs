//! Durable sidecar primitives for the Zellij Orchestrator Kernel.
//!
//! This crate intentionally starts with the P2.0/P2.1 substrate from the
//! assimilated plan: explicit state paths, `SQLite` WAL, canonical event hashing,
//! replay, chain verification, and a constrained built-in recipe path.

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::env;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Current sidecar schema version.
pub const SCHEMA_VERSION: i64 = 1;

/// Canonical event kind for perceive snapshots emitted by the
/// `orchestrator-perceive` assembler.  Uses the dotted-namespace format
/// (`perceive.snapshot`) which the extended kind validator accepts.
pub const PERCEIVE_SNAPSHOT_KIND: &str = "perceive.snapshot";

/// CLI commands for which `--read-only` (a non-mutating event-log open) is valid.
/// Mutating commands (`init`, `submit`, `append`) are fail-closed rejected.
pub const READ_ONLY_COMMANDS: [&str; 5] =
    ["snapshot", "snapshot-v2", "verify-chain", "replay", "events"];

/// Returns `true` iff `command` may run under `--read-only`.
///
/// Anchored exact-match allowlist (never prefix/substring) — the fail-closed
/// guard the CLI applies before opening the event log read-only.
#[must_use]
pub fn read_only_allowed(command: &str) -> bool {
    READ_ONLY_COMMANDS.contains(&command)
}

const GENESIS_HASH: &str = "sha256:habitat.kernel.event_log.genesis.v1";
const SUBMIT_REQUEST_SCHEMA: &str = "habitat.kernel.submit.request.v1";
const SUBMIT_RESPONSE_SCHEMA: &str = "habitat.kernel.submit.response.v1";
const DEFAULT_POLICY_REF: &str = "config/zellij-orchestrator-kernel-warrants.v2.json";
const DEFAULT_POLICY_VERSION: &str = "warrants.v2";
const DEFAULT_POLICY_HASH: &str =
    "sha256:e9015f7850bc3d8528e500f2dfee999d61c03334dc0939802f74db7f1167ac73";
const BUILTIN_VERIFY_CHAIN: &str = "verify_chain";
const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS event_log (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT NOT NULL UNIQUE,
  trace_id TEXT NOT NULL,
  parent_id TEXT,
  kind TEXT NOT NULL,
  actor TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  hash TEXT NOT NULL,
  prev_hash TEXT,
  schema_version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
  msg_id TEXT PRIMARY KEY,
  trace_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  from_role TEXT,
  to_role TEXT,
  idempotency_key TEXT,
  state TEXT NOT NULL,
  integration_state TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pane_snapshots (
  snapshot_id TEXT PRIMARY KEY,
  captured_at TEXT NOT NULL,
  manifest_json TEXT NOT NULL,
  pane_count INTEGER NOT NULL,
  command_tab_count INTEGER NOT NULL,
  chain_verified_at TEXT,
  last_replayed_seq INTEGER
);

CREATE TABLE IF NOT EXISTS warrants (
  warrant_id TEXT PRIMARY KEY,
  trace_id TEXT NOT NULL,
  class TEXT NOT NULL,
  verdict TEXT NOT NULL,
  reason TEXT NOT NULL,
  policy_ref TEXT NOT NULL,
  policy_version TEXT NOT NULL,
  policy_hash TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  justfile_hash TEXT,
  recipe_body_hash TEXT,
  decision_before_execution INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS recipe_runs (
  run_id TEXT PRIMARY KEY,
  trace_id TEXT NOT NULL,
  recipe TEXT NOT NULL,
  args_json TEXT NOT NULL,
  exit_code INTEGER,
  semantic_state TEXT NOT NULL,
  semantic_reason TEXT,
  stdout_path TEXT,
  stderr_path TEXT,
  stdout_truncated INTEGER NOT NULL DEFAULT 0,
  stderr_truncated INTEGER NOT NULL DEFAULT 0,
  started_at TEXT NOT NULL,
  finished_at TEXT
);

CREATE TABLE IF NOT EXISTS recipe_run_warrants (
  run_id TEXT PRIMARY KEY,
  warrant_id TEXT NOT NULL,
  linked_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES recipe_runs(run_id),
  FOREIGN KEY(warrant_id) REFERENCES warrants(warrant_id)
);

CREATE TABLE IF NOT EXISTS edge_coherence (
  edge_id TEXT PRIMARY KEY,
  trace_id TEXT NOT NULL,
  edge TEXT NOT NULL,
  state TEXT NOT NULL,
  observed_at TEXT NOT NULL,
  evidence_ref TEXT
);

CREATE TABLE IF NOT EXISTS idempotency_records (
  idempotency_key TEXT PRIMARY KEY,
  request_hash TEXT NOT NULL,
  first_trace TEXT NOT NULL,
  response_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
";

/// A result alias for sidecar operations.
pub type Result<T> = std::result::Result<T, KernelError>;

/// Error type used by the sidecar crate.
#[derive(Debug)]
pub enum KernelError {
    /// Filesystem error.
    Io(std::io::Error),
    /// `SQLite` error.
    Sql(rusqlite::Error),
    /// JSON error.
    Json(serde_json::Error),
    /// Invalid CLI or API input.
    InvalidInput(String),
    /// Hash-chain verification failed.
    ChainViolation(String),
}

impl Display for KernelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Sql(err) => write!(f, "sqlite error: {err}"),
            Self::Json(err) => write!(f, "json error: {err}"),
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::ChainViolation(msg) => write!(f, "chain violation: {msg}"),
        }
    }
}

impl std::error::Error for KernelError {}

impl From<std::io::Error> for KernelError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<rusqlite::Error> for KernelError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

impl From<serde_json::Error> for KernelError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

/// Configured sidecar state paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatePaths {
    /// Directory containing sidecar state.
    pub state_dir: PathBuf,
    /// `SQLite` database path.
    pub db_path: PathBuf,
    /// Directory for bounded stdout/stderr artifacts.
    pub artifacts_dir: PathBuf,
}

impl StatePaths {
    /// Resolve default state paths.
    ///
    /// `ORCH_KERNEL_STATE_DIR` overrides the default. Otherwise the path is
    /// `Orchestrator/operator-kernel/state` under the current workspace root.
    ///
    /// # Errors
    ///
    /// Returns an error if the current directory cannot be read.
    pub fn default_from_env() -> Result<Self> {
        if let Some(dir) = env::var_os("ORCH_KERNEL_STATE_DIR") {
            return Ok(Self::from_state_dir(PathBuf::from(dir)));
        }

        let cwd = env::current_dir()?;
        let workspace = find_workspace_root(&cwd).unwrap_or(cwd);
        Ok(Self::from_state_dir(
            workspace.join("Orchestrator/operator-kernel/state"),
        ))
    }

    /// Build paths from a state directory.
    #[must_use]
    pub fn from_state_dir(state_dir: PathBuf) -> Self {
        Self {
            db_path: state_dir.join("orchestrator-kernel.sqlite"),
            artifacts_dir: state_dir.join("artifacts"),
            state_dir,
        }
    }

    /// Ensure all directories exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the state or artifact directories cannot be created.
    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        fs::create_dir_all(&self.artifacts_dir)?;
        Ok(())
    }
}

/// Input for appending a durable event.
#[derive(Clone, Debug)]
pub struct AppendEvent {
    /// Event kind, e.g. `HEARTBEAT`, `TASK`, `RESULT`.
    pub kind: String,
    /// Trace id tying events together.
    pub trace_id: String,
    /// Optional parent event id.
    pub parent_id: Option<String>,
    /// Actor writing the event.
    pub actor: String,
    /// Event payload as JSON.
    pub payload: Value,
}

/// Durable task admission request.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SubmitRequest {
    /// Request schema id.
    pub schema: String,
    /// Trace id tying all future events together.
    pub trace_id: String,
    /// Caller supplied idempotency key.
    pub idempotency_key: String,
    /// Semantic task kind. MVP accepts `TASK`.
    pub kind: String,
    /// Actor or subsystem requesting admission.
    #[serde(default = "default_operator")]
    pub operator: String,
    /// Optional recipe request. Durable admission does not execute recipes.
    #[serde(default)]
    pub requested_recipe: Option<String>,
    /// Caller payload.
    #[serde(default)]
    pub payload: Value,
}

/// Sidecar idempotency result.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdempotencyState {
    /// Newly admitted request.
    New,
    /// Repeat delivery of the same canonical request.
    Replay,
    /// Same idempotency key reused for different canonical bytes.
    Conflict,
}

/// Durable submit verdict.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubmitVerdict {
    /// Event log append completed and returned a hash.
    AckDurable,
    /// Request was rejected before durable admission.
    Nack,
}

/// Durable task admission response.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SubmitResponse {
    /// Response schema id.
    pub schema: String,
    /// Admission verdict.
    pub verdict: SubmitVerdict,
    /// Trace id from the request.
    pub trace_id: String,
    /// Durable event id when admitted.
    pub event_id: Option<String>,
    /// Durable event hash when admitted.
    pub event_hash: Option<String>,
    /// Integration state after admission.
    pub integration_state: String,
    /// Idempotency state.
    pub idempotency: IdempotencyState,
    /// Machine-readable reason.
    pub reason: String,
    /// Canonical request hash.
    pub request_hash: String,
    /// Built-in run id when a recipe completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Result event id when a recipe completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_event_id: Option<String>,
}

/// Event row stored in the durable event log.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EventRow {
    /// Monotonic `SQLite` sequence.
    pub seq: i64,
    /// Stable event id.
    pub event_id: String,
    /// Trace id tying events together.
    pub trace_id: String,
    /// Parent event id if any.
    pub parent_id: Option<String>,
    /// Event kind.
    pub kind: String,
    /// Actor that emitted the event.
    pub actor: String,
    /// Payload JSON string as stored.
    pub payload_json: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Hash of previous hash plus canonical event bytes.
    pub hash: String,
    /// Previous row hash, or genesis.
    pub prev_hash: Option<String>,
    /// Schema version used for this row.
    pub schema_version: i64,
}

/// Measured edge state emitted by snapshots.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EdgeSnapshot {
    /// Edge name.
    pub edge: String,
    /// Current measured state.
    pub state: String,
    /// Observation timestamp.
    pub observed_at: String,
    /// Evidence reference, usually an event id.
    pub evidence_ref: Option<String>,
}

/// Summary emitted by `snapshot --json`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Snapshot {
    /// Health status string.
    pub status: String,
    /// Database path.
    pub db_path: String,
    /// Last event sequence, or zero when empty.
    pub last_seq: i64,
    /// Last event hash, or genesis when empty.
    pub last_hash: String,
    /// Total events in the log.
    pub event_count: i64,
    /// Whether `verify_chain` passed.
    pub verify_chain_ok: bool,
    /// Schema version.
    pub schema_version: i64,
    /// Snapshot generation timestamp.
    pub generated_at: String,
    /// Most recently measured edge states.
    pub edges: Vec<EdgeSnapshot>,
    /// Number of policy warrant rows.
    pub warrant_count: i64,
    /// Number of message rows not yet integrated.
    pub queue_depth: i64,
    /// Parsed payload of the most recent `perceive.snapshot` event, if any.
    /// Absent from serialized output when `None` (cortex uses this to skip
    /// the round-trip when no perceive pass has run yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_perceive: Option<serde_json::Value>,
}

/// Snapshot v2 sidecar block.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SnapshotV2Sidecar {
    /// Database path.
    pub db_path: String,
    /// Schema version.
    pub schema_version: i64,
    /// Whether `verify_chain` passed.
    pub verify_chain_ok: bool,
    /// Last event id when available.
    pub last_event_id: Option<String>,
    /// Last event hash, or genesis.
    pub last_event_hash: String,
}

/// Snapshot v2 fitness block.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SnapshotV2Fitness {
    /// Fitness score in [0, 1].
    pub score: f64,
    /// Dominant current loss term.
    pub dominant_loss: String,
}

/// Snapshot v2 pipe block.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SnapshotV2Pipe {
    /// Current pipe mode.
    pub mode: String,
    /// Current circuit state.
    pub circuit_state: String,
    /// p99 latency when measured.
    pub p99_ms: Option<f64>,
    /// Timeout count in the current observation window.
    pub timeouts: i64,
}

/// Snapshot v2 dashboard truth block.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SnapshotV2DashboardTruth {
    /// Dashboard is projection-only and should render measured facts only.
    pub measured_only: bool,
    /// Fields known to be stale.
    pub stale_fields: Vec<String>,
}

/// Contract snapshot for dashboard truth projection.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SnapshotV2 {
    /// Snapshot schema id.
    pub schema: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Runtime source.
    pub source: String,
    /// State directory.
    pub state_dir: String,
    /// Sidecar truth block.
    pub sidecar: SnapshotV2Sidecar,
    /// Measured edges.
    pub edges: Vec<EdgeSnapshot>,
    /// Fitness projection.
    pub fitness: SnapshotV2Fitness,
    /// Pipe state.
    pub pipe: SnapshotV2Pipe,
    /// Dashboard truth metadata.
    pub dashboard_truth: SnapshotV2DashboardTruth,
}

struct BuiltinRecipeResult {
    run_id: String,
    result_event_id: String,
}

#[derive(Debug, Deserialize)]
struct PolicyConfig {
    policy_ref: String,
    policy_version: String,
    policy_hash: String,
    rules: Vec<PolicyRule>,
}

#[derive(Debug, Deserialize)]
struct PolicyRule {
    id: String,
    class: String,
    verdict: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    recipe: Option<String>,
}

struct PolicyDecision {
    class: String,
    reason: String,
    policy_ref: String,
    policy_version: String,
    policy_hash: String,
}

struct RecipeRunRecord<'a> {
    run_id: &'a str,
    warrant_id: &'a str,
    request: &'a SubmitRequest,
    semantic_state: &'a str,
    semantic_reason: &'a str,
    exit_code: i32,
    started_at: &'a str,
}

/// Durable event log backed by `SQLite`.
pub struct EventLog {
    conn: Connection,
    db_path: PathBuf,
    state_dir: PathBuf,
}

impl EventLog {
    /// Open or create the event log at the given paths.
    ///
    /// # Errors
    ///
    /// Returns an error if directories cannot be created, the database cannot be
    /// opened, or schema initialization fails.
    pub fn open(paths: &StatePaths) -> Result<Self> {
        paths.ensure_dirs()?;
        let conn = Connection::open(&paths.db_path)?;
        let log = Self {
            conn,
            db_path: paths.db_path.clone(),
            state_dir: paths.state_dir.clone(),
        };
        log.initialize()?;
        Ok(log)
    }

    /// Open the event log **read-only** for projection / witness reads.
    ///
    /// Unlike [`EventLog::open`], this does NOT create directories, run
    /// [`EventLog::initialize`] (no `WAL` pragma, DDL, or `INSERT`), or otherwise
    /// mutate the database: the connection is `SQLITE_OPEN_READ_ONLY`. A witness
    /// (e.g. the dashboard) can therefore read `snapshot`, `snapshot-v2`,
    /// `verify-chain`, `replay`, and `events` without contending with — or
    /// triggering a `WAL` checkpoint against — a live writer.
    ///
    /// # Errors
    ///
    /// Returns an error if the database does not exist (a read-only open cannot
    /// create it) or cannot be opened read-only.
    pub fn open_read_only(paths: &StatePaths) -> Result<Self> {
        let conn =
            Connection::open_with_flags(&paths.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // busy_timeout is a per-connection runtime setting (it does not write to
        // the database), so it is permitted on a read-only connection and avoids
        // an immediate SQLITE_BUSY when a writer momentarily holds the lock.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self {
            conn,
            db_path: paths.db_path.clone(),
            state_dir: paths.state_dir.clone(),
        })
    }

    /// Initialize `WAL` mode and schema.
    ///
    /// # Errors
    ///
    /// Returns an error if `SQLite` pragmas or schema statements fail.
    pub fn initialize(&self) -> Result<()> {
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "busy_timeout", 5000_i64)?;
        self.conn.pragma_update(None, "synchronous", "NORMAL")?;
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        self.conn.execute_batch(SCHEMA_SQL)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (?1, ?2)",
            params![SCHEMA_VERSION, now_timestamp()?],
        )?;
        Ok(())
    }

    /// Admit a task into the durable sidecar log.
    ///
    /// This method is intentionally admission-only: it records an immutable
    /// task event, idempotency receipt, warrant row, message row, and edge facts.
    /// Recipe execution remains a separate, policy-gated step.
    ///
    /// # Errors
    ///
    /// Returns an error if request validation, canonicalization, append, or
    /// receipt persistence fails.
    pub fn submit(&self, request: &SubmitRequest) -> Result<SubmitResponse> {
        self.begin_immediate()?;
        let result = self.submit_unlocked(request);
        self.finish_transaction(result)
    }

    fn submit_unlocked(&self, request: &SubmitRequest) -> Result<SubmitResponse> {
        validate_submit_request(request)?;
        let policy = resolve_policy_decision(request)?;
        let request_hash = submit_request_hash(request)?;

        if let Some((stored_hash, response_json)) =
            self.idempotency_record(&request.idempotency_key)?
        {
            if stored_hash == request_hash {
                let mut response: SubmitResponse = serde_json::from_str(&response_json)?;
                response.idempotency = IdempotencyState::Replay;
                return Ok(response);
            }
            return Ok(SubmitResponse {
                schema: SUBMIT_RESPONSE_SCHEMA.to_string(),
                verdict: SubmitVerdict::Nack,
                trace_id: request.trace_id.clone(),
                event_id: None,
                event_hash: None,
                integration_state: "REJECTED".to_string(),
                idempotency: IdempotencyState::Conflict,
                reason: "IDEMPOTENCY_CONFLICT".to_string(),
                request_hash,
                run_id: None,
                result_event_id: None,
            });
        }

        let event = self.append_event_unlocked(&AppendEvent {
            kind: "TASK_INGESTED".to_string(),
            trace_id: request.trace_id.clone(),
            parent_id: None,
            actor: request.operator.clone(),
            payload: json!({
                "schema": SUBMIT_REQUEST_SCHEMA,
                "kind": request.kind,
                "idempotency_key": request.idempotency_key,
                "payload": request.payload,
                "requested_recipe": request.requested_recipe,
                "request_hash": request_hash
            }),
        })?;
        let warrant_id = self.record_warrant(request, &request_hash, &policy)?;
        self.record_message(request)?;
        self.record_edge(
            &format!("edge_{}_submit_to_event", event.event_id),
            &request.trace_id,
            "submit_to_event",
            "MEASURED",
            Some(&event.event_id),
        )?;
        self.record_edge(
            &format!("edge_{warrant_id}_event_to_warrant"),
            &request.trace_id,
            "event_to_warrant",
            "MEASURED",
            Some(&warrant_id),
        )?;

        let mut response = SubmitResponse {
            schema: SUBMIT_RESPONSE_SCHEMA.to_string(),
            verdict: SubmitVerdict::AckDurable,
            trace_id: request.trace_id.clone(),
            event_id: Some(event.event_id.clone()),
            event_hash: Some(event.hash.clone()),
            integration_state: "INGESTED".to_string(),
            idempotency: IdempotencyState::New,
            reason: "TASK_INGESTED_DURABLY".to_string(),
            request_hash,
            run_id: None,
            result_event_id: None,
        };
        if request.requested_recipe.as_deref() == Some(BUILTIN_VERIFY_CHAIN) {
            let result = self.execute_verify_chain_recipe(request, &event, &warrant_id)?;
            response.integration_state = "INTEGRATED".to_string();
            response.reason = "BUILTIN_VERIFY_CHAIN_INTEGRATED".to_string();
            response.run_id = Some(result.run_id);
            response.result_event_id = Some(result.result_event_id);
        }
        self.record_idempotency(&request.idempotency_key, &response)?;
        Ok(response)
    }

    /// Append an event and return the stored row.
    ///
    /// # Errors
    ///
    /// Returns an error if input validation, canonicalization, hashing, or the
    /// database insert fails.
    pub fn append_event(&self, input: &AppendEvent) -> Result<EventRow> {
        self.begin_immediate()?;
        let result = self.append_event_unlocked(input);
        self.finish_transaction(result)
    }

    fn append_event_unlocked(&self, input: &AppendEvent) -> Result<EventRow> {
        validate_event_kind(&input.kind)?;
        validate_non_empty("trace_id", &input.trace_id)?;
        validate_non_empty("actor", &input.actor)?;

        let payload_json = canonical_json(&input.payload)?;
        let created_at = now_timestamp()?;
        let prev_hash = self
            .last_hash()?
            .unwrap_or_else(|| GENESIS_HASH.to_string());
        let event_id = make_event_id(&input.kind)?;
        let canonical = CanonicalEvent {
            event_id: &event_id,
            trace_id: &input.trace_id,
            parent_id: input.parent_id.as_deref(),
            kind: &input.kind,
            actor: &input.actor,
            payload_json: &payload_json,
            created_at: &created_at,
            prev_hash: Some(&prev_hash),
            schema_version: SCHEMA_VERSION,
        }
        .to_json()?;
        let hash = hash_event(&prev_hash, &canonical);

        self.conn.execute(
            "INSERT INTO event_log
             (event_id, trace_id, parent_id, kind, actor, payload_json, created_at, hash, prev_hash, schema_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                event_id,
                input.trace_id,
                input.parent_id,
                input.kind,
                input.actor,
                payload_json,
                created_at,
                hash,
                prev_hash,
                SCHEMA_VERSION,
            ],
        )?;

        let seq = self.conn.last_insert_rowid();
        self.event_by_seq(seq)
    }

    /// Return a health snapshot, including chain verification status.
    ///
    /// The `latest_perceive` field is populated when a `perceive.snapshot`
    /// event is present; it is `None` until the first perceive pass emits.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be queried or a stored
    /// `perceive.snapshot` payload cannot be parsed as JSON.
    pub fn snapshot(&self) -> Result<Snapshot> {
        let event_count = self.event_count()?;
        let (last_seq, last_hash) = self
            .last_seq_hash()?
            .unwrap_or_else(|| (0, GENESIS_HASH.to_string()));
        let verify_chain_ok = self.verify_chain().is_ok();
        let latest_perceive = self
            .latest_event_of_kind(PERCEIVE_SNAPSHOT_KIND)?
            .map(|row| serde_json::from_str::<serde_json::Value>(&row.payload_json))
            .transpose()
            .map_err(KernelError::from)?;
        Ok(Snapshot {
            status: "ok".to_string(),
            db_path: self.db_path.display().to_string(),
            last_seq,
            last_hash,
            event_count,
            verify_chain_ok,
            schema_version: SCHEMA_VERSION,
            generated_at: now_timestamp()?,
            edges: self.edge_snapshots()?,
            warrant_count: self.warrant_count()?,
            queue_depth: self.queue_depth()?,
            latest_perceive,
        })
    }

    /// Return the most recent event of `kind`, or `None` when the log
    /// contains no events of that kind.
    ///
    /// This is a read-only query; it does not validate the kind as
    /// `UPPER_SNAKE_CASE` — callers may look up any stored kind string
    /// (including dotted-namespace kinds such as `perceive.snapshot`).
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::InvalidInput`] when `kind` is empty or
    /// contains only whitespace.  Returns [`KernelError::Sql`] on a
    /// database read failure.
    pub fn latest_event_of_kind(&self, kind: &str) -> Result<Option<EventRow>> {
        validate_non_empty("kind", kind)?;
        self.conn
            .query_row(
                "SELECT seq, event_id, trace_id, parent_id, kind, actor, payload_json,
                        created_at, hash, prev_hash, schema_version
                 FROM event_log
                 WHERE kind = ?1
                 ORDER BY seq DESC LIMIT 1",
                params![kind],
                row_from_sql,
            )
            .optional()
            .map_err(KernelError::from)
    }

    /// Return a contract-shaped snapshot v2 for dashboard projection.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing snapshot cannot be queried.
    pub fn snapshot_v2(&self) -> Result<SnapshotV2> {
        let snapshot = self.snapshot()?;
        let last_event_id = self.last_event_id()?;
        let missing_edges = required_edges()
            .iter()
            .filter(|edge| !snapshot.edges.iter().any(|item| item.edge == **edge))
            .count();
        let (fitness, dominant_loss) = if !snapshot.verify_chain_ok {
            (0.0, "durable_admission_integrity".to_string())
        } else if missing_edges > 0 {
            (0.74, "edge_coherence".to_string())
        } else {
            (0.80, "pipe_terminality".to_string())
        };

        Ok(SnapshotV2 {
            schema: "habitat.kernel.snapshot.v2".to_string(),
            created_at: snapshot.generated_at,
            source: "orchestrator-kernel-sidecar".to_string(),
            state_dir: self.state_dir.display().to_string(),
            sidecar: SnapshotV2Sidecar {
                db_path: snapshot.db_path,
                schema_version: snapshot.schema_version,
                verify_chain_ok: snapshot.verify_chain_ok,
                last_event_id,
                last_event_hash: snapshot.last_hash,
            },
            edges: snapshot.edges,
            fitness: SnapshotV2Fitness {
                score: fitness,
                dominant_loss,
            },
            pipe: SnapshotV2Pipe {
                mode: "A_FAIL_CLOSED".to_string(),
                circuit_state: "unknown".to_string(),
                p99_ms: None,
                timeouts: 0,
            },
            dashboard_truth: SnapshotV2DashboardTruth {
                measured_only: true,
                stale_fields: Vec::new(),
            },
        })
    }

    /// Replay events after a sequence number, inclusive of greater-than only.
    ///
    /// # Errors
    ///
    /// Returns an error if rows cannot be queried or decoded.
    pub fn replay_since(&self, seq: i64) -> Result<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, event_id, trace_id, parent_id, kind, actor, payload_json,
                    created_at, hash, prev_hash, schema_version
             FROM event_log
             WHERE seq > ?1
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![seq], row_from_sql)?;
        collect_rows(rows)
    }

    /// Return all events for a trace id.
    ///
    /// # Errors
    ///
    /// Returns an error if the trace id is empty or rows cannot be queried.
    pub fn events_for_trace(&self, trace_id: &str) -> Result<Vec<EventRow>> {
        validate_non_empty("trace_id", trace_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT seq, event_id, trace_id, parent_id, kind, actor, payload_json,
                    created_at, hash, prev_hash, schema_version
             FROM event_log
             WHERE trace_id = ?1
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![trace_id], row_from_sql)?;
        collect_rows(rows)
    }

    /// Verify the full hash chain.
    ///
    /// # Errors
    ///
    /// Returns an error if replay fails or any sequence, previous-hash, or row
    /// hash invariant is violated.
    pub fn verify_chain(&self) -> Result<()> {
        let rows = self.replay_since(0)?;
        let mut expected_prev = GENESIS_HASH.to_string();
        for (expected_seq, row) in (1_i64..).zip(rows) {
            if row.seq != expected_seq {
                return Err(KernelError::ChainViolation(format!(
                    "expected seq {expected_seq}, got {}",
                    row.seq
                )));
            }
            let actual_prev = row.prev_hash.as_deref().unwrap_or(GENESIS_HASH);
            if actual_prev != expected_prev {
                return Err(KernelError::ChainViolation(format!(
                    "seq {} prev_hash mismatch",
                    row.seq
                )));
            }
            let canonical = CanonicalEvent {
                event_id: &row.event_id,
                trace_id: &row.trace_id,
                parent_id: row.parent_id.as_deref(),
                kind: &row.kind,
                actor: &row.actor,
                payload_json: &row.payload_json,
                created_at: &row.created_at,
                prev_hash: row.prev_hash.as_deref(),
                schema_version: row.schema_version,
            }
            .to_json()?;
            let expected_hash = hash_event(&expected_prev, &canonical);
            if row.hash != expected_hash {
                return Err(KernelError::ChainViolation(format!(
                    "seq {} hash mismatch",
                    row.seq
                )));
            }
            expected_prev = row.hash;
        }
        Ok(())
    }

    fn event_by_seq(&self, seq: i64) -> Result<EventRow> {
        self.conn
            .query_row(
                "SELECT seq, event_id, trace_id, parent_id, kind, actor, payload_json,
                    created_at, hash, prev_hash, schema_version
             FROM event_log
             WHERE seq = ?1",
                params![seq],
                row_from_sql,
            )
            .map_err(KernelError::from)
    }

    fn begin_immediate(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(())
    }

    fn finish_transaction<T>(&self, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    fn event_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM event_log", [], |row| row.get(0))
            .map_err(KernelError::from)
    }

    fn last_hash(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT hash FROM event_log ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(KernelError::from)
    }

    fn last_seq_hash(&self) -> Result<Option<(i64, String)>> {
        self.conn
            .query_row(
                "SELECT seq, hash FROM event_log ORDER BY seq DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(KernelError::from)
    }

    fn last_event_id(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT event_id FROM event_log ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(KernelError::from)
    }

    fn idempotency_record(&self, key: &str) -> Result<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT request_hash, response_json FROM idempotency_records WHERE idempotency_key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(KernelError::from)
    }

    fn record_idempotency(&self, key: &str, response: &SubmitResponse) -> Result<()> {
        let now = now_timestamp()?;
        let response_json = canonical_json(&serde_json::to_value(response)?)?;
        self.conn.execute(
            "INSERT INTO idempotency_records
             (idempotency_key, request_hash, first_trace, response_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key,
                response.request_hash,
                response.trace_id,
                response_json,
                now,
                now,
            ],
        )?;
        Ok(())
    }

    fn record_warrant(
        &self,
        request: &SubmitRequest,
        request_hash: &str,
        policy: &PolicyDecision,
    ) -> Result<String> {
        let ts = epoch_nanos()?;
        let warrant_id = format!("warrant_{ts}");
        self.conn.execute(
            "INSERT INTO warrants
             (warrant_id, trace_id, class, verdict, reason, policy_ref, policy_version,
              policy_hash, request_hash, justfile_hash, recipe_body_hash,
              decision_before_execution, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, 1, ?10)",
            params![
                warrant_id,
                request.trace_id,
                &policy.class,
                "ALLOW",
                &policy.reason,
                &policy.policy_ref,
                &policy.policy_version,
                &policy.policy_hash,
                request_hash,
                now_timestamp()?,
            ],
        )?;
        Ok(warrant_id)
    }

    fn record_message(&self, request: &SubmitRequest) -> Result<()> {
        let now = now_timestamp()?;
        self.conn.execute(
            "INSERT INTO messages
             (msg_id, trace_id, kind, from_role, to_role, idempotency_key, state,
              integration_state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'INGESTED', 'INGESTED', ?7, ?8)",
            params![
                format!(
                    "msg_{}",
                    hash_text(&request.idempotency_key).trim_start_matches("sha256:")
                ),
                request.trace_id,
                request.kind,
                request.operator,
                "orchestrator_kernel",
                request.idempotency_key,
                now,
                now,
            ],
        )?;
        Ok(())
    }

    fn record_edge(
        &self,
        edge_id: &str,
        trace_id: &str,
        edge: &str,
        state: &str,
        evidence_ref: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO edge_coherence
             (edge_id, trace_id, edge, state, observed_at, evidence_ref)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                edge_id,
                trace_id,
                edge,
                state,
                now_timestamp()?,
                evidence_ref
            ],
        )?;
        Ok(())
    }

    fn execute_verify_chain_recipe(
        &self,
        request: &SubmitRequest,
        admitted_event: &EventRow,
        warrant_id: &str,
    ) -> Result<BuiltinRecipeResult> {
        let run_id = format!("run_{}", epoch_nanos()?);
        let started_at = now_timestamp()?;
        let run_event = self.append_event_unlocked(&AppendEvent {
            kind: "RECIPE_RUN_STARTED".to_string(),
            trace_id: request.trace_id.clone(),
            parent_id: Some(admitted_event.event_id.clone()),
            actor: "orchestrator_kernel_sidecar".to_string(),
            payload: json!({
                "run_id": run_id,
                "recipe": BUILTIN_VERIFY_CHAIN,
                "warrant_id": warrant_id,
                "mode": "builtin_no_shell"
            }),
        })?;
        self.record_edge(
            &format!("edge_{run_id}_warrant_to_run"),
            &request.trace_id,
            "warrant_to_run",
            "MEASURED",
            Some(warrant_id),
        )?;

        let semantic = match self.verify_chain() {
            Ok(()) => ("VERIFIED", "hash chain verified after run start", 0),
            Err(_err) => ("FAILED", "hash chain verification failed", 1_i32),
        };
        let (semantic_state, semantic_reason, exit_code) = semantic;
        let result_event = self.append_event_unlocked(&AppendEvent {
            kind: "RESULT_VERIFIED".to_string(),
            trace_id: request.trace_id.clone(),
            parent_id: Some(run_event.event_id.clone()),
            actor: "orchestrator_kernel_sidecar".to_string(),
            payload: json!({
                "run_id": run_id,
                "recipe": BUILTIN_VERIFY_CHAIN,
                "semantic_state": semantic_state,
                "semantic_reason": semantic_reason,
                "exit_code": exit_code
            }),
        })?;

        self.record_recipe_run(&RecipeRunRecord {
            run_id: &run_id,
            warrant_id,
            request,
            semantic_state,
            semantic_reason,
            exit_code,
            started_at: &started_at,
        })?;
        self.update_message_integrated(&request.idempotency_key, semantic_state)?;
        self.record_edge(
            &format!("edge_{run_id}_run_to_result"),
            &request.trace_id,
            "run_to_result",
            "MEASURED",
            Some(&result_event.event_id),
        )?;
        self.record_edge(
            &format!("edge_{run_id}_result_to_replay_dashboard"),
            &request.trace_id,
            "result_to_replay_dashboard",
            "MEASURED",
            Some(&result_event.event_id),
        )?;

        Ok(BuiltinRecipeResult {
            run_id,
            result_event_id: result_event.event_id,
        })
    }

    fn record_recipe_run(&self, record: &RecipeRunRecord<'_>) -> Result<()> {
        let finished_at = now_timestamp()?;
        self.conn.execute(
            "INSERT INTO recipe_runs
             (run_id, trace_id, recipe, args_json, exit_code, semantic_state,
              semantic_reason, stdout_path, stderr_path, stdout_truncated,
             stderr_truncated, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, 0, 0, ?8, ?9)",
            params![
                record.run_id,
                record.request.trace_id,
                BUILTIN_VERIFY_CHAIN,
                canonical_json(&json!({"mode": "builtin_no_shell"}))?,
                record.exit_code,
                record.semantic_state,
                record.semantic_reason,
                record.started_at,
                finished_at,
            ],
        )?;
        self.conn.execute(
            "INSERT INTO recipe_run_warrants (run_id, warrant_id, linked_at)
             VALUES (?1, ?2, ?3)",
            params![record.run_id, record.warrant_id, finished_at],
        )?;
        Ok(())
    }

    fn update_message_integrated(&self, idempotency_key: &str, semantic_state: &str) -> Result<()> {
        let integration_state = if semantic_state == "VERIFIED" {
            "INTEGRATED"
        } else {
            "FAILED"
        };
        self.conn.execute(
            "UPDATE messages
             SET state = ?1, integration_state = ?2, updated_at = ?3
             WHERE idempotency_key = ?4",
            params![
                semantic_state,
                integration_state,
                now_timestamp()?,
                idempotency_key,
            ],
        )?;
        Ok(())
    }

    fn edge_snapshots(&self) -> Result<Vec<EdgeSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT edge, state, observed_at, evidence_ref
             FROM edge_coherence
             ORDER BY observed_at DESC, edge ASC
             LIMIT 20",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(EdgeSnapshot {
                edge: row.get(0)?,
                state: row.get(1)?,
                observed_at: row.get(2)?,
                evidence_ref: row.get(3)?,
            })
        })?;
        let mut edges = Vec::new();
        for row in rows {
            edges.push(row?);
        }
        Ok(edges)
    }

    fn warrant_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM warrants", [], |row| row.get(0))
            .map_err(KernelError::from)
    }

    fn queue_depth(&self) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE integration_state != 'INTEGRATED'",
                [],
                |row| row.get(0),
            )
            .map_err(KernelError::from)
    }
}

/// Parse a `JSON` payload string into a value.
///
/// # Errors
///
/// Returns an error when `input` is not valid `JSON`.
pub fn parse_payload(input: &str) -> Result<Value> {
    serde_json::from_str(input).map_err(KernelError::from)
}

/// Serialize a value as canonical `JSON` with sorted object keys.
///
/// # Errors
///
/// Returns an error if the normalized value cannot be serialized.
pub fn canonical_json(value: &Value) -> Result<String> {
    let normalized = normalize_json(value);
    serde_json::to_string(&normalized).map_err(KernelError::from)
}

fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(normalize_json).collect()),
        Value::Object(map) => {
            let mut normalized = Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(item) = map.get(key) {
                    normalized.insert(key.clone(), normalize_json(item));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

struct CanonicalEvent<'a> {
    event_id: &'a str,
    trace_id: &'a str,
    parent_id: Option<&'a str>,
    kind: &'a str,
    actor: &'a str,
    payload_json: &'a str,
    created_at: &'a str,
    prev_hash: Option<&'a str>,
    schema_version: i64,
}

impl CanonicalEvent<'_> {
    fn to_json(&self) -> Result<String> {
        let payload: Value = serde_json::from_str(self.payload_json)?;
        let value = json!({
            "actor": self.actor,
            "created_at": self.created_at,
            "event_id": self.event_id,
            "kind": self.kind,
            "parent_id": self.parent_id,
            "payload": payload,
            "prev_hash": self.prev_hash,
            "schema_version": self.schema_version,
            "trace_id": self.trace_id
        });
        canonical_json(&value)
    }
}

fn hash_event(prev_hash: &str, canonical_event: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(canonical_event.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        seq: row.get(0)?,
        event_id: row.get(1)?,
        trace_id: row.get(2)?,
        parent_id: row.get(3)?,
        kind: row.get(4)?,
        actor: row.get(5)?,
        payload_json: row.get(6)?,
        created_at: row.get(7)?,
        hash: row.get(8)?,
        prev_hash: row.get(9)?,
        schema_version: row.get(10)?,
    })
}

fn collect_rows<F>(mapped: rusqlite::MappedRows<'_, F>) -> Result<Vec<EventRow>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<EventRow>,
{
    let mut rows = Vec::new();
    for row in mapped {
        rows.push(row?);
    }
    Ok(rows)
}

fn validate_event_kind(kind: &str) -> Result<()> {
    validate_non_empty("kind", kind)?;
    if is_upper_snake_case(kind) || is_dotted_namespace(kind) {
        Ok(())
    } else {
        Err(KernelError::InvalidInput(format!(
            "event kind must be UPPER_SNAKE_CASE or dotted.namespace.kind, got {kind:?}"
        )))
    }
}

/// Returns `true` when `kind` consists entirely of ASCII uppercase letters,
/// digits, and underscores (e.g., `HEARTBEAT`, `TASK_INGESTED`).
fn is_upper_snake_case(kind: &str) -> bool {
    !kind.is_empty()
        && kind
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit())
}

/// Returns `true` when `kind` is a dotted-namespace kind with at least two
/// segments (e.g., `perceive.snapshot`, `result.verified`).  Each segment
/// must start with an ASCII lowercase letter and contain only ASCII lowercase
/// letters, digits, and underscores.
fn is_dotted_namespace(kind: &str) -> bool {
    let segments: Vec<&str> = kind.split('.').collect();
    // Require at least one dot so that plain lowercase words are rejected.
    if segments.len() < 2 {
        return false;
    }
    segments.iter().all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
            && segment
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch.is_ascii_digit())
    })
}

fn validate_submit_request(request: &SubmitRequest) -> Result<()> {
    if request.schema != SUBMIT_REQUEST_SCHEMA {
        return Err(KernelError::InvalidInput(format!(
            "submit schema must be {SUBMIT_REQUEST_SCHEMA:?}, got {:?}",
            request.schema
        )));
    }
    validate_non_empty("trace_id", &request.trace_id)?;
    validate_non_empty("idempotency_key", &request.idempotency_key)?;
    validate_non_empty("operator", &request.operator)?;
    if request.kind != "TASK" {
        return Err(KernelError::InvalidInput(format!(
            "submit kind must be TASK, got {:?}",
            request.kind
        )));
    }
    if let Some(recipe) = request.requested_recipe.as_deref() {
        if recipe != BUILTIN_VERIFY_CHAIN {
            return Err(KernelError::InvalidInput(format!(
                "unsupported requested_recipe {recipe:?}; supported built-ins: {BUILTIN_VERIFY_CHAIN}"
            )));
        }
    }
    Ok(())
}

fn resolve_policy_decision(request: &SubmitRequest) -> Result<PolicyDecision> {
    let config = load_default_policy_config()?;
    validate_policy_hash(&config)?;

    let rule = match request.requested_recipe.as_deref() {
        Some(BUILTIN_VERIFY_CHAIN) => config
            .rules
            .iter()
            .find(|rule| {
                rule.id == "builtin-verify-chain"
                    && rule.verdict == "ALLOW"
                    && rule.recipe.as_deref() == Some(BUILTIN_VERIFY_CHAIN)
            })
            .ok_or_else(|| {
                KernelError::InvalidInput(
                    "policy missing ALLOW rule for built-in verify_chain".to_string(),
                )
            })?,
        Some(recipe) => {
            return Err(KernelError::InvalidInput(format!(
                "unsupported requested_recipe {recipe:?}; fixed allowlist only permits {BUILTIN_VERIFY_CHAIN:?}"
            )));
        }
        None => config
            .rules
            .iter()
            .find(|rule| rule.id == "durable-submit-admission" && rule.verdict == "ALLOW")
            .ok_or_else(|| {
                KernelError::InvalidInput(
                    "policy missing ALLOW rule for durable submit admission".to_string(),
                )
            })?,
    };

    let reason = rule.reason.clone().unwrap_or_else(|| {
        if request.requested_recipe.as_deref() == Some(BUILTIN_VERIFY_CHAIN) {
            "built-in verify_chain recipe allowed; no shell execution".to_string()
        } else {
            "durable admission only; recipe execution deferred".to_string()
        }
    });

    let class = rule.class.clone();
    Ok(PolicyDecision {
        class,
        reason,
        policy_ref: config.policy_ref.clone(),
        policy_version: config.policy_version.clone(),
        policy_hash: config.policy_hash.clone(),
    })
}

fn load_default_policy_config() -> Result<PolicyConfig> {
    let policy_path = default_policy_path()?;
    let raw = fs::read_to_string(&policy_path)?;
    serde_json::from_str(&raw).map_err(KernelError::from)
}

fn default_policy_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("ORCH_KERNEL_POLICY_PATH") {
        return Ok(PathBuf::from(path));
    }

    let cwd = env::current_dir()?;
    let workspace = find_workspace_root(&cwd).ok_or_else(|| {
        KernelError::InvalidInput(format!(
            "cannot locate workspace root containing {DEFAULT_POLICY_REF}; set ORCH_KERNEL_POLICY_PATH"
        ))
    })?;
    Ok(workspace.join(DEFAULT_POLICY_REF))
}

fn validate_policy_hash(config: &PolicyConfig) -> Result<()> {
    let policy_path = default_policy_path()?;
    let value: Value = serde_json::from_str(&fs::read_to_string(policy_path)?)?;
    validate_policy_config_value(config, &value)
}

fn validate_policy_config_value(config: &PolicyConfig, value: &Value) -> Result<()> {
    if config.policy_ref != DEFAULT_POLICY_REF {
        return Err(KernelError::InvalidInput(format!(
            "policy_ref must be {DEFAULT_POLICY_REF:?}, got {:?}",
            config.policy_ref
        )));
    }
    if config.policy_version != DEFAULT_POLICY_VERSION {
        return Err(KernelError::InvalidInput(format!(
            "policy_version must be {DEFAULT_POLICY_VERSION:?}, got {:?}",
            config.policy_version
        )));
    }
    if !config.policy_hash.starts_with("sha256:") || config.policy_hash.contains("placeholder") {
        return Err(KernelError::InvalidInput(format!(
            "policy_hash must be a concrete sha256 digest, got {:?}",
            config.policy_hash
        )));
    }
    if config.policy_hash != DEFAULT_POLICY_HASH {
        return Err(KernelError::InvalidInput(format!(
            "policy_hash drift: sidecar expected {DEFAULT_POLICY_HASH}, config has {}",
            config.policy_hash
        )));
    }
    let mut value = value.clone();
    if let Value::Object(map) = &mut value {
        map.insert("policy_hash".to_string(), Value::Null);
    } else {
        return Err(KernelError::InvalidInput(
            "policy config must be a JSON object".to_string(),
        ));
    }
    let expected = hash_text(&canonical_json(&value)?);
    if expected != config.policy_hash {
        return Err(KernelError::InvalidInput(format!(
            "policy_hash mismatch: canonical config is {expected}, stored is {}",
            config.policy_hash
        )));
    }
    Ok(())
}

fn required_edges() -> &'static [&'static str] {
    &[
        "submit_to_event",
        "event_to_warrant",
        "warrant_to_run",
        "run_to_result",
        "result_to_replay_dashboard",
    ]
}

fn submit_request_hash(request: &SubmitRequest) -> Result<String> {
    let value = json!({
        "schema": request.schema,
        "kind": request.kind,
        "operator": request.operator,
        "requested_recipe": request.requested_recipe,
        "payload": request.payload
    });
    Ok(hash_text(&canonical_json(&value)?))
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(KernelError::InvalidInput(format!(
            "{field} cannot be empty"
        )))
    } else {
        Ok(())
    }
}

fn make_event_id(kind: &str) -> Result<String> {
    let ts = epoch_nanos()?;
    Ok(format!("evt_{ts}_{kind}"))
}

fn now_timestamp() -> Result<String> {
    Ok(format!("{}Z", epoch_millis()?))
}

fn epoch_millis() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|err| KernelError::InvalidInput(format!("system clock before epoch: {err}")))
}

fn epoch_nanos() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|err| KernelError::InvalidInput(format!("system clock before epoch: {err}")))
}

fn hash_text(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn default_operator() -> String {
    "orch-kernelctl".to_string()
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(path) = current {
        if path.join("CLAUDE.md").exists() && path.join("habitat-zellij").exists() {
            return Some(path.to_path_buf());
        }
        current = path.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::thread;

    fn temp_paths(name: &str) -> StatePaths {
        let mut dir = env::temp_dir();
        dir.push(format!(
            "orch_kernel_test_{}_{}",
            name,
            epoch_millis().unwrap()
        ));
        StatePaths::from_state_dir(dir)
    }

    #[test]
    fn canonical_json_sorts_nested_keys() {
        let value = json!({"z": 1, "a": {"d": 4, "b": 2}});
        assert_eq!(
            canonical_json(&value).unwrap(),
            r#"{"a":{"b":2,"d":4},"z":1}"#
        );
    }

    #[test]
    fn default_policy_hash_matches_canonical_config() {
        let policy = load_default_policy_config().unwrap();
        validate_policy_hash(&policy).unwrap();
        assert_eq!(policy.policy_hash, DEFAULT_POLICY_HASH);
    }

    #[test]
    fn policy_hash_rejects_placeholder_or_drift() {
        let policy_path = default_policy_path().unwrap();
        let raw = fs::read_to_string(policy_path).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        let mut policy: PolicyConfig = serde_json::from_str(&raw).unwrap();
        policy.policy_hash = "sha256:policy-hash-recorded-in-config".to_string();

        let err = validate_policy_config_value(&policy, &value).unwrap_err();

        assert!(matches!(err, KernelError::InvalidInput(_)));
    }

    #[test]
    fn builtin_verify_chain_policy_decision_is_fixed_allowlist() {
        let mut request =
            submit_request("trace-policy-decision", "idem-policy-decision", json!({}));
        request.requested_recipe = Some(BUILTIN_VERIFY_CHAIN.to_string());

        let decision = resolve_policy_decision(&request).unwrap();

        assert_eq!(decision.class, "BUILTIN_RECIPE");
        assert_eq!(decision.policy_hash, DEFAULT_POLICY_HASH);
        assert!(decision.reason.contains("verify_chain"));
    }

    #[test]
    fn append_snapshot_replay_and_verify_chain() {
        let paths = temp_paths("append_snapshot_replay");
        let log = EventLog::open(&paths).unwrap();
        let row = log
            .append_event(&AppendEvent {
                kind: "HEARTBEAT".into(),
                trace_id: "trace-1".into(),
                parent_id: None,
                actor: "test".into(),
                payload: json!({"ok": true}),
            })
            .unwrap();
        assert_eq!(row.seq, 1);
        assert_eq!(row.prev_hash.as_deref(), Some(GENESIS_HASH));
        assert!(row.hash.starts_with("sha256:"));

        let snapshot = log.snapshot().unwrap();
        assert_eq!(snapshot.event_count, 1);
        assert!(snapshot.verify_chain_ok);

        let replay = log.replay_since(0).unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].trace_id, "trace-1");
        log.verify_chain().unwrap();
    }

    #[test]
    fn events_for_trace_filters_rows() {
        let paths = temp_paths("trace_filter");
        let log = EventLog::open(&paths).unwrap();
        for trace_id in ["trace-a", "trace-b"] {
            log.append_event(&AppendEvent {
                kind: "HEARTBEAT".into(),
                trace_id: trace_id.into(),
                parent_id: None,
                actor: "test".into(),
                payload: json!({"trace": trace_id}),
            })
            .unwrap();
        }
        let trace_a = log.events_for_trace("trace-a").unwrap();
        assert_eq!(trace_a.len(), 1);
        assert_eq!(trace_a[0].trace_id, "trace-a");
    }

    #[test]
    fn invalid_kind_is_rejected() {
        let paths = temp_paths("invalid_kind");
        let log = EventLog::open(&paths).unwrap();
        let err = log
            .append_event(&AppendEvent {
                kind: "bad-kind".into(),
                trace_id: "trace-1".into(),
                parent_id: None,
                actor: "test".into(),
                payload: Value::Null,
            })
            .unwrap_err();
        assert!(matches!(err, KernelError::InvalidInput(_)));
    }

    #[test]
    fn submit_ack_durable_includes_event_id_and_hash() {
        let paths = temp_paths("submit_ack");
        let log = EventLog::open(&paths).unwrap();
        let response = log
            .submit(&submit_request("trace-submit", "idem-1", json!({"a": 1})))
            .unwrap();

        assert_eq!(response.verdict, SubmitVerdict::AckDurable);
        assert_eq!(response.idempotency, IdempotencyState::New);
        assert_eq!(response.integration_state, "INGESTED");
        assert!(response
            .event_id
            .as_deref()
            .is_some_and(|id| id.starts_with("evt_")));
        assert!(response
            .event_hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("sha256:")));

        let snapshot = log.snapshot().unwrap();
        assert_eq!(snapshot.event_count, 1);
        assert_eq!(snapshot.warrant_count, 1);
        assert_eq!(snapshot.queue_depth, 1);
        assert!(snapshot
            .edges
            .iter()
            .any(|edge| edge.edge == "submit_to_event"));
        assert!(snapshot
            .edges
            .iter()
            .any(|edge| edge.edge == "event_to_warrant"));
    }

    #[test]
    fn submit_replays_same_idempotency_key() {
        let paths = temp_paths("submit_replay");
        let log = EventLog::open(&paths).unwrap();
        let request = submit_request("trace-submit", "idem-1", json!({"a": 1}));

        let first = log.submit(&request).unwrap();
        let second = log.submit(&request).unwrap();

        assert_eq!(first.verdict, SubmitVerdict::AckDurable);
        assert_eq!(second.verdict, SubmitVerdict::AckDurable);
        assert_eq!(second.idempotency, IdempotencyState::Replay);
        assert_eq!(first.event_id, second.event_id);
        assert_eq!(log.snapshot().unwrap().event_count, 1);
    }

    #[test]
    fn concurrent_submits_preserve_hash_chain() {
        let paths = temp_paths("submit_concurrent");
        let log = EventLog::open(&paths).unwrap();
        drop(log);

        let mut handles = Vec::new();
        for worker in 0..8 {
            let paths = paths.clone();
            handles.push(thread::spawn(move || {
                let log = EventLog::open(&paths).unwrap();
                for item in 0..25 {
                    let request = submit_request(
                        &format!("trace-concurrent-{worker}-{item}"),
                        &format!("idem-concurrent-{worker}-{item}"),
                        json!({"worker": worker, "item": item}),
                    );
                    let response = log.submit(&request).unwrap();
                    assert_eq!(response.verdict, SubmitVerdict::AckDurable);
                    assert!(response.event_hash.is_some());
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let log = EventLog::open(&paths).unwrap();
        log.verify_chain().unwrap();
        assert_eq!(log.snapshot().unwrap().event_count, 200);
    }

    #[test]
    fn concurrent_raw_appends_preserve_hash_chain() {
        let paths = temp_paths("append_concurrent");
        let log = EventLog::open(&paths).unwrap();
        drop(log);

        let mut handles = Vec::new();
        for worker in 0..6 {
            let paths = paths.clone();
            handles.push(thread::spawn(move || {
                let log = EventLog::open(&paths).unwrap();
                for item in 0..30 {
                    let row = log
                        .append_event(&AppendEvent {
                            kind: "HEARTBEAT".into(),
                            trace_id: format!("trace-append-{worker}-{item}"),
                            parent_id: None,
                            actor: "raw-append-regression".into(),
                            payload: json!({"worker": worker, "item": item}),
                        })
                        .unwrap();
                    assert!(row.hash.starts_with("sha256:"));
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let log = EventLog::open(&paths).unwrap();
        log.verify_chain().unwrap();
        let replay = log.replay_since(0).unwrap();
        assert_eq!(replay.len(), 180);
        assert_eq!(replay.first().unwrap().seq, 1);
        assert_eq!(replay.last().unwrap().seq, 180);
        let mut hashes = BTreeSet::new();
        let mut expected_prev = GENESIS_HASH.to_string();
        for row in replay {
            assert_eq!(row.prev_hash.as_deref(), Some(expected_prev.as_str()));
            assert!(
                hashes.insert(row.hash.clone()),
                "duplicate hash {}",
                row.hash
            );
            expected_prev = row.hash;
        }
    }

    #[test]
    fn concurrent_same_idempotency_key_admits_once_and_replays() {
        let paths = temp_paths("submit_same_idem_concurrent");
        let log = EventLog::open(&paths).unwrap();
        drop(log);

        let mut handles = Vec::new();
        for worker in 0..12 {
            let paths = paths.clone();
            handles.push(thread::spawn(move || {
                let log = EventLog::open(&paths).unwrap();
                let request = submit_request(
                    "trace-same-idem",
                    "idem-same-concurrent",
                    json!({"same": true, "payload": "stable"}),
                );
                let response = log.submit(&request).unwrap();
                assert_eq!(response.verdict, SubmitVerdict::AckDurable, "{worker}");
                assert!(matches!(
                    response.idempotency,
                    IdempotencyState::New | IdempotencyState::Replay
                ));
                response
            }));
        }

        let responses: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let new_count = responses
            .iter()
            .filter(|response| response.idempotency == IdempotencyState::New)
            .count();
        let event_ids: BTreeSet<_> = responses
            .iter()
            .map(|response| response.event_id.clone())
            .collect();

        assert_eq!(new_count, 1);
        assert_eq!(event_ids.len(), 1);

        let log = EventLog::open(&paths).unwrap();
        log.verify_chain().unwrap();
        assert_eq!(log.snapshot().unwrap().event_count, 1);
    }

    #[test]
    fn submit_conflicts_on_same_key_different_hash() {
        let paths = temp_paths("submit_conflict");
        let log = EventLog::open(&paths).unwrap();

        let first = log
            .submit(&submit_request("trace-a", "idem-1", json!({"a": 1})))
            .unwrap();
        let second = log
            .submit(&submit_request("trace-a", "idem-1", json!({"a": 2})))
            .unwrap();

        assert_eq!(first.verdict, SubmitVerdict::AckDurable);
        assert_eq!(second.verdict, SubmitVerdict::Nack);
        assert_eq!(second.idempotency, IdempotencyState::Conflict);
        assert_eq!(second.event_id, None);
        assert_eq!(second.event_hash, None);
        assert_eq!(log.snapshot().unwrap().event_count, 1);
    }

    #[test]
    fn submit_ack_records_policy_bound_warrant_before_execution() {
        let paths = temp_paths("submit_warrant_policy");
        let log = EventLog::open(&paths).unwrap();
        let mut request = submit_request("trace-policy", "idem-policy", json!({"goal": "policy"}));
        request.requested_recipe = Some(BUILTIN_VERIFY_CHAIN.to_string());

        let response = log.submit(&request).unwrap();

        assert_eq!(response.verdict, SubmitVerdict::AckDurable);
        assert!(response.event_hash.is_some());
        assert!(response.run_id.is_some());

        let (class, verdict, policy_ref, policy_version, policy_hash, request_hash, decision): (
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
        ) = log
            .conn
            .query_row(
                "SELECT class, verdict, policy_ref, policy_version, policy_hash, request_hash,
                        decision_before_execution
                 FROM warrants
                 WHERE trace_id = ?1",
                params![request.trace_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(class, "BUILTIN_RECIPE");
        assert_eq!(verdict, "ALLOW");
        assert_eq!(policy_ref, DEFAULT_POLICY_REF);
        assert_eq!(policy_version, DEFAULT_POLICY_VERSION);
        assert!(policy_hash.starts_with("sha256:"));
        assert_eq!(request_hash, response.request_hash);
        assert_eq!(decision, 1);
    }

    #[test]
    fn submit_request_hash_is_canonical() {
        let paths = temp_paths("submit_canonical");
        let log = EventLog::open(&paths).unwrap();

        let left = log
            .submit(&submit_request(
                "trace-a",
                "idem-left",
                json!({"z": 1, "a": 2}),
            ))
            .unwrap();
        let right = log
            .submit(&submit_request(
                "trace-b",
                "idem-right",
                json!({"a": 2, "z": 1}),
            ))
            .unwrap();

        assert_eq!(left.request_hash, right.request_hash);
    }

    #[test]
    fn submit_builtin_verify_chain_completes_full_edge_stack() {
        let paths = temp_paths("submit_builtin_verify");
        let log = EventLog::open(&paths).unwrap();
        let mut request = submit_request("trace-e2e", "idem-e2e", json!({"goal": "full-e2e"}));
        request.requested_recipe = Some(BUILTIN_VERIFY_CHAIN.to_string());

        let response = log.submit(&request).unwrap();

        assert_eq!(response.verdict, SubmitVerdict::AckDurable);
        assert_eq!(response.integration_state, "INTEGRATED");
        assert_eq!(response.reason, "BUILTIN_VERIFY_CHAIN_INTEGRATED");
        assert!(response
            .run_id
            .as_deref()
            .is_some_and(|id| id.starts_with("run_")));
        assert!(response
            .result_event_id
            .as_deref()
            .is_some_and(|id| id.contains("RESULT_VERIFIED")));

        let snapshot = log.snapshot().unwrap();
        assert_eq!(snapshot.event_count, 3);
        assert_eq!(snapshot.queue_depth, 0);
        for edge in [
            "submit_to_event",
            "event_to_warrant",
            "warrant_to_run",
            "run_to_result",
            "result_to_replay_dashboard",
        ] {
            assert!(
                snapshot.edges.iter().any(|item| item.edge == edge),
                "{edge}"
            );
        }
        log.verify_chain().unwrap();
    }

    #[test]
    fn snapshot_v2_emits_contract_shape() {
        let paths = temp_paths("snapshot_v2");
        let log = EventLog::open(&paths).unwrap();
        let mut request = submit_request("trace-v2", "idem-v2", json!({"goal": "snapshot-v2"}));
        request.requested_recipe = Some(BUILTIN_VERIFY_CHAIN.to_string());
        log.submit(&request).unwrap();

        let snapshot = log.snapshot_v2().unwrap();

        assert_eq!(snapshot.schema, "habitat.kernel.snapshot.v2");
        assert_eq!(snapshot.source, "orchestrator-kernel-sidecar");
        assert!(snapshot.sidecar.verify_chain_ok);
        assert!(snapshot.sidecar.last_event_id.is_some());
        assert!(snapshot.sidecar.last_event_hash.starts_with("sha256:"));
        assert!(snapshot.dashboard_truth.measured_only);
        assert_eq!(snapshot.pipe.mode, "A_FAIL_CLOSED");
    }

    #[test]
    fn submit_rejects_unknown_recipe_before_append() {
        let paths = temp_paths("submit_unknown_recipe");
        let log = EventLog::open(&paths).unwrap();
        let mut request = submit_request("trace-reject", "idem-reject", json!({}));
        request.requested_recipe = Some("shell:anything".to_string());

        let err = log.submit(&request).unwrap_err();

        assert!(matches!(err, KernelError::InvalidInput(_)));
        assert_eq!(log.snapshot().unwrap().event_count, 0);
    }

    #[test]
    fn submit_rejects_network_and_shell_recipes_before_append() {
        let paths = temp_paths("submit_reject_security");
        let log = EventLog::open(&paths).unwrap();

        for recipe in ["shell:rm -rf /", "network:curl", "../verify_chain"] {
            let mut request = submit_request(
                &format!("trace-reject-{recipe}"),
                &format!("idem-reject-{recipe}"),
                json!({}),
            );
            request.requested_recipe = Some(recipe.to_string());

            let err = log.submit(&request).unwrap_err();

            assert!(matches!(err, KernelError::InvalidInput(_)));
        }
        assert_eq!(log.snapshot().unwrap().event_count, 0);
        assert_eq!(log.snapshot().unwrap().warrant_count, 0);
    }

    fn submit_request(trace_id: &str, idempotency_key: &str, payload: Value) -> SubmitRequest {
        SubmitRequest {
            schema: SUBMIT_REQUEST_SCHEMA.to_string(),
            trace_id: trace_id.to_string(),
            idempotency_key: idempotency_key.to_string(),
            kind: "TASK".to_string(),
            operator: "test".to_string(),
            requested_recipe: None,
            payload,
        }
    }

    // ── FIBER-2: latest_event_of_kind + Snapshot.latest_perceive ────────────

    #[test]
    fn latest_event_of_kind_returns_none_on_empty_log() {
        let paths = temp_paths("lek_empty");
        let log = EventLog::open(&paths).unwrap();
        assert!(log.latest_event_of_kind("HEARTBEAT").unwrap().is_none());
    }

    #[test]
    fn latest_event_of_kind_rejects_empty_kind() {
        let paths = temp_paths("lek_empty_kind");
        let log = EventLog::open(&paths).unwrap();
        let err = log.latest_event_of_kind("").unwrap_err();
        assert!(matches!(err, KernelError::InvalidInput(_)));
    }

    #[test]
    fn latest_event_of_kind_rejects_whitespace_only_kind() {
        let paths = temp_paths("lek_ws_kind");
        let log = EventLog::open(&paths).unwrap();
        let err = log.latest_event_of_kind("   ").unwrap_err();
        assert!(matches!(err, KernelError::InvalidInput(_)));
    }

    #[test]
    fn latest_event_of_kind_returns_none_for_absent_kind() {
        let paths = temp_paths("lek_absent");
        let log = EventLog::open(&paths).unwrap();
        log.append_event(&AppendEvent {
            kind: "HEARTBEAT".into(),
            trace_id: "trace-absent".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({"ok": true}),
        })
        .unwrap();
        // A different kind — must return None.
        assert!(log.latest_event_of_kind("TASK_INGESTED").unwrap().is_none());
    }

    #[test]
    fn latest_event_of_kind_returns_most_recent_row() {
        let paths = temp_paths("lek_most_recent");
        let log = EventLog::open(&paths).unwrap();
        for i in 0_u32..5 {
            log.append_event(&AppendEvent {
                kind: "HEARTBEAT".into(),
                trace_id: format!("trace-{i}"),
                parent_id: None,
                actor: "test".into(),
                payload: json!({"iteration": i}),
            })
            .unwrap();
        }
        let latest = log.latest_event_of_kind("HEARTBEAT").unwrap().unwrap();
        // seq is 1-based; the 5th append is seq 5.
        assert_eq!(latest.seq, 5);
        let payload: serde_json::Value =
            serde_json::from_str(&latest.payload_json).unwrap();
        assert_eq!(payload["iteration"], 4);
    }

    #[test]
    fn latest_event_of_kind_does_not_perturb_last_seq_or_verify_chain() {
        let paths = temp_paths("lek_no_perturb");
        let log = EventLog::open(&paths).unwrap();
        log.append_event(&AppendEvent {
            kind: "HEARTBEAT".into(),
            trace_id: "trace-perturb".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({}),
        })
        .unwrap();
        let before = log.snapshot().unwrap();
        let _ = log.latest_event_of_kind("HEARTBEAT").unwrap();
        let after_seq = log.last_seq_hash().unwrap().map_or(0, |(s, _)| s);
        assert_eq!(before.last_seq, after_seq);
        log.verify_chain().unwrap();
    }

    #[test]
    fn latest_event_of_kind_returns_single_match_among_mixed_kinds() {
        let paths = temp_paths("lek_single_mixed");
        let log = EventLog::open(&paths).unwrap();
        for kind in ["HEARTBEAT", "RESULT_VERIFIED", "HEARTBEAT"] {
            log.append_event(&AppendEvent {
                kind: kind.into(),
                trace_id: "trace-mixed".into(),
                parent_id: None,
                actor: "test".into(),
                payload: json!({"kind": kind}),
            })
            .unwrap();
        }
        // Only one RESULT_VERIFIED was appended.
        let result = log
            .latest_event_of_kind("RESULT_VERIFIED")
            .unwrap()
            .unwrap();
        assert_eq!(result.kind, "RESULT_VERIFIED");
        assert_eq!(result.seq, 2);
    }

    #[test]
    fn latest_event_of_kind_is_case_sensitive() {
        let paths = temp_paths("lek_case");
        let log = EventLog::open(&paths).unwrap();
        log.append_event(&AppendEvent {
            kind: "HEARTBEAT".into(),
            trace_id: "trace-case".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({}),
        })
        .unwrap();
        // Querying the lowercase variant must return None.
        assert!(log.latest_event_of_kind("heartbeat.beat").unwrap().is_none());
    }

    #[test]
    fn dotted_namespace_kind_is_accepted_and_stored() {
        let paths = temp_paths("dotted_ns_store");
        let log = EventLog::open(&paths).unwrap();
        let row = log
            .append_event(&AppendEvent {
                kind: PERCEIVE_SNAPSHOT_KIND.into(),
                trace_id: "trace-perceive-ns".into(),
                parent_id: None,
                actor: "test-fiber".into(),
                payload: json!({"schema": "perceive.snapshot.v1", "captured_at_ms": 12345}),
            })
            .unwrap();
        assert_eq!(row.kind, PERCEIVE_SNAPSHOT_KIND);
        assert_eq!(row.seq, 1);
    }

    #[test]
    fn dotted_namespace_kind_validates_segment_rules() {
        let paths = temp_paths("dotted_ns_valid");
        let log = EventLog::open(&paths).unwrap();
        // Valid dotted-namespace kinds
        for kind in ["perceive.snapshot", "a.b", "result.v2.ok"] {
            log.append_event(&AppendEvent {
                kind: kind.into(),
                trace_id: "trace-valid".into(),
                parent_id: None,
                actor: "test".into(),
                payload: json!({}),
            })
            .unwrap();
        }
        // Invalid: hyphen, starts with digit, single word, empty segment
        for kind in ["bad-kind", "2bad.start", "single", "has..empty"] {
            let err = log
                .append_event(&AppendEvent {
                    kind: kind.into(),
                    trace_id: "trace-invalid".into(),
                    parent_id: None,
                    actor: "test".into(),
                    payload: json!({}),
                })
                .unwrap_err();
            assert!(
                matches!(err, KernelError::InvalidInput(_)),
                "expected InvalidInput for kind {kind:?}"
            );
        }
    }

    #[test]
    fn snapshot_latest_perceive_is_none_on_empty_log() {
        let paths = temp_paths("snap_lp_empty");
        let log = EventLog::open(&paths).unwrap();
        let snapshot = log.snapshot().unwrap();
        assert!(snapshot.latest_perceive.is_none());
        assert_eq!(snapshot.event_count, 0);
    }

    #[test]
    fn snapshot_latest_perceive_is_none_when_no_perceive_events() {
        let paths = temp_paths("snap_lp_no_perceive");
        let log = EventLog::open(&paths).unwrap();
        log.append_event(&AppendEvent {
            kind: "HEARTBEAT".into(),
            trace_id: "trace-np".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({"ok": true}),
        })
        .unwrap();
        let snapshot = log.snapshot().unwrap();
        // HEARTBEAT is not a perceive.snapshot event.
        assert!(snapshot.latest_perceive.is_none());
        // Chain remains valid.
        assert!(snapshot.verify_chain_ok);
    }

    #[test]
    fn snapshot_latest_perceive_parses_payload_of_single_perceive_event() {
        let paths = temp_paths("snap_lp_single");
        let log = EventLog::open(&paths).unwrap();
        let payload = json!({
            "schema": "perceive.snapshot.v1",
            "captured_at_ms": 99_999_u64,
            "source": "test-fiber"
        });
        log.append_event(&AppendEvent {
            kind: PERCEIVE_SNAPSHOT_KIND.into(),
            trace_id: "trace-perceive-single".into(),
            parent_id: None,
            actor: "test".into(),
            payload: payload.clone(),
        })
        .unwrap();
        let snapshot = log.snapshot().unwrap();
        let lp = snapshot.latest_perceive.unwrap();
        assert_eq!(lp["schema"], "perceive.snapshot.v1");
        assert_eq!(lp["captured_at_ms"], 99_999_u64);
        assert_eq!(lp["source"], "test-fiber");
    }

    #[test]
    fn snapshot_latest_perceive_returns_most_recent_of_several_perceive_events() {
        let paths = temp_paths("snap_lp_newest");
        let log = EventLog::open(&paths).unwrap();
        for i in 0_u32..4 {
            log.append_event(&AppendEvent {
                kind: PERCEIVE_SNAPSHOT_KIND.into(),
                trace_id: format!("trace-perceive-{i}"),
                parent_id: None,
                actor: "test".into(),
                payload: json!({"iteration": i}),
            })
            .unwrap();
        }
        let snapshot = log.snapshot().unwrap();
        let lp = snapshot.latest_perceive.unwrap();
        // Only the 4th (index 3) perceive event is surfaced.
        assert_eq!(lp["iteration"], 3);
    }

    #[test]
    fn snapshot_verify_chain_stays_green_after_perceive_appends() {
        let paths = temp_paths("snap_lp_chain");
        let log = EventLog::open(&paths).unwrap();
        for i in 0_u32..3 {
            log.append_event(&AppendEvent {
                kind: PERCEIVE_SNAPSHOT_KIND.into(),
                trace_id: format!("trace-chain-{i}"),
                parent_id: None,
                actor: "test".into(),
                payload: json!({"schema": "perceive.snapshot.v1", "i": i}),
            })
            .unwrap();
        }
        let snapshot = log.snapshot().unwrap();
        assert!(snapshot.verify_chain_ok);
        log.verify_chain().unwrap();
    }

    #[test]
    fn latest_event_of_kind_returns_correct_field_values() {
        let paths = temp_paths("lek_fields");
        let log = EventLog::open(&paths).unwrap();
        let appended = log
            .append_event(&AppendEvent {
                kind: PERCEIVE_SNAPSHOT_KIND.into(),
                trace_id: "trace-fields-check".into(),
                parent_id: None,
                actor: "fiber-perceive".into(),
                payload: json!({"check": "fields"}),
            })
            .unwrap();
        let latest = log
            .latest_event_of_kind(PERCEIVE_SNAPSHOT_KIND)
            .unwrap()
            .unwrap();
        assert_eq!(latest.seq, appended.seq);
        assert_eq!(latest.event_id, appended.event_id);
        assert_eq!(latest.kind, PERCEIVE_SNAPSHOT_KIND);
        assert_eq!(latest.actor, "fiber-perceive");
        assert_eq!(latest.trace_id, "trace-fields-check");
        assert!(latest.hash.starts_with("sha256:"));
        assert_eq!(latest.prev_hash.as_deref(), Some(GENESIS_HASH));
    }

    #[test]
    fn snapshot_latest_perceive_mixed_with_other_events() {
        let paths = temp_paths("snap_lp_mixed");
        let log = EventLog::open(&paths).unwrap();
        // Interleave other events with perceive events.
        log.append_event(&AppendEvent {
            kind: "HEARTBEAT".into(),
            trace_id: "trace-hb-1".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({}),
        })
        .unwrap();
        log.append_event(&AppendEvent {
            kind: PERCEIVE_SNAPSHOT_KIND.into(),
            trace_id: "trace-p-1".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({"pass": 1}),
        })
        .unwrap();
        log.append_event(&AppendEvent {
            kind: "HEARTBEAT".into(),
            trace_id: "trace-hb-2".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({}),
        })
        .unwrap();
        log.append_event(&AppendEvent {
            kind: PERCEIVE_SNAPSHOT_KIND.into(),
            trace_id: "trace-p-2".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({"pass": 2}),
        })
        .unwrap();
        log.append_event(&AppendEvent {
            kind: "RESULT_VERIFIED".into(),
            trace_id: "trace-rv".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({}),
        })
        .unwrap();
        let snapshot = log.snapshot().unwrap();
        let lp = snapshot.latest_perceive.unwrap();
        // Must be the second perceive event (pass=2), not any other kind.
        assert_eq!(lp["pass"], 2);
        assert_eq!(snapshot.event_count, 5);
        assert!(snapshot.verify_chain_ok);
    }

    // ── read-only open path ──────────────────────────────────────────────────

    #[test]
    fn open_read_only_reads_but_cannot_write() {
        let paths = temp_paths("ro_read");
        // Seed via a normal read-write open, then drop it (checkpoints the WAL).
        {
            let rw = EventLog::open(&paths).unwrap();
            rw.append_event(&AppendEvent {
                kind: "HEARTBEAT".into(),
                trace_id: "t".into(),
                parent_id: None,
                actor: "test".into(),
                payload: json!({"ok": true}),
            })
            .unwrap();
        }
        // A read-only open sees the committed data and can verify the chain …
        let ro = EventLog::open_read_only(&paths).unwrap();
        assert_eq!(ro.snapshot().unwrap().event_count, 1);
        assert!(ro.snapshot().unwrap().verify_chain_ok);
        // … but cannot mutate the durable log.
        let err = ro.append_event(&AppendEvent {
            kind: "HEARTBEAT".into(),
            trace_id: "t2".into(),
            parent_id: None,
            actor: "test".into(),
            payload: json!({}),
        });
        assert!(err.is_err());
    }

    #[test]
    fn open_read_only_does_not_create_a_missing_db() {
        let paths = temp_paths("ro_absent");
        // open_read_only must NOT create the database (unlike open()).
        assert!(EventLog::open_read_only(&paths).is_err());
        assert!(!paths.db_path.exists());
    }

    // --- --read-only CLI guard (read_only_allowed) — the previously-untested guard ---

    #[test]
    fn read_only_allowed_accepts_every_read_command() {
        for c in ["snapshot", "snapshot-v2", "verify-chain", "replay", "events"] {
            assert!(read_only_allowed(c), "{c} must be read-only-allowed");
        }
    }

    #[test]
    fn read_only_allowed_rejects_write_and_unknown_commands() {
        // fail-closed: mutating, unknown, empty, and near-miss prefixes all rejected.
        for c in ["init", "submit", "append", "unknown", "", "snap", "snapshotX", "events "] {
            assert!(!read_only_allowed(c), "{c:?} must NOT be read-only-allowed");
        }
    }

    #[test]
    fn read_only_commands_are_exactly_the_five_read_verbs() {
        assert_eq!(READ_ONLY_COMMANDS.len(), 5);
        for v in ["snapshot", "snapshot-v2", "verify-chain", "replay", "events"] {
            assert!(READ_ONLY_COMMANDS.contains(&v));
        }
    }

    #[test]
    fn open_read_only_serves_all_read_paths() {
        let paths = temp_paths("ro_reads");
        {
            let rw = EventLog::open(&paths).unwrap();
            rw.append_event(&AppendEvent {
                kind: "HEARTBEAT".into(),
                trace_id: "tr".into(),
                parent_id: None,
                actor: "t".into(),
                payload: json!({"n": 1}),
            })
            .unwrap();
            rw.append_event(&AppendEvent {
                kind: PERCEIVE_SNAPSHOT_KIND.into(),
                trace_id: "tr".into(),
                parent_id: None,
                actor: "t".into(),
                payload: json!({"panes": []}),
            })
            .unwrap();
        }
        // Every read path serves correctly from a read-only connection.
        let ro = EventLog::open_read_only(&paths).unwrap();
        assert!(ro.snapshot_v2().is_ok(), "snapshot_v2 must work read-only");
        assert_eq!(ro.replay_since(0).unwrap().len(), 2);
        assert_eq!(ro.events_for_trace("tr").unwrap().len(), 2);
        assert!(ro
            .latest_event_of_kind(PERCEIVE_SNAPSHOT_KIND)
            .unwrap()
            .is_some());
        ro.verify_chain().unwrap();
    }
}
