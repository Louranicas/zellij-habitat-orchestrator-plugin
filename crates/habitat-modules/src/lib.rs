// Charter §2 suppression discipline:
// The lints below are scoped crate-wide because they apply to a recurring
// display/render pattern, not to specific sites. Scope: habitat-modules
// renders terminal UI where bar widths, progress percentages, and count
// displays are derived from wire-schema numeric fields. f64 precision loss,
// truncation, and sign loss are accepted for display-only computations.
// Revisit if we ever render values >= 2^53 exactly (we do not today).
#![allow(
    clippy::cast_precision_loss,
    reason = "display-only count → f64 rounding accepted for K/M formatting and percentages"
)]
#![allow(
    clippy::cast_possible_truncation,
    reason = "render bar widths are bounded by cols (usize) derived from non-negative config ranges"
)]
#![allow(
    clippy::cast_sign_loss,
    reason = "coherence r ∈ [0,1] by PV2 contract; cast to usize after clamp"
)]
#![allow(
    clippy::wildcard_imports,
    reason = "habitat_core::render::* is a curated ANSI + RenderLine symbol set used in every module"
)]

pub mod bridge_health;
pub mod campaign_attention;
pub mod cmd_pipe;
pub mod coherence_gauge;
pub mod event_feed;
pub mod fiber_cockpit;
pub mod fleet_view;
pub mod na_panel;
pub mod orchestrator_kernel;
pub mod orchestrator_witness;
pub mod session_timer;
pub mod sphere_warden;
