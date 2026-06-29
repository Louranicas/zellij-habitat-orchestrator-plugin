//! The monotonic write fence.
//!
//! A [`Fence`] is a bounded newtype over the orchestrator-kernel spine's
//! monotonic sequence (`EventRow.seq`, build-plan §0). It is the anti-stale-write
//! token: an admission is accepted only when the presented fence strictly
//! [`Fence::supersedes`] the resource's last-admitted fence. Fences are minted
//! from the spine `seq` (never wall-clock), so strict monotonicity is inherited
//! from the append-only chain.
//!
//! Architect-owned foundation: fully implemented so the admission logic and the
//! `kv-lease` fence field share one well-defined ordering semantics.

use serde::Serialize;

use crate::error::DcgError;
use crate::Result;

/// A monotonic write fence minted from the spine sequence.
///
/// Ordering is the natural ordering of the wrapped `u64`; [`Fence::supersedes`]
/// is the strict-greater guard used at the admission boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Fence(u64);

impl Fence {
    /// The genesis fence (`0`); precedes every minted fence. A resource with no
    /// prior admission is treated as holding [`Fence::GENESIS`].
    pub const GENESIS: Self = Self(0);

    /// Wraps a raw fence value. All `u64` values are representable; use
    /// [`Fence::from_seq`] to mint from a signed spine sequence with validation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Mints a fence from a spine sequence number.
    ///
    /// # Errors
    /// Returns [`DcgError::OutOfRange`] when `seq` is negative. A valid spine
    /// `seq` is always non-negative; a negative value signals a corrupt or
    /// missing read and must not be coerced into a fence.
    pub fn from_seq(seq: i64) -> Result<Self> {
        u64::try_from(seq)
            .map(Self)
            .map_err(|_| DcgError::OutOfRange {
                field: "fence.seq",
                value: seq.to_string(),
            })
    }

    /// Returns the underlying fence value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns `true` iff `self` strictly supersedes `prior`.
    ///
    /// This is the stale-write guard: an admission is accepted only when the
    /// presented fence strictly exceeds the last-admitted fence. Equality is
    /// rejected (a replay of the same fence is not a fresh write).
    #[must_use]
    pub const fn supersedes(self, prior: Self) -> bool {
        self.0 > prior.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // -----------------------------------------------------------------------
    // new / get round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn new_then_get_roundtrips() {
        for v in [0_u64, 1, 42, 1024, u64::MAX] {
            assert_eq!(Fence::new(v).get(), v);
        }
    }

    #[test]
    fn genesis_is_zero() {
        assert_eq!(Fence::GENESIS.get(), 0);
    }

    // -----------------------------------------------------------------------
    // from_seq validation
    // -----------------------------------------------------------------------

    #[test]
    fn from_seq_zero_ok() {
        assert_eq!(Fence::from_seq(0).unwrap().get(), 0);
    }

    #[test]
    fn from_seq_positive_ok() {
        assert_eq!(Fence::from_seq(3022).unwrap().get(), 3022);
    }

    #[test]
    fn from_seq_max_i64_ok() {
        let f = Fence::from_seq(i64::MAX).unwrap();
        let expected = u64::try_from(i64::MAX).unwrap();
        assert_eq!(f.get(), expected);
    }

    #[test]
    fn from_seq_negative_rejected() {
        assert!(Fence::from_seq(-1).is_err());
    }

    #[test]
    fn from_seq_min_i64_rejected() {
        assert!(Fence::from_seq(i64::MIN).is_err());
    }

    #[test]
    fn from_seq_negative_error_is_out_of_range_and_names_field() {
        let err = Fence::from_seq(-5).unwrap_err();
        assert!(matches!(
            err,
            DcgError::OutOfRange {
                field: "fence.seq",
                ..
            }
        ));
        assert!(err.to_string().contains("fence.seq"));
    }

    // -----------------------------------------------------------------------
    // supersedes — the stale-write guard
    // -----------------------------------------------------------------------

    #[test]
    fn supersedes_strictly_greater_is_true() {
        assert!(Fence::new(2).supersedes(Fence::new(1)));
    }

    #[test]
    fn supersedes_equal_is_false() {
        assert!(!Fence::new(7).supersedes(Fence::new(7)));
    }

    #[test]
    fn supersedes_lesser_is_false() {
        assert!(!Fence::new(1).supersedes(Fence::new(2)));
    }

    #[test]
    fn any_minted_fence_supersedes_genesis() {
        assert!(Fence::new(1).supersedes(Fence::GENESIS));
    }

    #[test]
    fn genesis_does_not_supersede_itself() {
        assert!(!Fence::GENESIS.supersedes(Fence::GENESIS));
    }

    #[test]
    fn supersedes_is_asymmetric() {
        let a = Fence::new(10);
        let b = Fence::new(20);
        assert!(b.supersedes(a));
        assert!(!a.supersedes(b));
    }

    // -----------------------------------------------------------------------
    // Ord / equality
    // -----------------------------------------------------------------------

    #[test]
    fn ordering_matches_inner_value() {
        assert!(Fence::new(1) < Fence::new(2));
        assert!(Fence::new(3) > Fence::new(2));
        assert_eq!(Fence::new(4), Fence::new(4));
    }

    #[test]
    fn sorts_ascending_by_value() {
        let mut v = [Fence::new(3), Fence::new(1), Fence::new(2)];
        v.sort_unstable();
        assert_eq!(v, [Fence::new(1), Fence::new(2), Fence::new(3)]);
    }

    // -----------------------------------------------------------------------
    // Serialization is transparent (matches the kv-lease `fence` JSON field)
    // -----------------------------------------------------------------------

    #[test]
    fn serializes_transparently_as_a_bare_integer() {
        let json = serde_json::to_string(&Fence::new(42)).unwrap();
        assert_eq!(json, "42");
    }

    #[test]
    fn serializes_genesis_as_zero() {
        let json = serde_json::to_string(&Fence::GENESIS).unwrap();
        assert_eq!(json, "0");
    }

    #[test]
    fn serializes_inside_a_struct_field() {
        #[derive(Serialize)]
        struct Wrap {
            fence: Fence,
        }
        let json = serde_json::to_string(&Wrap { fence: Fence::new(9) }).unwrap();
        assert_eq!(json, r#"{"fence":9}"#);
    }
}
