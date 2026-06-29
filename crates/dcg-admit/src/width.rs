//! DCG D2: Instantaneous fan-out width computation.
//!
//! The fan-out width is `MIN` over all *present* ceilings. The semaphore is the
//! sole hard cap and is always required. Optional ceilings reduce the width when
//! they are smaller than the semaphore; when absent they are treated as unbounded
//! (excluded from the minimum entirely).
//!
//! # Antichain ceiling (R-D7)
//!
//! `antichain` represents the maximum number of causally independent tasks that
//! can proceed in the live task-DAG without violating partial-order constraints.
//! It is **speculative** — the measurement algorithm (R-D7) is not yet deployed.
//!
//! Until R-D7 ships:
//!
//! - `antichain = None` is the correct default (unbounded: excluded from MIN).
//! - Do **not** guess a value; use the live gauge once R-D7 ships.
//! - `antichain = Some(n)` is accepted for forward-compatibility, but `n` MUST
//!   come from the measurement, not from intuition.
//!
//! # CLI surface
//!
//! ```text
//! dcg-admit width --semaphore N [--model-tier M] [--budget-soft B] [--antichain A]
//! ```
//!
//! Prints a single-line JSON object: `{"width":<u8>,"bound_by":[...]}`.

use serde::{Deserialize, Serialize};

use crate::error::DcgError;
use crate::Result;

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// The computed instantaneous fan-out width, bounded to `[0, 255]`.
///
/// A width of `0` is a valid (and safe) operational state — it means the active
/// ceilings permit zero concurrent agents (for example during maintenance or
/// when the semaphore drops to zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Width(u8);

impl Width {
    /// Wraps a raw width value. Every `u8` is in range; the full `[0, 255]`
    /// interval is valid.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the underlying width value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl std::fmt::Display for Width {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The ceiling that imposed the computed [`Width`].
///
/// When multiple ceilings tie at the minimum, all tying ceilings appear in
/// [`WidthResult::bound_by`] in deterministic order:
/// `Semaphore` → `ModelTier` → `BudgetSoft` → `Antichain`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundCeiling {
    /// The semaphore permit count — the only hard cap.
    Semaphore,
    /// The model-tier soft cap.
    ModelTier,
    /// The budget-headroom soft cap.
    BudgetSoft,
    /// The partial-order antichain cap (speculative R-D7).
    Antichain,
}

/// The set of active ceilings for a fan-out width computation.
///
/// `semaphore` is the only hard cap and is always required. Optional ceilings
/// are omitted (treated as unbounded) when `None`.
///
/// # Antichain note
///
/// `antichain` defaults to `None` (unbounded) because the antichain measurement
/// algorithm (R-D7) is not yet deployed. Do not supply a value unless the live
/// gauge is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidthCeilings {
    /// Semaphore permit count — the only hard cap; always required.
    pub semaphore: u8,
    /// Optional soft cap imposed by the model tier currently active.
    pub model_tier: Option<u8>,
    /// Optional soft cap imposed by available budget headroom.
    pub budget_soft: Option<u8>,
    /// Optional partial-order antichain cap (speculative R-D7; `None` =
    /// unbounded). Only supply when the live antichain gauge is running.
    pub antichain: Option<u8>,
}

/// Result of a [`compute_width`] call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidthResult {
    /// The computed instantaneous fan-out width.
    pub width: Width,
    /// All ceilings that jointly impose `width` (typically one; more on ties).
    /// Order is deterministic: `Semaphore` → `ModelTier` → `BudgetSoft` → `Antichain`.
    pub bound_by: Vec<BoundCeiling>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Core computation
// ──────────────────────────────────────────────────────────────────────────────

