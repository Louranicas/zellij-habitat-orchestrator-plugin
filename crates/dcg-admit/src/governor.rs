//! Concurrency governance for DCG D6 delegated-write admission.
//!
//! Wraps delegated-write admission with layered governors. Two tiers:
//!
//! ## Tier-A (HARD — load-bearing)
//!
//! **FAIR-SEMAPHORE** — bounds the number of concurrently in-flight admissions
//! to a configured [`MaxInflight`] width and serves waiters in strict FIFO
//! order. No waiter can be starved by a later arrival. This tier is
//! unconditional and cannot be bypassed by any soft-governor.
//!
//! **TRANSPARENT-RETRY-FIRST** — retries [`crate::DcgError::Subprocess`]
//! (transient) up to [`MaxRetries`] times with a computable backoff schedule.
//! Non-transient errors ([`crate::DcgError::NotArmed`],
//! [`crate::DcgError::StaleFence`], [`crate::DcgError::Denied`]) are
//! surfaced immediately without retry.
//!
//! ## Tier-B (TUNABLE — measure-first, defaults are NOT proven)
//!
//! Every tier-B parameter is tagged **Measure-first default** in its rustdoc.
//! These are starting points that MUST be instrumented and adjusted against
//! observed habitat behaviour before being treated as correct.
//!
//! - **AIMD backpressure** — tracks an *effective width* initialised to
//!   [`MaxInflight`]; on congestion the width is multiplicatively decreased; on
//!   success it is additively increased toward [`MaxInflight`]. Operations that
//!   would exceed the effective width return [`crate::DcgError::Denied`] with
//!   reason `"aimd backpressure"`.
//!
//! - **Circuit-breaker** — opens after [`FailureThreshold`] consecutive
//!   failures. All operations in the open state return
//!   [`crate::DcgError::Denied`] with reason `"circuit open"`. After
//!   [`HalfOpenTimeoutMs`] milliseconds a single probe is allowed; success
//!   closes the breaker; failure re-opens it.
//!
//! - **Per-agent budget soft-kill** — accumulates per-agent cost. When an
//!   agent exceeds its [`BudgetLimit`] the call returns
//!   [`crate::DcgError::Denied`] with reason `"agent budget exceeded"`.
//!
//! - **Congestion throttle** — reads an injectable [`QueueDepthReader`]. When
//!   depth exceeds [`CongestionThreshold`] the AIMD effective width is
//!   immediately decreased before the operation is attempted.
//!
//! # Injectable seams
//!
//! [`MonotonicClock`] and [`Sleeper`] are injected so tests are fully
//! deterministic — no real sleeping, no wall-clock dependency. A
//! [`BackoffSchedule`] determines the delay sequence; tests inject one that
//! returns zero delays.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

use crate::error::DcgError;
use crate::Result;

// ─────────────────────────────────────────────────────────────────────────────
// Bounded newtypes
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum number of concurrently in-flight admissions (hard cap).
///
/// Values in `[1, 255]` are valid. Zero is rejected because a semaphore that
/// permanently blocks all callers is not a useful runtime state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MaxInflight(u8);