/// Computes the instantaneous fan-out width from the active ceilings.
///
/// Width equals the minimum over all *present* ceilings. The semaphore is
/// always present. Each `Option` ceiling contributes to the minimum only when
/// `Some`; `None` means unbounded and is excluded from the minimum.
///
/// On ties all tying ceilings appear in [`WidthResult::bound_by`] in
/// deterministic order: `Semaphore` → `ModelTier` → `BudgetSoft` → `Antichain`.
#[must_use]
pub fn compute_width(ceilings: &WidthCeilings) -> WidthResult {
    // Seed from the semaphore (the only hard cap; always present).
    let mut min_val = ceilings.semaphore;
    let mut bound_by = vec![BoundCeiling::Semaphore];

    // Evaluate each optional ceiling in deterministic priority order.
    // Moving owned values out of the array is correct: Option<u8> is Copy,
    // BoundCeiling is Clone/move.
    for (opt_val, label) in [
        (ceilings.model_tier, BoundCeiling::ModelTier),
        (ceilings.budget_soft, BoundCeiling::BudgetSoft),
        (ceilings.antichain, BoundCeiling::Antichain),
    ] {
        let Some(v) = opt_val else {
            continue;
        };
        match v.cmp(&min_val) {
            std::cmp::Ordering::Less => {
                min_val = v;
                bound_by = vec![label];
            }
            std::cmp::Ordering::Equal => {
                bound_by.push(label);
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    WidthResult {
        width: Width::new(min_val),
        bound_by,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// CLI helpers (pub(crate) — used by cli::run dispatch)
// ──────────────────────────────────────────────────────────────────────────────

/// Parsed CLI arguments for the `dcg-admit width` subcommand.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WidthCliArgs {
    /// Semaphore ceiling (required).
    pub(crate) semaphore: u8,
    /// Optional model-tier ceiling.
    pub(crate) model_tier: Option<u8>,
    /// Optional budget-soft ceiling.
    pub(crate) budget_soft: Option<u8>,
    /// Optional antichain ceiling (speculative R-D7; `None` = unbounded).
    pub(crate) antichain: Option<u8>,
}

/// Parses arguments for the `dcg-admit width` subcommand.
///
/// # Errors
///
/// Returns [`DcgError::Empty`] when `--semaphore` is absent.
/// Returns [`DcgError::Parse`] when any value is not a valid `u8` (out of
/// range or non-numeric), when a flag is unknown, or when a value token is
/// missing after a flag.
pub(crate) fn parse_width_args(args: &[String]) -> Result<WidthCliArgs> {
    let mut semaphore: Option<u8> = None;
    let mut model_tier: Option<u8> = None;
    let mut budget_soft: Option<u8> = None;
    let mut antichain: Option<u8> = None;

    let mut i = 0_usize;
    while i < args.len() {
        match args[i].as_str() {
            "--semaphore" => {
                semaphore = Some(parse_u8_flag(args, i, "--semaphore")?);
                i += 2;
            }
            "--model-tier" => {
                model_tier = Some(parse_u8_flag(args, i, "--model-tier")?);
                i += 2;
            }
            "--budget-soft" => {
                budget_soft = Some(parse_u8_flag(args, i, "--budget-soft")?);
                i += 2;
            }
            "--antichain" => {
                antichain = Some(parse_u8_flag(args, i, "--antichain")?);
                i += 2;
            }
            other => {
                return Err(DcgError::Parse {
                    source: "width-args",
                    detail: format!("unknown argument: {other:?}"),
                });
            }
        }
    }

    let semaphore = semaphore.ok_or(DcgError::Empty { field: "--semaphore" })?;
    Ok(WidthCliArgs {
        semaphore,
        model_tier,
        budget_soft,
        antichain,
    })
}

/// Runs the `dcg-admit width` subcommand: computes and prints width as JSON.
///
/// Output is a single-line JSON object on stdout:
/// `{"width":<u8>,"bound_by":[<ceiling>, ...]}`.
///
/// # Errors
///
/// Returns [`DcgError`] if argument parsing fails. JSON serialisation of
/// [`WidthResult`] is infallible by construction, but the function signature
/// surfaces it as [`DcgError::Json`] should a logic bug occur.
pub(crate) fn run_width_command(args: &[String]) -> Result<i32> {
    let cli = parse_width_args(args)?;
    let ceilings = WidthCeilings {
        semaphore: cli.semaphore,
        model_tier: cli.model_tier,
        budget_soft: cli.budget_soft,
        antichain: cli.antichain,
    };
    let result = compute_width(&ceilings);
    let json = serde_json::to_string(&result).map_err(|e| DcgError::Json(e.to_string()))?;
    println!("{json}");
    Ok(0)
}

// ──────────────────────────────────────────────────────────────────────────────
// Private helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Parses and validates a `u8` value for a named CLI flag.
///
/// Accepts any decimal string in `[0, 255]`. Returns an error if the next
/// token is absent, is itself a `--`-prefixed flag, or cannot be parsed as
/// `u8` (overflow or non-numeric).
fn parse_u8_flag(args: &[String], index: usize, flag: &str) -> Result<u8> {
    let raw = args
        .get(index + 1)
        .ok_or(DcgError::Empty { field: "flag value" })?;
    if raw.starts_with("--") {
        return Err(DcgError::Parse {
            source: "width-args",
            detail: format!("{flag} requires a value, got next flag: {raw:?}"),
        });
    }
    raw.parse::<u8>().map_err(|_| DcgError::Parse {
        source: "width-args",
        detail: format!("{flag} must be an integer in [0, 255], got: {raw:?}"),
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // ── Width newtype ─────────────────────────────────────────────────────────

    #[test]
    fn width_new_zero_get_zero() {
        assert_eq!(Width::new(0).get(), 0);
    }

    #[test]
    fn width_new_max_get_max() {
        assert_eq!(Width::new(255).get(), 255);
    }

    #[test]
    fn width_new_mid_get_mid() {
        assert_eq!(Width::new(128).get(), 128);
    }

    #[test]
    fn width_one_get_one() {
        assert_eq!(Width::new(1).get(), 1);
    }

    #[test]
    fn width_copy_original_still_usable() {
        let a = Width::new(7);
        let b = a; // Copy
        assert_eq!(a.get(), b.get());
    }

    #[test]
    fn width_clone_equals_original() {
        let a = Width::new(42);
        assert_eq!(a.clone(), a);
    }

    #[test]
    fn width_debug_contains_value() {
        assert!(format!("{:?}", Width::new(13)).contains("13"));
    }

    #[test]
    fn width_display_is_plain_number() {
        assert_eq!(Width::new(99).to_string(), "99");
    }

    #[test]
    fn width_display_zero() {
        assert_eq!(Width::new(0).to_string(), "0");
    }

    #[test]
    fn width_eq_same_values() {
        assert_eq!(Width::new(5), Width::new(5));
    }

    #[test]
    fn width_ne_different_values() {
        assert_ne!(Width::new(5), Width::new(6));
    }

    #[test]
    fn width_ord_less_than() {
        assert!(Width::new(1) < Width::new(2));
    }

    #[test]
    fn width_ord_greater_than() {
        assert!(Width::new(3) > Width::new(2));
    }

    #[test]
    fn width_serialize_transparent_as_u8() {
        let j = serde_json::to_string(&Width::new(42)).unwrap();
        assert_eq!(j, "42");
    }

    #[test]
    fn width_serialize_zero() {
        let j = serde_json::to_string(&Width::new(0)).unwrap();
        assert_eq!(j, "0");
    }

    #[test]
    fn width_deserialize_from_integer() {
        let w: Width = serde_json::from_str("17").unwrap();
        assert_eq!(w.get(), 17);
    }

    // ── BoundCeiling ──────────────────────────────────────────────────────────

    #[test]
    fn bound_ceiling_semaphore_serializes_snake_case() {
        let j = serde_json::to_string(&BoundCeiling::Semaphore).unwrap();
        assert_eq!(j, r#""semaphore""#);
    }

    #[test]
    fn bound_ceiling_model_tier_serializes_snake_case() {
        let j = serde_json::to_string(&BoundCeiling::ModelTier).unwrap();
        assert_eq!(j, r#""model_tier""#);
    }

    #[test]
    fn bound_ceiling_budget_soft_serializes_snake_case() {
        let j = serde_json::to_string(&BoundCeiling::BudgetSoft).unwrap();
        assert_eq!(j, r#""budget_soft""#);
    }

    #[test]
    fn bound_ceiling_antichain_serializes_snake_case() {
        let j = serde_json::to_string(&BoundCeiling::Antichain).unwrap();
        assert_eq!(j, r#""antichain""#);
    }

    #[test]
    fn bound_ceiling_deserialize_semaphore() {
        let b: BoundCeiling = serde_json::from_str(r#""semaphore""#).unwrap();
        assert_eq!(b, BoundCeiling::Semaphore);
    }

    #[test]
    fn bound_ceiling_deserialize_model_tier() {
        let b: BoundCeiling = serde_json::from_str(r#""model_tier""#).unwrap();
        assert_eq!(b, BoundCeiling::ModelTier);
    }

    #[test]
    fn bound_ceiling_clone_equals() {
        assert_eq!(BoundCeiling::Semaphore.clone(), BoundCeiling::Semaphore);
    }

    #[test]
    fn bound_ceiling_debug_contains_variant_name() {
        assert!(format!("{:?}", BoundCeiling::ModelTier).contains("ModelTier"));
    }

    // ── compute_width — semaphore-only ────────────────────────────────────────

    #[test]
    fn semaphore_only_width_equals_semaphore() {
        let c = WidthCeilings { semaphore: 8, model_tier: None, budget_soft: None, antichain: None };
        assert_eq!(compute_width(&c).width.get(), 8);
    }

    #[test]
    fn semaphore_only_bound_by_is_semaphore() {
        let c = WidthCeilings { semaphore: 8, model_tier: None, budget_soft: None, antichain: None };
        assert_eq!(compute_width(&c).bound_by, vec![BoundCeiling::Semaphore]);
    }

    #[test]
    fn semaphore_zero_yields_width_zero() {
        let c = WidthCeilings { semaphore: 0, model_tier: None, budget_soft: None, antichain: None };
        assert_eq!(compute_width(&c).width.get(), 0);
    }

    #[test]
    fn semaphore_zero_bound_by_is_semaphore() {
        let c = WidthCeilings { semaphore: 0, model_tier: None, budget_soft: None, antichain: None };
        assert_eq!(compute_width(&c).bound_by, vec![BoundCeiling::Semaphore]);
    }

    #[test]
    fn semaphore_max_only_yields_max_width() {
        let c = WidthCeilings { semaphore: 255, model_tier: None, budget_soft: None, antichain: None };
        assert_eq!(compute_width(&c).width.get(), 255);
    }

    // ── compute_width — single optional ceiling wins ──────────────────────────

    #[test]
    fn model_tier_less_than_semaphore_wins() {
        let c = WidthCeilings { semaphore: 10, model_tier: Some(5), budget_soft: None, antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 5);
        assert_eq!(r.bound_by, vec![BoundCeiling::ModelTier]);
    }

    #[test]
    fn budget_soft_less_than_semaphore_wins() {
        let c = WidthCeilings { semaphore: 10, model_tier: None, budget_soft: Some(3), antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 3);
        assert_eq!(r.bound_by, vec![BoundCeiling::BudgetSoft]);
    }

    #[test]
    fn antichain_less_than_semaphore_wins() {
        let c = WidthCeilings { semaphore: 10, model_tier: None, budget_soft: None, antichain: Some(7) };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 7);
        assert_eq!(r.bound_by, vec![BoundCeiling::Antichain]);
    }

    // ── compute_width — optional ceiling does NOT win ─────────────────────────

    #[test]
    fn model_tier_greater_than_semaphore_semaphore_wins() {
        let c = WidthCeilings { semaphore: 4, model_tier: Some(10), budget_soft: None, antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 4);
        assert_eq!(r.bound_by, vec![BoundCeiling::Semaphore]);
    }

    #[test]
    fn budget_soft_greater_than_semaphore_semaphore_wins() {
        let c = WidthCeilings { semaphore: 4, model_tier: None, budget_soft: Some(10), antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 4);
        assert_eq!(r.bound_by, vec![BoundCeiling::Semaphore]);
    }

    #[test]
    fn antichain_greater_than_semaphore_semaphore_wins() {
        let c = WidthCeilings { semaphore: 4, model_tier: None, budget_soft: None, antichain: Some(10) };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 4);
        assert_eq!(r.bound_by, vec![BoundCeiling::Semaphore]);
    }

    // ── compute_width — antichain absent = unbounded (R-D7 speculative) ───────

    #[test]
    fn antichain_absent_not_in_bound_by() {
        let c = WidthCeilings { semaphore: 3, model_tier: None, budget_soft: None, antichain: None };
        assert!(!compute_width(&c).bound_by.contains(&BoundCeiling::Antichain));
    }

    #[test]
    fn antichain_absent_does_not_reduce_width() {
        let c = WidthCeilings { semaphore: 8, model_tier: None, budget_soft: None, antichain: None };
        assert_eq!(compute_width(&c).width.get(), 8);
    }

    #[test]
    fn antichain_absent_with_other_optionals_present() {
        // antichain=None; model_tier=5 wins over semaphore=10
        let c = WidthCeilings { semaphore: 10, model_tier: Some(5), budget_soft: None, antichain: None };
        assert!(!compute_width(&c).bound_by.contains(&BoundCeiling::Antichain));
        assert_eq!(compute_width(&c).width.get(), 5);
    }

    // ── compute_width — all optionals present ────────────────────────────────

    #[test]
    fn all_present_model_tier_smallest() {
        let c = WidthCeilings { semaphore: 10, model_tier: Some(2), budget_soft: Some(5), antichain: Some(7) };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 2);
        assert_eq!(r.bound_by, vec![BoundCeiling::ModelTier]);
    }

    #[test]
    fn all_present_budget_soft_smallest() {
        let c = WidthCeilings { semaphore: 10, model_tier: Some(8), budget_soft: Some(1), antichain: Some(5) };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 1);
        assert_eq!(r.bound_by, vec![BoundCeiling::BudgetSoft]);
    }

    #[test]
    fn all_present_antichain_smallest() {
        let c = WidthCeilings { semaphore: 10, model_tier: Some(8), budget_soft: Some(6), antichain: Some(3) };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 3);
        assert_eq!(r.bound_by, vec![BoundCeiling::Antichain]);
    }

    #[test]
    fn all_present_semaphore_smallest() {
        let c = WidthCeilings { semaphore: 1, model_tier: Some(8), budget_soft: Some(6), antichain: Some(3) };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 1);
        assert_eq!(r.bound_by, vec![BoundCeiling::Semaphore]);
    }

    // ── compute_width — ties ─────────────────────────────────────────────────

    #[test]
    fn tie_semaphore_model_tier_both_appear() {
        let c = WidthCeilings { semaphore: 5, model_tier: Some(5), budget_soft: None, antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 5);
        assert!(r.bound_by.contains(&BoundCeiling::Semaphore));
        assert!(r.bound_by.contains(&BoundCeiling::ModelTier));
        assert_eq!(r.bound_by.len(), 2);
    }

    #[test]
    fn tie_semaphore_budget_soft_both_appear() {
        let c = WidthCeilings { semaphore: 5, model_tier: None, budget_soft: Some(5), antichain: None };
        let r = compute_width(&c);
        assert!(r.bound_by.contains(&BoundCeiling::Semaphore));
        assert!(r.bound_by.contains(&BoundCeiling::BudgetSoft));
        assert_eq!(r.bound_by.len(), 2);
    }

    #[test]
    fn tie_semaphore_antichain_both_appear() {
        let c = WidthCeilings { semaphore: 4, model_tier: None, budget_soft: None, antichain: Some(4) };
        let r = compute_width(&c);
        assert!(r.bound_by.contains(&BoundCeiling::Semaphore));
        assert!(r.bound_by.contains(&BoundCeiling::Antichain));
        assert_eq!(r.bound_by.len(), 2);
    }

    #[test]
    fn tie_model_tier_budget_soft_both_below_semaphore() {
        let c = WidthCeilings { semaphore: 10, model_tier: Some(3), budget_soft: Some(3), antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 3);
        assert!(!r.bound_by.contains(&BoundCeiling::Semaphore));
        assert!(r.bound_by.contains(&BoundCeiling::ModelTier));
        assert!(r.bound_by.contains(&BoundCeiling::BudgetSoft));
        assert_eq!(r.bound_by.len(), 2);
    }

    #[test]
    fn tie_all_four_equal() {
        let c = WidthCeilings { semaphore: 3, model_tier: Some(3), budget_soft: Some(3), antichain: Some(3) };
        let r = compute_width(&c);
        assert_eq!(r.width.get(), 3);
        assert_eq!(r.bound_by.len(), 4);
        assert!(r.bound_by.contains(&BoundCeiling::Semaphore));
        assert!(r.bound_by.contains(&BoundCeiling::ModelTier));
        assert!(r.bound_by.contains(&BoundCeiling::BudgetSoft));
        assert!(r.bound_by.contains(&BoundCeiling::Antichain));
    }

    #[test]
    fn tie_three_equal_antichain_absent() {
        let c = WidthCeilings { semaphore: 5, model_tier: Some(5), budget_soft: Some(5), antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.bound_by.len(), 3);
        assert!(!r.bound_by.contains(&BoundCeiling::Antichain));
    }

    // ── compute_width — bound_by ordering is deterministic ────────────────────

    #[test]
    fn bound_by_order_semaphore_before_model_tier_on_tie() {
        let c = WidthCeilings { semaphore: 5, model_tier: Some(5), budget_soft: None, antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.bound_by[0], BoundCeiling::Semaphore);
        assert_eq!(r.bound_by[1], BoundCeiling::ModelTier);
    }

    #[test]
    fn bound_by_order_model_tier_before_budget_soft_on_tie() {
        let c = WidthCeilings { semaphore: 10, model_tier: Some(3), budget_soft: Some(3), antichain: None };
        let r = compute_width(&c);
        assert_eq!(r.bound_by[0], BoundCeiling::ModelTier);
        assert_eq!(r.bound_by[1], BoundCeiling::BudgetSoft);
    }

    #[test]
    fn bound_by_order_budget_soft_before_antichain_on_tie() {
        let c = WidthCeilings { semaphore: 10, model_tier: None, budget_soft: Some(3), antichain: Some(3) };
        let r = compute_width(&c);
        assert_eq!(r.bound_by[0], BoundCeiling::BudgetSoft);
        assert_eq!(r.bound_by[1], BoundCeiling::Antichain);
    }

    #[test]
    fn bound_by_never_empty() {
        for s in [0_u8, 1, 127, 255] {
            let c = WidthCeilings { semaphore: s, model_tier: None, budget_soft: None, antichain: None };
            assert!(!compute_width(&c).bound_by.is_empty());
        }
    }

    // ── parse_width_args — valid inputs ──────────────────────────────────────

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parse_semaphore_only() {
        let a = parse_width_args(&argv(&["--semaphore", "4"])).unwrap();
        assert_eq!(a.semaphore, 4);
        assert_eq!(a.model_tier, None);
        assert_eq!(a.budget_soft, None);
        assert_eq!(a.antichain, None);
    }

    #[test]
    fn parse_all_flags() {
        let a = parse_width_args(&argv(&[
            "--semaphore", "10",
            "--model-tier", "8",
            "--budget-soft", "6",
            "--antichain", "4",
        ])).unwrap();
        assert_eq!(a.semaphore, 10);
        assert_eq!(a.model_tier, Some(8));
        assert_eq!(a.budget_soft, Some(6));
        assert_eq!(a.antichain, Some(4));
    }

    #[test]
    fn parse_semaphore_zero_valid() {
        let a = parse_width_args(&argv(&["--semaphore", "0"])).unwrap();
        assert_eq!(a.semaphore, 0);
    }

    #[test]
    fn parse_semaphore_max_valid() {
        let a = parse_width_args(&argv(&["--semaphore", "255"])).unwrap();
        assert_eq!(a.semaphore, 255);
    }

    #[test]
    fn parse_model_tier_none_when_absent() {
        let a = parse_width_args(&argv(&["--semaphore", "5"])).unwrap();
        assert_eq!(a.model_tier, None);
    }

    #[test]
    fn parse_antichain_none_when_absent() {
        let a = parse_width_args(&argv(&["--semaphore", "5"])).unwrap();
        assert_eq!(a.antichain, None);
    }

    #[test]
    fn parse_flags_any_order() {
        let a = parse_width_args(&argv(&[
            "--antichain", "2",
            "--semaphore", "9",
            "--budget-soft", "7",
        ])).unwrap();
        assert_eq!(a.semaphore, 9);
        assert_eq!(a.budget_soft, Some(7));
        assert_eq!(a.antichain, Some(2));
    }

    // ── parse_width_args — error cases ───────────────────────────────────────

    #[test]
    fn parse_missing_semaphore_returns_empty_error() {
        let err = parse_width_args(&argv(&["--model-tier", "3"])).unwrap_err();
        assert!(matches!(err, DcgError::Empty { field: "--semaphore" }));
    }

    #[test]
    fn parse_empty_args_missing_semaphore() {
        let err = parse_width_args(&[]).unwrap_err();
        assert!(matches!(err, DcgError::Empty { field: "--semaphore" }));
    }

    #[test]
    fn parse_semaphore_not_a_number_returns_parse_error() {
        let err = parse_width_args(&argv(&["--semaphore", "abc"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    #[test]
    fn parse_semaphore_negative_rejected() {
        let err = parse_width_args(&argv(&["--semaphore", "-1"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    #[test]
    fn parse_semaphore_overflow_rejected() {
        let err = parse_width_args(&argv(&["--semaphore", "256"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    #[test]
    fn parse_model_tier_too_large_rejected() {
        let err = parse_width_args(&argv(&["--semaphore", "4", "--model-tier", "300"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    #[test]
    fn parse_budget_soft_too_large_rejected() {
        let err = parse_width_args(&argv(&["--semaphore", "4", "--budget-soft", "999"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    #[test]
    fn parse_antichain_too_large_rejected() {
        let err = parse_width_args(&argv(&["--semaphore", "4", "--antichain", "256"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    #[test]
    fn parse_unknown_flag_returns_parse_error() {
        let err = parse_width_args(&argv(&["--semaphore", "4", "--unknown"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    #[test]
    fn parse_unknown_flag_error_message_contains_name() {
        let err = parse_width_args(&argv(&["--semaphore", "4", "--bogus"])).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn parse_flag_without_value_returns_error() {
        let err = parse_width_args(&argv(&["--semaphore"])).unwrap_err();
        assert!(matches!(err, DcgError::Empty { .. }));
    }

    #[test]
    fn parse_flag_followed_by_another_flag_returns_parse_error() {
        let err = parse_width_args(&argv(&["--semaphore", "--model-tier"])).unwrap_err();
        assert!(matches!(err, DcgError::Parse { source: "width-args", .. }));
    }

    // ── WidthResult serialisation ─────────────────────────────────────────────

    #[test]
    fn width_result_json_has_width_key() {
        let r = WidthResult { width: Width::new(5), bound_by: vec![BoundCeiling::Semaphore] };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"width\""));
    }

    #[test]
    fn width_result_json_has_bound_by_key() {
        let r = WidthResult { width: Width::new(5), bound_by: vec![BoundCeiling::Semaphore] };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"bound_by\""));
    }

    #[test]
    fn width_result_json_width_value_is_number() {
        let r = WidthResult { width: Width::new(7), bound_by: vec![BoundCeiling::Semaphore] };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(v["width"], 7_u8);
    }

    #[test]
    fn width_result_json_bound_by_is_array() {
        let r = WidthResult { width: Width::new(7), bound_by: vec![BoundCeiling::Semaphore] };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(v["bound_by"].is_array());
    }

    #[test]
    fn width_result_deserialize_roundtrip() {
        let original = WidthResult {
            width: Width::new(5),
            bound_by: vec![BoundCeiling::Semaphore, BoundCeiling::ModelTier],
        };
        let json = serde_json::to_string(&original).unwrap();
        let recovered: WidthResult = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, original);
    }

    #[test]
    fn width_result_debug_contains_width_value() {
        let r = WidthResult { width: Width::new(42), bound_by: vec![BoundCeiling::Semaphore] };
        assert!(format!("{r:?}").contains("42"));
    }

    #[test]
    fn width_result_clone_equals() {
        let r = WidthResult { width: Width::new(3), bound_by: vec![BoundCeiling::BudgetSoft] };
        assert_eq!(r.clone(), r);
    }

    // ── run_width_command ─────────────────────────────────────────────────────

    #[test]
    fn run_width_command_valid_args_returns_zero() {
        let a = argv(&["--semaphore", "4"]);
        assert_eq!(run_width_command(&a).unwrap(), 0);
    }

    #[test]
    fn run_width_command_missing_semaphore_returns_error() {
        let a = argv(&["--model-tier", "4"]);
        assert!(run_width_command(&a).is_err());
    }

    #[test]
    fn run_width_command_unknown_flag_returns_error() {
        let a = argv(&["--semaphore", "4", "--unknown"]);
        assert!(run_width_command(&a).is_err());
    }

    #[test]
    fn run_width_command_all_flags_returns_zero() {
        let a = argv(&["--semaphore", "10", "--model-tier", "8", "--budget-soft", "6", "--antichain", "3"]);
        assert_eq!(run_width_command(&a).unwrap(), 0);
    }
}