impl MaxInflight {
    /// Constructs a [`MaxInflight`] ceiling.
    ///
    /// # Errors
    /// Returns [`DcgError::OutOfRange`] if `value` is `0`.
    pub fn new(value: u8) -> Result<Self> {
        if value == 0 {
            return Err(DcgError::OutOfRange {
                field: "max_inflight",
                value: "0".to_string(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the underlying maximum.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Maximum number of retry attempts for transient failures.
///
/// Values in `[0, 255]` are all valid; `0` means "no retries — surface the
/// first failure". **Measure-first default:** `3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MaxRetries(u8);

impl MaxRetries {
    /// Constructs a [`MaxRetries`] bound. All `u8` values are valid.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the underlying limit.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Number of consecutive failures that open the circuit-breaker.
///
/// Values in `[1, u32::MAX]` are valid. **Measure-first default:** `5`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FailureThreshold(u32);

impl FailureThreshold {
    /// Constructs a [`FailureThreshold`].
    ///
    /// # Errors
    /// Returns [`DcgError::OutOfRange`] if `value` is `0`.
    pub fn new(value: u32) -> Result<Self> {
        if value == 0 {
            return Err(DcgError::OutOfRange {
                field: "failure_threshold",
                value: "0".to_string(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the underlying threshold.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Duration in milliseconds before the circuit-breaker transitions from open
/// to half-open and allows a single probe.
///
/// **Measure-first default:** `10_000` (10 s).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HalfOpenTimeoutMs(u64);

impl HalfOpenTimeoutMs {
    /// Constructs a [`HalfOpenTimeoutMs`]. All `u64` values are valid.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the timeout in milliseconds.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Dimensionless cost unit charged to a per-agent budget per operation.
///
/// The unit is application-defined; callers choose a value proportional to
/// the expected cost of the operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CostUnit(u64);

impl CostUnit {
    /// Constructs a [`CostUnit`]. All `u64` values are valid.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying cost.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Cumulative cost limit per agent before a soft-kill is applied.
///
/// **Measure-first default:** `1_000_000`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BudgetLimit(u64);

impl BudgetLimit {
    /// Constructs a [`BudgetLimit`]. All `u64` values are valid.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying limit.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Observed sidecar queue depth (number of pending items in the admission
/// queue at the time of reading).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueueDepth(u32);

impl QueueDepth {
    /// Constructs a [`QueueDepth`]. All `u32` values are valid.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the underlying depth.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Queue depth at or above which the congestion throttle is applied.
///
/// **Measure-first default:** `10`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CongestionThreshold(u32);

impl CongestionThreshold {
    /// Constructs a [`CongestionThreshold`]. All `u32` values are valid.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the underlying threshold.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Injectable seams — clock and sleeper
// ─────────────────────────────────────────────────────────────────────────────

/// Monotonic clock seam.
///
/// The production implementation reads [`std::time::Instant`]; tests inject a
/// controllable stub.
pub trait MonotonicClock: Send + Sync {
    /// Returns the current time as milliseconds since an arbitrary epoch.
    ///
    /// The value MUST be monotonically non-decreasing within a process.
    fn now_ms(&self) -> u64;
}

/// Production [`MonotonicClock`] backed by [`std::time::Instant`].
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    origin: std::time::Instant,
}

impl SystemClock {
    /// Creates a new clock anchored to the moment of construction.
    #[must_use]
    pub fn new() -> Self {
        Self { origin: std::time::Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock for SystemClock {
    fn now_ms(&self) -> u64 {
        // as_millis() returns u128; clamp to u64::MAX rather than truncating.
        let ms = self.origin.elapsed().as_millis();
        u64::try_from(ms).unwrap_or(u64::MAX)
    }
}

/// Thread-sleeper seam.
///
/// The production implementation calls [`std::thread::sleep`]; tests inject a
/// no-op stub so retry backoff does not stall the test suite.
pub trait Sleeper: Send + Sync {
    /// Sleeps for the requested duration.
    ///
    /// Tests MUST inject a no-op implementation; production uses the system
    /// sleep.
    fn sleep_ms(&self, ms: u64);
}

/// Production [`Sleeper`] backed by [`std::thread::sleep`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSleeper;

impl Sleeper for SystemSleeper {
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Backoff schedule
// ─────────────────────────────────────────────────────────────────────────────

/// Computes the delay to apply before the `attempt`-th retry (0-indexed).
///
/// The implementation MUST be pure — it only computes delays; it does not
/// sleep. Sleeping is handled by the injected [`Sleeper`].
pub trait BackoffSchedule: Send + Sync {
    /// Returns the delay in milliseconds before retry `attempt`.
    ///
    /// `attempt` is 0-indexed: `0` is the first retry, `1` the second, and so
    /// on. The implementation is free to return `0` (no delay).
    fn delay_ms(&self, attempt: u8) -> u64;
}

/// Exponential backoff with a configurable base and ceiling.
///
/// Delay for attempt `n` is `min(base_ms * 2^n, max_ms)`.
///
/// **Measure-first defaults:** `base_ms = 50`, `max_ms = 2000`.
#[derive(Clone, Copy, Debug)]
pub struct ExponentialBackoff {
    /// Base delay in milliseconds.
    ///
    /// **Measure-first default:** `50`.
    pub base_ms: u64,
    /// Maximum delay in milliseconds.
    ///
    /// **Measure-first default:** `2000`.
    pub max_ms: u64,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self { base_ms: 50, max_ms: 2_000 }
    }
}

impl BackoffSchedule for ExponentialBackoff {
    fn delay_ms(&self, attempt: u8) -> u64 {
        // 2^attempt, capped so shift is in [0, 63] (avoids overflow on <<).
        let shift = u64::from(attempt).min(63) as u32;
        let factor = 1_u64 << shift; // safe: shift in [0, 63]
        self.base_ms.saturating_mul(factor).min(self.max_ms)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Queue-depth reader seam
// ─────────────────────────────────────────────────────────────────────────────

/// Reads the current admission queue depth from the sidecar (or another
/// congestion signal such as PV2 `r`).
///
/// The production implementation queries the sidecar snapshot endpoint; tests
/// inject a stub that returns a fixed depth.
pub trait QueueDepthReader: Send + Sync {
    /// Returns the current queue depth.
    fn read(&self) -> QueueDepth;
}

/// A [`QueueDepthReader`] that always returns zero.
///
/// Suitable for contexts where no congestion signal is available; the
/// congestion throttle will never fire.
#[derive(Clone, Copy, Debug, Default)]
pub struct ZeroDepthReader;

impl QueueDepthReader for ZeroDepthReader {
    fn read(&self) -> QueueDepth {
        QueueDepth::new(0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FAIR-SEMAPHORE (Tier-A, hard)
// ─────────────────────────────────────────────────────────────────────────────

/// A single waiter blocked in the FIFO queue.
struct Waiter {
    /// Condvar-pair used to signal this specific waiter.
    ready: Arc<(Mutex<bool>, Condvar)>,
}

/// Internal state protected by the semaphore mutex.
struct SemState {
    /// Permits currently available for immediate acquisition.
    available: u8,
    /// Hard capacity — never changes after construction.
    total: u8,
    /// Number of permits currently held by callers.
    in_flight: u8,
    /// FIFO queue of blocked waiters.
    waiters: VecDeque<Waiter>,
}

/// Shared, reference-counted core of [`FairSemaphore`].
struct SemShared {
    state: Mutex<SemState>,
}

impl SemShared {
    /// Attempts to acquire one permit, blocking in FIFO order if necessary.
    fn acquire_permit(self: &Arc<Self>) -> Result<Permit> {
        // Fast path: permit available and no waiters ahead of us.
        let waiter_pair = {
            let mut guard = self.state.lock().map_err(|_| DcgError::Subprocess {
                command: "fair-semaphore".to_string(),
                detail: "mutex poisoned on acquire".to_string(),
            })?;

            if guard.available > 0 && guard.waiters.is_empty() {
                guard.available -= 1;
                guard.in_flight += 1;
                return Ok(Permit { shared: Arc::clone(self) });
            }

            // Slow path: queue a waiter.
            let pair: Arc<(Mutex<bool>, Condvar)> =
                Arc::new((Mutex::new(false), Condvar::new()));
            guard.waiters.push_back(Waiter { ready: Arc::clone(&pair) });
            pair
        };

        // Wait on our private condvar until release_one signals us.
        let (lock, cvar) = &*waiter_pair;
        let ready = lock.lock().map_err(|_| DcgError::Subprocess {
            command: "fair-semaphore".to_string(),
            detail: "waiter mutex poisoned".to_string(),
        })?;
        // `wait_while` re-acquires the guard each time it is notified.
        let _guard = cvar
            .wait_while(ready, |r| !*r)
            .map_err(|_| DcgError::Subprocess {
                command: "fair-semaphore".to_string(),
                detail: "waiter condvar poisoned".to_string(),
            })?;

        // `in_flight` was incremented by `release_one` when it granted us the permit.
        Ok(Permit { shared: Arc::clone(self) })
    }

    /// Releases one permit and serves the head of the FIFO queue, if any.
    fn release_one(&self) {
        let waiter_opt = match self.state.lock() {
            Err(_) => return, // poisoned — can't safely update
            Ok(mut g) => {
                g.in_flight = g.in_flight.saturating_sub(1);
                if let Some(w) = g.waiters.pop_front() {
                    // Transfer: grant permit to first waiter.
                    g.in_flight += 1;
                    Some(w)
                } else {
                    g.available = g.available.saturating_add(1);
                    None
                }
            }
        };

        // Signal the waiter outside the lock to avoid priority inversion.
        if let Some(w) = waiter_opt {
            let (lock, cvar) = &*w.ready;
            if let Ok(mut ready) = lock.lock() {
                *ready = true;
                cvar.notify_one();
            }
            // If the waiter's lock is poisoned the waiter will remain blocked
            // indefinitely; this is an unrecoverable scenario outside our control.
        }
    }
}

/// FIFO fair semaphore (Tier-A).
///
/// Bounds the number of concurrently in-flight admissions to at most
/// [`MaxInflight`] permits. When no permit is available the caller blocks and
/// is queued in strict arrival order (FIFO). The earliest waiter is always
/// served first when a permit becomes available; later arrivals cannot bypass
/// an earlier waiter.
pub struct FairSemaphore {
    shared: Arc<SemShared>,
}

impl FairSemaphore {
    /// Creates a new semaphore with the given hard capacity.
    #[must_use]
    pub fn new(max: MaxInflight) -> Self {
        Self {
            shared: Arc::new(SemShared {
                state: Mutex::new(SemState {
                    total: max.get(),
                    available: max.get(),
                    in_flight: 0,
                    waiters: VecDeque::new(),
                }),
            }),
        }
    }

    /// Acquires one permit, blocking in FIFO order if necessary.
    ///
    /// Returns a [`Permit`] that releases the slot when dropped.
    ///
    /// # Errors
    /// Returns [`DcgError::Subprocess`] only if an internal mutex or condvar
    /// becomes poisoned (a programming error or external panic); this should
    /// not occur under normal operation.
    pub fn acquire(&self) -> Result<Permit> {
        self.shared.acquire_permit()
    }

    /// Returns the number of permits currently held.
    #[must_use]
    pub fn in_flight(&self) -> u8 {
        self.shared.state.lock().map_or(0, |g| g.in_flight)
    }

    /// Returns the number of permits available for immediate acquisition.
    #[must_use]
    pub fn available(&self) -> u8 {
        self.shared.state.lock().map_or(0, |g| g.available)
    }

    /// Returns the hard capacity configured at construction.
    #[must_use]
    pub fn total(&self) -> u8 {
        self.shared.state.lock().map_or(0, |g| g.total)
    }

    /// Returns the number of waiters currently blocked in the FIFO queue.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.shared.state.lock().map_or(0, |g| g.waiters.len())
    }
}

/// RAII permit returned by [`FairSemaphore::acquire`].
///
/// Releases the semaphore slot when dropped. Do not clone or share; each
/// call to [`FairSemaphore::acquire`] must be paired with exactly one
/// `Permit` being dropped.
pub struct Permit {
    shared: Arc<SemShared>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.shared.release_one();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry policy (Tier-A transparent-retry-first)
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` when `err` represents a transient failure that should be
/// retried.
///
/// Only [`DcgError::Subprocess`] is transient. All other variants represent
/// explicit denials (`NotArmed`, `StaleFence`, `Denied`) or configuration /
/// parse errors — none of which will succeed on retry.
fn is_transient(err: &DcgError) -> bool {
    matches!(err, DcgError::Subprocess { .. })
}

/// Executes `op` with up to `max_retries` retries on transient failures.
///
/// On each retry the `backoff` schedule is consulted for the delay, and the
/// `sleeper` is invoked to apply it (the sleeper is a no-op in tests).
/// Non-transient errors short-circuit immediately.
///
/// # Errors
/// Returns the last encountered error if all attempts are exhausted, or the
/// first non-transient error encountered.
fn run_with_retry<T, F>(
    max_retries: MaxRetries,
    backoff: &dyn BackoffSchedule,
    sleeper: &dyn Sleeper,
    op: &F,
) -> Result<T>
where
    F: Fn() -> Result<T>,
{
    let attempts = max_retries.get().saturating_add(1); // retries + first attempt
    let mut last_err = DcgError::Subprocess {
        command: String::new(),
        detail: "no attempts made".to_string(),
    };

    for attempt in 0..attempts {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) if !is_transient(&e) => return Err(e),
            Err(e) => {
                last_err = e;
                // Compute and apply backoff before the next attempt, but only
                // when there is a next attempt.
                let is_last = attempt.saturating_add(1) >= attempts;
                if !is_last {
                    let delay = backoff.delay_ms(attempt);
                    sleeper.sleep_ms(delay);
                }
            }
        }
    }
    Err(last_err)
}

// ─────────────────────────────────────────────────────────────────────────────
// Circuit-breaker (Tier-B, measure-first)
// ─────────────────────────────────────────────────────────────────────────────

/// Observed state of the circuit-breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakerState {
    /// Normal operation: requests pass through.
    Closed,
    /// Breaker tripped: requests are denied. Transitions to [`BreakerState::HalfOpen`]
    /// after `half_open_timeout_ms`.
    Open {
        /// Clock time (ms) when the breaker opened.
        opened_at_ms: u64,
    },
    /// Allowing a single probe: next request passes through.
    /// Success closes; failure re-opens the breaker.
    HalfOpen,
}

/// Circuit-breaker tracking consecutive failures.
///
/// **Measure-first defaults** — the [`FailureThreshold`] and
/// [`HalfOpenTimeoutMs`] used at construction must be tuned to the observed
/// failure rate and expected recovery time of the underlying system.
pub struct CircuitBreaker {
    state: BreakerState,
    consecutive_failures: u32,
    threshold: FailureThreshold,
    half_open_timeout_ms: HalfOpenTimeoutMs,
}

impl CircuitBreaker {
    /// Creates a closed circuit-breaker with the given threshold and half-open
    /// timeout.
    #[must_use]
    pub fn new(threshold: FailureThreshold, half_open_timeout_ms: HalfOpenTimeoutMs) -> Self {
        Self {
            state: BreakerState::Closed,
            consecutive_failures: 0,
            threshold,
            half_open_timeout_ms,
        }
    }

    /// Returns the current [`BreakerState`].
    #[must_use]
    pub fn state(&self) -> BreakerState {
        self.state
    }

    /// Checks whether the breaker allows the current operation to proceed.
    ///
    /// - `Closed` → allowed.
    /// - `Open` and timeout not elapsed → denied.
    /// - `Open` and timeout elapsed → transitions to `HalfOpen`; allowed (probe).
    /// - `HalfOpen` → denied (only one probe is active at a time; subsequent
    ///   callers wait until the probe settles).
    ///
    /// # Errors
    /// Returns [`DcgError::Denied`] when the breaker is open or mid-probe.
    pub fn check(&mut self, clock: &dyn MonotonicClock) -> Result<()> {
        match self.state {
            BreakerState::Closed => Ok(()),
            BreakerState::Open { opened_at_ms } => {
                let elapsed = clock.now_ms().saturating_sub(opened_at_ms);
                if elapsed >= self.half_open_timeout_ms.get() {
                    self.state = BreakerState::HalfOpen;
                    Ok(()) // allow the probe
                } else {
                    Err(DcgError::Denied {
                        reason: "circuit open".to_string(),
                    })
                }
            }
            BreakerState::HalfOpen => {
                // A probe is already in flight. Deny to prevent a thundering
                // herd on recovery.
                Err(DcgError::Denied {
                    reason: "circuit open".to_string(),
                })
            }
        }
    }

    /// Records a successful operation.
    ///
    /// Resets the consecutive-failure counter and closes the breaker.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.state = BreakerState::Closed;
    }

    /// Records a failed operation.
    ///
    /// Increments the failure counter; opens the breaker when the threshold is
    /// reached.
    pub fn record_failure(&mut self, clock: &dyn MonotonicClock) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= self.threshold.get() {
            self.state = BreakerState::Open { opened_at_ms: clock.now_ms() };
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AIMD controller (Tier-B, measure-first)
// ─────────────────────────────────────────────────────────────────────────────

/// AIMD (additive-increase / multiplicative-decrease) soft concurrency cap.
///
/// Tracks an *effective width* that starts at [`MaxInflight`] and is adjusted
/// as follows:
///
/// - **Additive increase** (on success): `effective += increase_step`, capped
///   at `max_inflight`.
/// - **Multiplicative decrease** (on congestion): `effective =
///   max(1, floor(effective * decrease_factor))`.
///
/// The effective width is a *soft ceiling*: it signals congestion but does not
/// enforce hard blocking. The [`Governor`] checks the soft ceiling before
/// acquiring the hard semaphore permit.
///
/// **Measure-first defaults** — `increase_step` and `decrease_factor` must be
/// tuned to the observed congestion frequency and recovery time; the defaults
/// are starting points only.
pub struct AimdController {
    effective_width: u8,
    max_inflight: u8,
    /// **Measure-first default:** `1`.
    increase_step: u8,
    /// **Measure-first default:** `0.5`.
    decrease_factor: f64,
}

impl AimdController {
    /// Creates a new AIMD controller.
    ///
    /// - `max_inflight` — the hard ceiling; effective width is initialised to
    ///   this value and capped here on increase.
    /// - `increase_step` — **Measure-first default: `1`**.
    /// - `decrease_factor` — **Measure-first default: `0.5`**; must be in
    ///   `(0.0, 1.0]`.
    #[must_use]
    pub fn new(max_inflight: MaxInflight, increase_step: u8, decrease_factor: f64) -> Self {
        // Clamp decrease_factor to a sensible range.
        let factor = decrease_factor.clamp(f64::from(f32::EPSILON), 1.0);
        Self {
            effective_width: max_inflight.get(),
            max_inflight: max_inflight.get(),
            increase_step,
            decrease_factor: factor,
        }
    }

    /// Returns the current effective width.
    #[must_use]
    pub fn effective_width(&self) -> u8 {
        self.effective_width
    }

    /// Additively increases the effective width toward [`MaxInflight`].
    pub fn increase(&mut self) {
        self.effective_width = self
            .effective_width
            .saturating_add(self.increase_step)
            .min(self.max_inflight);
    }

    /// Multiplicatively decreases the effective width, floored at `1`.
    pub fn decrease(&mut self) {
        let raw = f64::from(self.effective_width)
            .mul_add(self.decrease_factor, 0.0)
            .floor();
        self.effective_width = saturating_f64_to_u8_min1(raw);
    }

    /// Returns `true` when the current in-flight count has met or exceeded the
    /// AIMD soft ceiling.
    #[must_use]
    pub fn is_congested(&self, in_flight: u8) -> bool {
        in_flight >= self.effective_width
    }
}

/// Converts an f64 to u8, clamped to `[1, 255]`.
///
/// The caller is responsible for ensuring `v` is a finite, non-NaN value
/// already in a meaningful range; this function clamps defensively.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn saturating_f64_to_u8_min1(v: f64) -> u8 {
    if v <= 1.0 {
        return 1;
    }
    if v >= f64::from(u8::MAX) {
        return u8::MAX;
    }
    v as u8
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-agent budget (Tier-B, measure-first)
// ─────────────────────────────────────────────────────────────────────────────

/// Per-agent cumulative cost tracker with a soft-kill threshold.
///
/// **Measure-first default** — the [`BudgetLimit`] must be tuned to the
/// observed per-agent cost distribution; the default is a starting point only.
pub struct AgentBudget {
    limit: BudgetLimit,
    /// Maps agent id → cumulative cost charged.
    costs: HashMap<String, u64>,
}

impl AgentBudget {
    /// Creates a new budget tracker with the given per-agent limit.
    #[must_use]
    pub fn new(limit: BudgetLimit) -> Self {
        Self { limit, costs: HashMap::new() }
    }

    /// Checks and charges `cost` to `agent_id`.
    ///
    /// If the agent's cumulative cost plus `cost` would exceed the limit, the
    /// charge is refused with a soft-kill denial. If allowed, the cost is
    /// accumulated.
    ///
    /// # Errors
    /// Returns [`DcgError::Denied`] with reason `"agent budget exceeded"` when
    /// the agent's cumulative spend plus the requested cost would exceed
    /// [`BudgetLimit`].
    pub fn charge(&mut self, agent_id: &str, cost: CostUnit) -> Result<()> {
        let current = self.costs.get(agent_id).copied().unwrap_or(0);
        let next = current.saturating_add(cost.get());
        if next > self.limit.get() {
            return Err(DcgError::Denied {
                reason: "agent budget exceeded".to_string(),
            });
        }
        self.costs.insert(agent_id.to_string(), next);
        Ok(())
    }

    /// Returns the cumulative cost accumulated for `agent_id`.
    #[must_use]
    pub fn spent(&self, agent_id: &str) -> u64 {
        self.costs.get(agent_id).copied().unwrap_or(0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Governor configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for all tiers of the [`Governor`].
///
/// All tier-B fields carry **measure-first defaults** — they are starting
/// points for instrumentation, not proven operational constants.
#[derive(Clone, Copy, Debug)]
pub struct GovernorConfig {
    /// Hard cap on concurrent in-flight admissions (Tier-A).
    pub max_inflight: MaxInflight,
    /// Maximum transient-retry count (Tier-A). **Measure-first default: `3`**.
    pub max_retries: MaxRetries,
    /// Consecutive-failure threshold to open the circuit-breaker (Tier-B).
    /// **Measure-first default: `5`**.
    pub failure_threshold: FailureThreshold,
    /// Milliseconds before the open breaker transitions to half-open (Tier-B).
    /// **Measure-first default: `10_000`**.
    pub half_open_timeout_ms: HalfOpenTimeoutMs,
    /// AIMD additive-increase step (Tier-B). **Measure-first default: `1`**.
    pub aimd_increase_step: u8,
    /// AIMD multiplicative-decrease factor in `(0.0, 1.0]` (Tier-B).
    /// **Measure-first default: `0.5`**.
    pub aimd_decrease_factor: f64,
    /// Per-agent cumulative cost limit (Tier-B).
    /// **Measure-first default: `1_000_000`**.
    pub agent_budget_limit: BudgetLimit,
    /// Queue depth above which the congestion throttle triggers (Tier-B).
    /// **Measure-first default: `10`**.
    pub congestion_threshold: CongestionThreshold,
}

impl GovernorConfig {
    /// Returns a configuration with measure-first defaults.
    ///
    /// # Errors
    /// This function is infallible; [`DcgError`] cannot be produced by the
    /// default values. Returns `Err` only if this function's internal defaults
    /// violate their own bounds — which would be a bug.
    pub fn default_config() -> Result<Self> {
        Ok(Self {
            max_inflight: MaxInflight::new(4)?,
            max_retries: MaxRetries::new(3),
            failure_threshold: FailureThreshold::new(5)?,
            half_open_timeout_ms: HalfOpenTimeoutMs::new(10_000),
            aimd_increase_step: 1,
            aimd_decrease_factor: 0.5,
            agent_budget_limit: BudgetLimit::new(1_000_000),
            congestion_threshold: CongestionThreshold::new(10),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Governor
// ─────────────────────────────────────────────────────────────────────────────

/// Composes Tier-A (fair semaphore + transparent retry) and Tier-B (AIMD,
/// circuit-breaker, per-agent budget, congestion throttle) governors around
/// a delegated-write admission operation.
pub struct Governor {
    semaphore: FairSemaphore,
    max_retries: MaxRetries,
    backoff: Arc<dyn BackoffSchedule>,
    sleeper: Arc<dyn Sleeper>,
    breaker: Mutex<CircuitBreaker>,
    aimd: Mutex<AimdController>,
    budget: Mutex<AgentBudget>,
    congestion_threshold: CongestionThreshold,
    queue_depth_reader: Arc<dyn QueueDepthReader>,
    clock: Arc<dyn MonotonicClock>,
}

impl Governor {
    /// Creates a governor from the given configuration and injected seams.
    #[must_use]
    pub fn new(
        config: GovernorConfig,
        backoff: Arc<dyn BackoffSchedule>,
        sleeper: Arc<dyn Sleeper>,
        queue_depth_reader: Arc<dyn QueueDepthReader>,
        clock: Arc<dyn MonotonicClock>,
    ) -> Self {
        let breaker = CircuitBreaker::new(
            config.failure_threshold,
            config.half_open_timeout_ms,
        );
        let aimd = AimdController::new(
            config.max_inflight,
            config.aimd_increase_step,
            config.aimd_decrease_factor,
        );
        Self {
            semaphore: FairSemaphore::new(config.max_inflight),
            max_retries: config.max_retries,
            backoff,
            sleeper,
            breaker: Mutex::new(breaker),
            aimd: Mutex::new(aimd),
            budget: Mutex::new(AgentBudget::new(config.agent_budget_limit)),
            congestion_threshold: config.congestion_threshold,
            queue_depth_reader,
            clock,
        }
    }

    /// Wraps `op` with full governance: budget check, congestion update, circuit-
    /// breaker check, AIMD soft-cap check, fair-semaphore acquisition, and
    /// transparent retry on transient failures.
    ///
    /// ## Execution order
    ///
    /// 1. Charge `cost` to `agent_id` budget; soft-kill if exceeded.
    /// 2. Read queue depth; if above threshold, apply AIMD decrease.
    /// 3. Check circuit-breaker; deny if open.
    /// 4. Check AIMD soft cap against current in-flight; deny if exceeded.
    /// 5. Acquire fair-semaphore permit (FIFO blocking).
    /// 6. Execute `op` with up to [`MaxRetries`] retries on transient errors.
    /// 7. Update AIMD and circuit-breaker with the outcome.
    /// 8. Release permit (RAII).
    ///
    /// # Errors
    /// Returns [`DcgError::Denied`] with reason `"agent budget exceeded"` if the
    /// agent is over budget.
    /// Returns [`DcgError::Denied`] with reason `"aimd backpressure"` if the
    /// AIMD soft cap is exceeded.
    /// Returns [`DcgError::Denied`] with reason `"circuit open"` if the
    /// circuit-breaker is in the open or half-open state.
    /// Returns any error produced by `op` after retries are exhausted.
    /// Returns [`DcgError::Subprocess`] if the fair-semaphore or any internal
    /// mutex becomes poisoned (should not occur under normal operation).
    pub fn govern<F, T>(&self, agent_id: &str, cost: CostUnit, op: F) -> Result<T>
    where
        F: Fn() -> Result<T>,
    {
        // Step 1: per-agent budget soft-kill.
        self.budget
            .lock()
            .map_err(|_| DcgError::Subprocess {
                command: "governor-budget".to_string(),
                detail: "mutex poisoned".to_string(),
            })?
            .charge(agent_id, cost)?;

        // Step 2: congestion signal → AIMD decrease if congested.
        let depth = self.queue_depth_reader.read();
        if depth.get() >= self.congestion_threshold.get() {
            if let Ok(mut aimd) = self.aimd.lock() {
                aimd.decrease();
            }
        }

        // Step 3: circuit-breaker check.
        self.breaker
            .lock()
            .map_err(|_| DcgError::Subprocess {
                command: "governor-breaker".to_string(),
                detail: "mutex poisoned".to_string(),
            })?
            .check(self.clock.as_ref())?;

        // Step 4: AIMD soft-cap check.
        let current_in_flight = self.semaphore.in_flight();
        {
            let aimd = self.aimd.lock().map_err(|_| DcgError::Subprocess {
                command: "governor-aimd".to_string(),
                detail: "mutex poisoned".to_string(),
            })?;
            if aimd.is_congested(current_in_flight) {
                return Err(DcgError::Denied {
                    reason: "aimd backpressure".to_string(),
                });
            }
        }

        // Step 5: acquire FIFO permit (blocks if no permits available).
        let _permit = self.semaphore.acquire()?;

        // Step 6: execute with transparent retry on transient errors.
        let result = run_with_retry(
            self.max_retries,
            self.backoff.as_ref(),
            self.sleeper.as_ref(),
            &op,
        );

        // Steps 7–8: update soft governors; permit drops (RAII) at block end.
        if result.is_ok() {
            if let Ok(mut aimd) = self.aimd.lock() {
                aimd.increase();
            }
            if let Ok(mut breaker) = self.breaker.lock() {
                breaker.record_success();
            }
        } else {
            if let Ok(mut aimd) = self.aimd.lock() {
                aimd.decrease();
            }
            if let Ok(mut breaker) = self.breaker.lock() {
                breaker.record_failure(self.clock.as_ref());
            }
        }

        result
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Backoff that returns zero delay for all attempts (avoids sleeping in tests).
    struct ZeroBackoff;
    impl BackoffSchedule for ZeroBackoff {
        fn delay_ms(&self, _attempt: u8) -> u64 {
            0
        }
    }

    /// Sleeper that records invocations without sleeping.
    struct RecordingSleeper {
        calls: Mutex<Vec<u64>>,
    }
    impl RecordingSleeper {
        fn new() -> Self {
            Self { calls: Mutex::new(Vec::new()) }
        }
    }
    impl Sleeper for RecordingSleeper {
        fn sleep_ms(&self, ms: u64) {
            self.calls.lock().unwrap().push(ms);
        }
    }

    /// Controllable clock backed by an atomic counter (no wall-clock).
    struct StubClock {
        now: AtomicU64,
    }
    impl StubClock {
        fn new(initial: u64) -> Self {
            Self { now: AtomicU64::new(initial) }
        }
        fn advance(&self, by_ms: u64) {
            self.now.fetch_add(by_ms, Ordering::Relaxed);
        }
    }
    impl MonotonicClock for StubClock {
        fn now_ms(&self) -> u64 {
            self.now.load(Ordering::Relaxed)
        }
    }

    /// Queue-depth reader that returns a configurable fixed depth.
    struct FixedDepth(u32);
    impl QueueDepthReader for FixedDepth {
        fn read(&self) -> QueueDepth {
            QueueDepth::new(self.0)
        }
    }

    /// Builds a default [`Governor`] with deterministic seams and optional
    /// queue depth. `inflight` sets [`MaxInflight`], `retries` sets [`MaxRetries`].
    fn make_governor(
        inflight: u8,
        retries: u8,
        queue_depth: u32,
        clock: Arc<dyn MonotonicClock>,
    ) -> Governor {
        let config = GovernorConfig {
            max_inflight: MaxInflight::new(inflight).unwrap(),
            max_retries: MaxRetries::new(retries),
            failure_threshold: FailureThreshold::new(5).unwrap(),
            half_open_timeout_ms: HalfOpenTimeoutMs::new(1_000),
            aimd_increase_step: 1,
            aimd_decrease_factor: 0.5,
            agent_budget_limit: BudgetLimit::new(1_000_000),
            congestion_threshold: CongestionThreshold::new(10),
        };
        Governor::new(
            config,
            Arc::new(ZeroBackoff),
            Arc::new(RecordingSleeper::new()),
            Arc::new(FixedDepth(queue_depth)),
            clock,
        )
    }

    fn stub_clock() -> Arc<StubClock> {
        Arc::new(StubClock::new(0))
    }

    // ═════════════════════════════════════════════════════════════════════════
    // MaxInflight newtype
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn max_inflight_zero_rejected() {
        assert!(MaxInflight::new(0).is_err());
    }

    #[test]
    fn max_inflight_one_valid() {
        assert_eq!(MaxInflight::new(1).unwrap().get(), 1);
    }

    #[test]
    fn max_inflight_max_u8_valid() {
        assert_eq!(MaxInflight::new(255).unwrap().get(), 255);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // FairSemaphore — bounds concurrency
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn semaphore_single_permit_allows_one_inflight() {
        let sem = FairSemaphore::new(MaxInflight::new(1).unwrap());
        let _p = sem.acquire().unwrap();
        assert_eq!(sem.in_flight(), 1);
        assert_eq!(sem.available(), 0);
    }

    #[test]
    fn semaphore_two_permits_allows_two_inflight() {
        let sem = FairSemaphore::new(MaxInflight::new(2).unwrap());
        let _p1 = sem.acquire().unwrap();
        let _p2 = sem.acquire().unwrap();
        assert_eq!(sem.in_flight(), 2);
        assert_eq!(sem.available(), 0);
    }

    #[test]
    fn semaphore_total_matches_construction() {
        let sem = FairSemaphore::new(MaxInflight::new(4).unwrap());
        assert_eq!(sem.total(), 4);
    }

    #[test]
    fn semaphore_available_decrements_on_acquire() {
        let sem = FairSemaphore::new(MaxInflight::new(3).unwrap());
        let _p = sem.acquire().unwrap();
        assert_eq!(sem.available(), 2);
    }

    #[test]
    fn semaphore_permit_drop_restores_available() {
        let sem = FairSemaphore::new(MaxInflight::new(1).unwrap());
        {
            let _p = sem.acquire().unwrap();
            assert_eq!(sem.available(), 0);
        }
        assert_eq!(sem.available(), 1);
        assert_eq!(sem.in_flight(), 0);
    }

    #[test]
    fn semaphore_in_flight_decrements_on_drop() {
        let sem = FairSemaphore::new(MaxInflight::new(2).unwrap());
        let p1 = sem.acquire().unwrap();
        let _p2 = sem.acquire().unwrap();
        assert_eq!(sem.in_flight(), 2);
        drop(p1);
        assert_eq!(sem.in_flight(), 1);
    }

    #[test]
    fn semaphore_blocks_when_full_then_unblocks() {
        use std::sync::Barrier;
        // Semaphore with 1 permit; second acquire blocks until first releases.
        let sem = Arc::new(FairSemaphore::new(MaxInflight::new(1).unwrap()));
        let sem2 = Arc::clone(&sem);

        let barrier_started = Arc::new(Barrier::new(2));
        let barrier2 = Arc::clone(&barrier_started);

        // Thread holds permit
        let permit = sem.acquire().unwrap();
        assert_eq!(sem.available(), 0);

        let got_permit = Arc::new(Mutex::new(false));
        let got_permit2 = Arc::clone(&got_permit);

        let handle = std::thread::spawn(move || {
            barrier2.wait(); // signal main we're about to block
            let _p = sem2.acquire().unwrap();
            *got_permit2.lock().unwrap() = true;
        });

        barrier_started.wait();
        std::thread::sleep(std::time::Duration::from_millis(5));
        // Thread should be blocked
        assert!(!*got_permit.lock().unwrap());
        drop(permit); // release → thread unblocks
        handle.join().unwrap();
        assert!(*got_permit.lock().unwrap());
    }

    #[test]
    fn semaphore_waiting_count_reflects_blocked_callers() {
        let sem = Arc::new(FairSemaphore::new(MaxInflight::new(1).unwrap()));
        let sem2 = Arc::clone(&sem);

        let held = sem.acquire().unwrap();
        let started = Arc::new(Mutex::new(false));
        let started2 = Arc::clone(&started);

        let handle = std::thread::spawn(move || {
            *started2.lock().unwrap() = true;
            let _p = sem2.acquire().unwrap();
        });

        // Poll until thread has queued.
        for _ in 0..100 {
            if *started.lock().unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
        assert_eq!(sem.waiting(), 1);
        drop(held);
        handle.join().unwrap();
        assert_eq!(sem.waiting(), 0);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // FIFO fairness
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn fifo_two_waiters_served_in_arrival_order() {
        // One permit; hold it, queue two waiters in known order, release and
        // verify the earlier arrival gets the permit first.
        let sem = Arc::new(FairSemaphore::new(MaxInflight::new(1).unwrap()));

        // Seize the only permit.
        let permit = sem.acquire().unwrap();

        let order: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

        // Waiter A: queues first.
        let sem_a = Arc::clone(&sem);
        let order_a = Arc::clone(&order);
        let a_ready = Arc::new((Mutex::new(false), Condvar::new()));
        let a_ready2 = Arc::clone(&a_ready);

        let t_a = std::thread::spawn(move || {
            {
                let (lk, cv) = &*a_ready2;
                *lk.lock().unwrap() = true;
                cv.notify_one();
            }
            let _p = sem_a.acquire().unwrap();
            order_a.lock().unwrap().push(1);
        });

        // Wait until A signals it is about to block.
        {
            let (lk, cv) = &*a_ready;
            let guard = lk.lock().unwrap();
            let _g = cv.wait_while(guard, |r| !*r).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(3));

        // Waiter B: queues second.
        let sem_b = Arc::clone(&sem);
        let order_b = Arc::clone(&order);
        let t_b = std::thread::spawn(move || {
            let _p = sem_b.acquire().unwrap();
            order_b.lock().unwrap().push(2);
        });

        std::thread::sleep(std::time::Duration::from_millis(3));

        // Release → A should get the permit first (it was first in the queue).
        drop(permit);

        t_a.join().unwrap();
        // After A gets and releases, B should get it.
        t_b.join().unwrap();

        let observed = order.lock().unwrap().clone();
        assert_eq!(observed, vec![1, 2], "FIFO: waiter A must be served before waiter B");
    }

    #[test]
    fn fifo_single_waiter_gets_permit_on_release() {
        let sem = Arc::new(FairSemaphore::new(MaxInflight::new(1).unwrap()));
        let permit = sem.acquire().unwrap();
        let sem2 = Arc::clone(&sem);

        let got = Arc::new(Mutex::new(false));
        let got2 = Arc::clone(&got);
        let t = std::thread::spawn(move || {
            let _p = sem2.acquire().unwrap();
            *got2.lock().unwrap() = true;
        });

        std::thread::sleep(std::time::Duration::from_millis(3));
        drop(permit);
        t.join().unwrap();
        assert!(*got.lock().unwrap());
    }

    #[test]
    fn fifo_multiple_releases_preserve_queue_order() {
        // Two permits; fill both; then queue three more in order; release sequentially.
        let sem = Arc::new(FairSemaphore::new(MaxInflight::new(2).unwrap()));
        let p1 = sem.acquire().unwrap();
        let p2 = sem.acquire().unwrap();

        let order: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();

        for id in 1_u8..=3 {
            let sem_c = Arc::clone(&sem);
            let ord = Arc::clone(&order);
            // Small stagger to ensure ordering in the VecDeque.
            std::thread::sleep(std::time::Duration::from_millis(2));
            handles.push(std::thread::spawn(move || {
                let _p = sem_c.acquire().unwrap();
                ord.lock().unwrap().push(id);
            }));
        }

        // Give all three threads time to queue.
        std::thread::sleep(std::time::Duration::from_millis(10));

        drop(p1); // unblocks waiter 1
        // Allow waiter 1 to wake, acquire the order mutex, and push before we
        // signal waiter 2.  Without this gap both threads race on the mutex
        // and the push order becomes non-deterministic.
        std::thread::sleep(std::time::Duration::from_millis(5));
        drop(p2); // unblocks waiter 2; waiter 3 still blocked
        for h in handles {
            h.join().unwrap();
        }

        let observed = order.lock().unwrap().clone();
        assert_eq!(
            observed[0], 1,
            "first waiter must be first served"
        );
        assert_eq!(
            observed[1], 2,
            "second waiter must be second served"
        );
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Retry: is_transient classification
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn subprocess_error_is_transient() {
        let e = DcgError::Subprocess { command: "c".to_string(), detail: "d".to_string() };
        assert!(is_transient(&e));
    }

    #[test]
    fn not_armed_is_not_transient() {
        let e = DcgError::NotArmed { key: "k".to_string() };
        assert!(!is_transient(&e));
    }

    #[test]
    fn stale_fence_is_not_transient() {
        let e = DcgError::StaleFence { resource: "r".to_string(), presented: 1, last_admitted: 2 };
        assert!(!is_transient(&e));
    }

    #[test]
    fn denied_is_not_transient() {
        let e = DcgError::Denied { reason: "policy".to_string() };
        assert!(!is_transient(&e));
    }

    #[test]
    fn parse_error_is_not_transient() {
        let e = DcgError::Parse { source: "s", detail: "d".to_string() };
        assert!(!is_transient(&e));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Retry: run_with_retry behaviour
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn retry_succeeds_on_first_attempt() {
        let call_count = Arc::new(AtomicU64::new(0));
        let cc = Arc::clone(&call_count);
        let result = run_with_retry(MaxRetries::new(3), &ZeroBackoff, &RecordingSleeper::new(), &move || {
            cc.fetch_add(1, Ordering::Relaxed);
            Ok::<u32, DcgError>(42)
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn retry_then_success_on_nth_attempt() {
        // Fail twice (transient) then succeed on third call.
        let attempts = Arc::new(AtomicU64::new(0));
        let a = Arc::clone(&attempts);
        let result = run_with_retry(MaxRetries::new(3), &ZeroBackoff, &RecordingSleeper::new(), &move || {
            let n = a.fetch_add(1, Ordering::Relaxed);
            if n < 2 {
                Err(DcgError::Subprocess { command: "cmd".to_string(), detail: "transient".to_string() })
            } else {
                Ok::<u8, DcgError>(7)
            }
        });
        assert_eq!(result.unwrap(), 7);
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn retry_exhausted_surfaces_last_subprocess_error() {
        let result = run_with_retry(
            MaxRetries::new(2),
            &ZeroBackoff,
            &RecordingSleeper::new(),
            &|| Err::<(), _>(DcgError::Subprocess { command: "cmd".to_string(), detail: "boom".to_string() }),
        );
        let err = result.unwrap_err();
        assert!(matches!(err, DcgError::Subprocess { .. }));
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn retry_exhausted_makes_retries_plus_one_total_calls() {
        let calls = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&calls);
        let _ = run_with_retry(MaxRetries::new(4), &ZeroBackoff, &RecordingSleeper::new(), &move || {
            c.fetch_add(1, Ordering::Relaxed);
            Err::<(), _>(DcgError::Subprocess { command: "c".to_string(), detail: "d".to_string() })
        });
        assert_eq!(calls.load(Ordering::Relaxed), 5); // 1 initial + 4 retries
    }

    #[test]
    fn non_transient_not_retried_returns_immediately() {
        let calls = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&calls);
        let result = run_with_retry(MaxRetries::new(5), &ZeroBackoff, &RecordingSleeper::new(), &move || {
            c.fetch_add(1, Ordering::Relaxed);
            Err::<(), _>(DcgError::NotArmed { key: "k".to_string() })
        });
        assert!(matches!(result.unwrap_err(), DcgError::NotArmed { .. }));
        assert_eq!(calls.load(Ordering::Relaxed), 1, "non-transient: must NOT retry");
    }

    #[test]
    fn backoff_delays_computed_by_schedule() {
        struct RecordingBackoff(Mutex<Vec<u8>>);
        impl BackoffSchedule for RecordingBackoff {
            fn delay_ms(&self, attempt: u8) -> u64 {
                self.0.lock().unwrap().push(attempt);
                0
            }
        }
        let sched = Arc::new(RecordingBackoff(Mutex::new(Vec::new())));
        let sched2 = Arc::clone(&sched);
        let _ = run_with_retry(
            MaxRetries::new(3),
            sched2.as_ref(),
            &RecordingSleeper::new(),
            &|| Err::<(), _>(DcgError::Subprocess { command: "c".to_string(), detail: "d".to_string() }),
        );
        // 3 retries → 3 delay computations (not for the final attempt)
        let recorded = sched.0.lock().unwrap().clone();
        assert_eq!(recorded, vec![0, 1, 2]);
    }

    #[test]
    fn zero_retries_makes_exactly_one_call() {
        let calls = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&calls);
        let _ = run_with_retry(MaxRetries::new(0), &ZeroBackoff, &RecordingSleeper::new(), &move || {
            c.fetch_add(1, Ordering::Relaxed);
            Err::<(), _>(DcgError::Subprocess { command: "c".to_string(), detail: "d".to_string() })
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ExponentialBackoff
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn exponential_backoff_attempt_zero_is_base() {
        let b = ExponentialBackoff { base_ms: 100, max_ms: 10_000 };
        assert_eq!(b.delay_ms(0), 100);
    }

    #[test]
    fn exponential_backoff_attempt_one_is_double_base() {
        let b = ExponentialBackoff { base_ms: 100, max_ms: 10_000 };
        assert_eq!(b.delay_ms(1), 200);
    }

    #[test]
    fn exponential_backoff_capped_at_max() {
        let b = ExponentialBackoff { base_ms: 100, max_ms: 500 };
        assert_eq!(b.delay_ms(10), 500);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // CircuitBreaker
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn breaker_starts_closed() {
        let b = CircuitBreaker::new(
            FailureThreshold::new(3).unwrap(),
            HalfOpenTimeoutMs::new(1_000),
        );
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[test]
    fn breaker_check_closed_returns_ok() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(3).unwrap(),
            HalfOpenTimeoutMs::new(1_000),
        );
        let clock = StubClock::new(0);
        assert!(b.check(&clock).is_ok());
    }

    #[test]
    fn breaker_opens_after_k_consecutive_failures() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(3).unwrap(),
            HalfOpenTimeoutMs::new(1_000),
        );
        let clock = StubClock::new(0);
        b.record_failure(&clock);
        b.record_failure(&clock);
        assert_eq!(b.state(), BreakerState::Closed); // not yet
        b.record_failure(&clock);
        assert!(matches!(b.state(), BreakerState::Open { .. }), "breaker must open after K failures");
    }

    #[test]
    fn breaker_open_returns_denied() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(2).unwrap(),
            HalfOpenTimeoutMs::new(1_000),
        );
        let clock = StubClock::new(0);
        b.record_failure(&clock);
        b.record_failure(&clock);
        let err = b.check(&clock).unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
        assert!(err.to_string().contains("circuit open"));
    }

    #[test]
    fn breaker_transitions_to_half_open_after_timeout() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(1).unwrap(),
            HalfOpenTimeoutMs::new(500),
        );
        let clock = StubClock::new(0);
        b.record_failure(&clock);
        assert!(matches!(b.state(), BreakerState::Open { .. }));

        // Advance clock past the timeout.
        clock.advance(600);
        b.check(&clock).unwrap(); // should transition to HalfOpen and allow probe
        assert_eq!(b.state(), BreakerState::HalfOpen);
    }

    #[test]
    fn breaker_half_open_probe_success_closes_breaker() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(1).unwrap(),
            HalfOpenTimeoutMs::new(100),
        );
        let clock = StubClock::new(0);
        b.record_failure(&clock);
        clock.advance(200);
        b.check(&clock).unwrap(); // enter half-open
        b.record_success();
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[test]
    fn breaker_half_open_probe_failure_reopens_breaker() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(1).unwrap(),
            HalfOpenTimeoutMs::new(100),
        );
        let clock = StubClock::new(0);
        b.record_failure(&clock);
        clock.advance(200);
        b.check(&clock).unwrap(); // enter half-open
        clock.advance(10);
        b.record_failure(&clock);
        assert!(
            matches!(b.state(), BreakerState::Open { .. }),
            "failed probe must re-open breaker"
        );
    }

    #[test]
    fn breaker_success_resets_consecutive_failure_count() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(5).unwrap(),
            HalfOpenTimeoutMs::new(1_000),
        );
        let clock = StubClock::new(0);
        b.record_failure(&clock);
        b.record_failure(&clock);
        b.record_success();
        // After success: 2 more failures should NOT open the breaker (count reset to 0).
        b.record_failure(&clock);
        b.record_failure(&clock);
        assert_eq!(b.state(), BreakerState::Closed, "count must reset on success");
    }

    #[test]
    fn breaker_open_before_timeout_still_denied() {
        let mut b = CircuitBreaker::new(
            FailureThreshold::new(1).unwrap(),
            HalfOpenTimeoutMs::new(5_000),
        );
        let clock = StubClock::new(0);
        b.record_failure(&clock);
        clock.advance(100); // only 100ms, timeout is 5000ms
        assert!(b.check(&clock).is_err(), "still in timeout window — must deny");
    }

    // ═════════════════════════════════════════════════════════════════════════
    // AimdController
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn aimd_starts_at_max_inflight() {
        let aimd = AimdController::new(MaxInflight::new(8).unwrap(), 1, 0.5);
        assert_eq!(aimd.effective_width(), 8);
    }

    #[test]
    fn aimd_increase_raises_effective_width() {
        let mut aimd = AimdController::new(MaxInflight::new(8).unwrap(), 2, 0.5);
        aimd.decrease(); // bring it down first
        let before = aimd.effective_width();
        aimd.increase();
        assert_eq!(aimd.effective_width(), before + 2);
    }

    #[test]
    fn aimd_increase_capped_at_max_inflight() {
        let mut aimd = AimdController::new(MaxInflight::new(4).unwrap(), 10, 0.5);
        aimd.increase();
        assert_eq!(aimd.effective_width(), 4, "effective width must not exceed max_inflight");
    }

    #[test]
    fn aimd_decrease_reduces_effective_width() {
        let mut aimd = AimdController::new(MaxInflight::new(8).unwrap(), 1, 0.5);
        let before = aimd.effective_width();
        aimd.decrease();
        assert!(aimd.effective_width() < before, "decrease must reduce effective width");
    }

    #[test]
    fn aimd_decrease_floored_at_one() {
        let mut aimd = AimdController::new(MaxInflight::new(1).unwrap(), 1, 0.1);
        for _ in 0..20 {
            aimd.decrease();
        }
        assert_eq!(aimd.effective_width(), 1, "effective width floor is 1");
    }

    #[test]
    fn aimd_is_congested_when_inflight_meets_effective_width() {
        let aimd = AimdController::new(MaxInflight::new(4).unwrap(), 1, 0.5);
        assert!(aimd.is_congested(4));
    }

    #[test]
    fn aimd_not_congested_when_inflight_below_effective_width() {
        let aimd = AimdController::new(MaxInflight::new(4).unwrap(), 1, 0.5);
        assert!(!aimd.is_congested(3));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // AgentBudget
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn budget_charge_within_limit_succeeds() {
        let mut b = AgentBudget::new(BudgetLimit::new(100));
        b.charge("alice", CostUnit::new(50)).unwrap();
        assert_eq!(b.spent("alice"), 50);
    }

    #[test]
    fn budget_exceeded_returns_denied() {
        let mut b = AgentBudget::new(BudgetLimit::new(10));
        let err = b.charge("alice", CostUnit::new(11)).unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
        assert!(err.to_string().contains("agent budget exceeded"));
    }

    #[test]
    fn budget_cumulative_charges_tracked() {
        let mut b = AgentBudget::new(BudgetLimit::new(100));
        b.charge("alice", CostUnit::new(30)).unwrap();
        b.charge("alice", CostUnit::new(30)).unwrap();
        assert_eq!(b.spent("alice"), 60);
    }

    #[test]
    fn budget_soft_kill_fires_when_cumulative_exceeds_limit() {
        let mut b = AgentBudget::new(BudgetLimit::new(50));
        b.charge("bob", CostUnit::new(40)).unwrap();
        let err = b.charge("bob", CostUnit::new(20)).unwrap_err(); // 40+20 > 50
        assert!(matches!(err, DcgError::Denied { .. }), "soft-kill must fire");
    }

    #[test]
    fn budget_different_agents_have_separate_budgets() {
        let mut b = AgentBudget::new(BudgetLimit::new(50));
        b.charge("alice", CostUnit::new(50)).unwrap();
        // bob has spent nothing — his charge should succeed
        b.charge("bob", CostUnit::new(50)).unwrap();
        assert_eq!(b.spent("alice"), 50);
        assert_eq!(b.spent("bob"), 50);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Congestion throttle
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn congestion_sensor_zero_depth_reader_always_returns_zero() {
        let r = ZeroDepthReader;
        assert_eq!(r.read().get(), 0);
    }

    #[test]
    fn congestion_below_threshold_no_aimd_decrease() {
        let mut aimd = AimdController::new(MaxInflight::new(4).unwrap(), 1, 0.5);
        let depth = QueueDepth::new(5);
        let threshold = CongestionThreshold::new(10);
        let before = aimd.effective_width();
        if depth.get() >= threshold.get() {
            aimd.decrease();
        }
        assert_eq!(aimd.effective_width(), before, "below threshold: no decrease");
    }

    #[test]
    fn congestion_at_threshold_triggers_aimd_decrease() {
        let mut aimd = AimdController::new(MaxInflight::new(8).unwrap(), 1, 0.5);
        let depth = QueueDepth::new(10);
        let threshold = CongestionThreshold::new(10);
        let before = aimd.effective_width();
        if depth.get() >= threshold.get() {
            aimd.decrease();
        }
        assert!(aimd.effective_width() < before, "at threshold: decrease must fire");
    }

    #[test]
    fn congestion_above_threshold_triggers_aimd_decrease() {
        let mut aimd = AimdController::new(MaxInflight::new(8).unwrap(), 1, 0.5);
        let depth = QueueDepth::new(50);
        let threshold = CongestionThreshold::new(10);
        let before = aimd.effective_width();
        if depth.get() >= threshold.get() {
            aimd.decrease();
        }
        assert!(aimd.effective_width() < before);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Governor integration
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn governor_happy_path_returns_ok() {
        let clock = stub_clock();
        let g = make_governor(4, 3, 0, clock);
        let result = g.govern("alice", CostUnit::new(1), || Ok::<u32, DcgError>(99));
        assert_eq!(result.unwrap(), 99);
    }

    #[test]
    fn governor_not_armed_surfaces_without_retry() {
        let clock = stub_clock();
        let g = make_governor(4, 3, 0, clock);
        let calls = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&calls);
        let err = g
            .govern("alice", CostUnit::new(1), move || {
                c.fetch_add(1, Ordering::Relaxed);
                Err::<(), _>(DcgError::NotArmed { key: "k".to_string() })
            })
            .unwrap_err();
        assert!(matches!(err, DcgError::NotArmed { .. }));
        assert_eq!(calls.load(Ordering::Relaxed), 1, "non-transient: no retries");
    }

    #[test]
    fn governor_subprocess_retried_then_success() {
        let clock = stub_clock();
        let g = make_governor(4, 3, 0, clock);
        let attempts = Arc::new(AtomicU64::new(0));
        let a = Arc::clone(&attempts);
        let result = g.govern("alice", CostUnit::new(1), move || {
            let n = a.fetch_add(1, Ordering::Relaxed);
            if n < 2 {
                Err(DcgError::Subprocess { command: "cmd".to_string(), detail: "t".to_string() })
            } else {
                Ok::<u8, DcgError>(1)
            }
        });
        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn governor_budget_exceeded_returns_denied() {
        let config = GovernorConfig {
            max_inflight: MaxInflight::new(4).unwrap(),
            max_retries: MaxRetries::new(0),
            failure_threshold: FailureThreshold::new(5).unwrap(),
            half_open_timeout_ms: HalfOpenTimeoutMs::new(1_000),
            aimd_increase_step: 1,
            aimd_decrease_factor: 0.5,
            agent_budget_limit: BudgetLimit::new(5), // very small
            congestion_threshold: CongestionThreshold::new(100),
        };
        let clock: Arc<dyn MonotonicClock> = stub_clock();
        let g = Governor::new(
            config,
            Arc::new(ZeroBackoff),
            Arc::new(RecordingSleeper::new()),
            Arc::new(ZeroDepthReader),
            clock,
        );
        let err = g
            .govern("alice", CostUnit::new(10), || Ok::<(), DcgError>(()))
            .unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
        assert!(err.to_string().contains("budget"));
    }

    #[test]
    fn governor_circuit_open_returns_denied() {
        // Use a threshold of 1 so one failure opens it.
        let config = GovernorConfig {
            max_inflight: MaxInflight::new(4).unwrap(),
            max_retries: MaxRetries::new(0),
            failure_threshold: FailureThreshold::new(1).unwrap(),
            half_open_timeout_ms: HalfOpenTimeoutMs::new(60_000), // won't expire in test
            aimd_increase_step: 1,
            aimd_decrease_factor: 0.5,
            agent_budget_limit: BudgetLimit::new(u64::MAX),
            congestion_threshold: CongestionThreshold::new(100),
        };
        let clock: Arc<dyn MonotonicClock> = stub_clock();
        let g = Governor::new(
            config,
            Arc::new(ZeroBackoff),
            Arc::new(RecordingSleeper::new()),
            Arc::new(ZeroDepthReader),
            Arc::clone(&clock),
        );

        // Trigger one failure to open the breaker.
        let _ = g.govern("alice", CostUnit::new(1), || {
            Err::<(), _>(DcgError::Subprocess { command: "c".to_string(), detail: "d".to_string() })
        });

        // Next call should be denied by the open breaker.
        let err = g
            .govern("alice", CostUnit::new(1), || Ok::<(), DcgError>(()))
            .unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
        assert!(err.to_string().contains("circuit open"));
    }

    #[test]
    fn governor_semaphore_bounds_inflight() {
        // Width=1: a second concurrent attempt cannot hold the semaphore simultaneously.
        let config = GovernorConfig {
            max_inflight: MaxInflight::new(1).unwrap(),
            max_retries: MaxRetries::new(0),
            failure_threshold: FailureThreshold::new(100).unwrap(),
            half_open_timeout_ms: HalfOpenTimeoutMs::new(1_000),
            aimd_increase_step: 1,
            aimd_decrease_factor: 0.5,
            agent_budget_limit: BudgetLimit::new(u64::MAX),
            congestion_threshold: CongestionThreshold::new(100),
        };
        let clock: Arc<dyn MonotonicClock> = stub_clock();
        let g = Arc::new(Governor::new(
            config,
            Arc::new(ZeroBackoff),
            Arc::new(RecordingSleeper::new()),
            Arc::new(ZeroDepthReader),
            clock,
        ));

        // Measure max concurrent via atomic peak counter.
        let concurrent = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU64::new(0));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let g2 = Arc::clone(&g);
            let con = Arc::clone(&concurrent);
            let pk = Arc::clone(&peak);
            handles.push(std::thread::spawn(move || {
                let _ = g2.govern("agent", CostUnit::new(1), move || {
                    let c = con.fetch_add(1, Ordering::SeqCst) + 1;
                    pk.fetch_max(c, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    con.fetch_sub(1, Ordering::SeqCst);
                    Ok::<(), DcgError>(())
                });
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "with width=1, peak concurrent must be 1"
        );
    }

    #[test]
    fn governor_aimd_soft_cap_blocks_when_congested() {
        // AIMD effective width starts at max_inflight=1.
        // Hold the semaphore's only permit externally so in_flight=1.
        // AIMD will report congested and deny the next govern() call.
        let config = GovernorConfig {
            max_inflight: MaxInflight::new(1).unwrap(),
            max_retries: MaxRetries::new(0),
            failure_threshold: FailureThreshold::new(100).unwrap(),
            half_open_timeout_ms: HalfOpenTimeoutMs::new(1_000),
            aimd_increase_step: 0, // never increases
            aimd_decrease_factor: 1.0,
            agent_budget_limit: BudgetLimit::new(u64::MAX),
            congestion_threshold: CongestionThreshold::new(100),
        };
        let clock: Arc<dyn MonotonicClock> = stub_clock();
        let g = Arc::new(Governor::new(
            config,
            Arc::new(ZeroBackoff),
            Arc::new(RecordingSleeper::new()),
            Arc::new(ZeroDepthReader),
            clock,
        ));

        // Acquire the semaphore externally to set in_flight=1.
        let _permit = g.semaphore.acquire().unwrap();

        // Now in_flight(1) >= aimd.effective_width(1) → AIMD backpressure denied.
        let err = g
            .govern("alice", CostUnit::new(1), || Ok::<(), DcgError>(()))
            .unwrap_err();
        assert!(matches!(err, DcgError::Denied { .. }));
        assert!(err.to_string().contains("aimd backpressure"));
    }

    #[test]
    fn governor_congestion_reader_triggers_aimd_decrease() {
        // High queue depth → AIMD decrease. We verify effective_width drops from
        // max by inspecting via the governor's aimd field after a govern() call.
        let config = GovernorConfig {
            max_inflight: MaxInflight::new(8).unwrap(),
            max_retries: MaxRetries::new(0),
            failure_threshold: FailureThreshold::new(100).unwrap(),
            half_open_timeout_ms: HalfOpenTimeoutMs::new(1_000),
            aimd_increase_step: 1,
            aimd_decrease_factor: 0.5,
            agent_budget_limit: BudgetLimit::new(u64::MAX),
            congestion_threshold: CongestionThreshold::new(5),
        };
        let clock: Arc<dyn MonotonicClock> = stub_clock();
        let g = Governor::new(
            config,
            Arc::new(ZeroBackoff),
            Arc::new(RecordingSleeper::new()),
            Arc::new(FixedDepth(10)), // depth(10) >= threshold(5)
            clock,
        );

        // Successful govern — congestion decrease fires first, then success increase.
        // Net effect depends on ordering; the key test is that AIMD interacted with the signal.
        let _ = g.govern("alice", CostUnit::new(1), || Ok::<(), DcgError>(()));
        // After success increase by 1, starting from a decreased value.
        // effective_width starts at 8, decrease makes it 4, success +1 = 5.
        let eff = g.aimd.lock().unwrap().effective_width();
        // FALSIFIABLE: deterministic (stub clock + FixedDepth(10) + ZeroBackoff).
        // congestion(depth 10 >= threshold 5) decreases 8 -> floor(8*0.5)=4, then the
        // successful govern increases +1 -> 5. A broken/absent congestion-AIMD path
        // would leave eff at 8 (or 8+1 capped to 8), NOT 5 — so == 5 actually proves it fired.
        assert_eq!(eff, 5, "congestion (depth>=threshold) must trigger AIMD decrease 8->4, then success +1 -> 5");
    }
}
