pub const R: &str = "\x1b[0m";
pub const B: &str = "\x1b[1m";
pub const D: &str = "\x1b[2m";
pub const GRN: &str = "\x1b[32m";
pub const YEL: &str = "\x1b[33m";
pub const RED: &str = "\x1b[31m";
pub const CYN: &str = "\x1b[36m";
pub const MAG: &str = "\x1b[35m";
pub const BLU: &str = "\x1b[34m";

pub const ICON_UP: &str = "\u{25cf}";
pub const ICON_DOWN: &str = "\u{25cb}";
pub const ICON_CHECK: &str = "\u{2713}";
pub const ICON_CROSS: &str = "\u{2717}";
pub const HLINE: &str = "\u{2500}";

#[derive(Clone, Debug)]
pub struct RenderLine {
    pub content: String,
}

impl RenderLine {
    #[must_use]
    pub fn new(content: String) -> Self {
        Self { content }
    }

    #[must_use]
    pub fn blank() -> Self {
        Self {
            content: String::new(),
        }
    }

    #[must_use]
    pub fn separator(width: usize) -> Self {
        Self {
            content: format!("{D}{}{R}", HLINE.repeat(width)),
        }
    }
}

#[must_use]
pub fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Format a count for compact display.
///
/// Uses `f64` division for the K/M thresholds. The precision-loss lint
/// is allowed here because the output is a 1-decimal display string —
/// a u64 above 2^53 would lose precision when cast to f64, but any such
/// count rendered as "nn.nM" is already imprecise for display. We are
/// deliberately trading precision for readability. See Charter §2.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "display-only; 1-dp K/M formatting accepts f64 rounding. Remove if we start rendering counts >= 2^53 exactly."
)]
pub fn fmt_num(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[must_use]
pub fn thermal_band(temp: f64, target: f64) -> (&'static str, &'static str) {
    let diff = (temp - target).abs();
    if diff <= 0.3 {
        (GRN, "NORMAL")
    } else if diff <= 0.5 {
        (YEL, if temp < target { "COOL" } else { "HOT" })
    } else {
        (RED, "CRITICAL")
    }
}

/// P3: staleness indicator for bridge data.
///
/// Returns `Some("[STALE Xs]")` (yellow-tinted) when `elapsed_since_valid`
/// exceeds `threshold_secs`, otherwise `None`. Modules render the tag in
/// their header line so the operator sees degraded state at a glance rather
/// than a confidently-rendered zeroed struct.
///
/// Threshold policy per plan v3 §P3: callers compute `max(3 × interval_secs,
/// 45.0)` and pass that as `threshold_secs`. See `BridgeClient.last_valid_tick`
/// for the elapsed-time source.
#[must_use]
pub fn stale_tag(elapsed_since_valid: f64, threshold_secs: f64) -> Option<String> {
    if elapsed_since_valid <= threshold_secs {
        return None;
    }
    // Integer seconds for display. Clamp defends against negative inputs
    // and caps width so the tag never blows out a narrow pane.
    let secs = elapsed_since_valid.clamp(0.0, 9999.0);
    // `as u64` here is safe: `secs` is non-negative and bounded at 9999.
    #[allow(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "secs is clamped to [0.0, 9999.0] before cast; no precision concern for display-seconds"
    )]
    let whole = secs as u64;
    Some(format!("{YEL}[STALE {whole}s]{R}"))
}

#[must_use]
pub fn cycle_indicator(phase: &str) -> String {
    use std::fmt::Write as _;
    let phases = ["Recognize", "Act", "Learn", "Predict", "Harden"];
    let idx = phases.iter().position(|p| *p == phase).unwrap_or(0);
    let mut out = String::new();
    for (i, p) in phases.iter().enumerate() {
        let initial = &p[..1];
        if i == idx {
            let _ = write!(out, "{B}{CYN}{initial}{R}");
        } else {
            let _ = write!(out, "{D}{initial}{R}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_shorter_than_max_returns_input_unchanged() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_exact_length_returns_input_unchanged() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
    }

    #[test]
    fn truncate_longer_than_max_returns_prefix() {
        assert_eq!(truncate("abcdefgh", 3), "abc");
    }

    #[test]
    fn truncate_respects_utf8_char_boundaries() {
        // "héllo" — 'é' is 2 bytes. max=2 would split 'é'; must back off to 1.
        let s = "héllo";
        let out = truncate(s, 2);
        // Must be a valid str slice and shorter than or equal to the original.
        assert!(out.is_char_boundary(out.len()));
        assert!(out.len() <= 2);
        assert_eq!(out, "h");
    }

    #[test]
    fn truncate_with_multibyte_only_input_returns_empty_prefix_when_max_splits_char() {
        // 3 multi-byte characters; max=1 can't hold any whole char.
        let s = "日本語";
        let out = truncate(s, 1);
        assert!(out.is_char_boundary(out.len()));
        assert!(out.is_empty());
    }

    #[test]
    fn truncate_empty_string_is_empty_for_any_max() {
        assert_eq!(truncate("", 0), "");
        assert_eq!(truncate("", 99), "");
    }

    #[test]
    fn fmt_num_small_values_use_bare_digits() {
        assert_eq!(fmt_num(0), "0");
        assert_eq!(fmt_num(1), "1");
        assert_eq!(fmt_num(999), "999");
    }

    #[test]
    fn fmt_num_thousands_threshold_is_inclusive_of_1000() {
        // At exactly 1000 we cross the branch — verify.
        assert_eq!(fmt_num(1_000), "1.0K");
        assert_eq!(fmt_num(1_500), "1.5K");
        assert_eq!(fmt_num(999_999), "1000.0K");
    }

    #[test]
    fn fmt_num_millions_threshold_is_inclusive_of_1_000_000() {
        assert_eq!(fmt_num(1_000_000), "1.0M");
        assert_eq!(fmt_num(25_000_000), "25.0M");
    }

    #[test]
    fn thermal_band_near_target_is_green_normal() {
        let (color, label) = thermal_band(0.5, 0.5);
        assert_eq!(color, GRN);
        assert_eq!(label, "NORMAL");
        let (color, label) = thermal_band(0.3, 0.5);
        assert_eq!(color, GRN);
        assert_eq!(label, "NORMAL");
    }

    #[test]
    fn thermal_band_cool_below_target_at_yellow_band() {
        let (color, label) = thermal_band(0.0, 0.5);
        assert_eq!(color, YEL);
        assert_eq!(label, "COOL");
    }

    #[test]
    fn thermal_band_hot_above_target_at_yellow_band() {
        let (color, label) = thermal_band(0.9, 0.5);
        assert_eq!(color, YEL);
        assert_eq!(label, "HOT");
    }

    #[test]
    fn thermal_band_extreme_delta_is_red_critical() {
        let (color, label) = thermal_band(1.5, 0.5);
        assert_eq!(color, RED);
        assert_eq!(label, "CRITICAL");
        let (color, label) = thermal_band(-0.5, 0.5);
        assert_eq!(color, RED);
        assert_eq!(label, "CRITICAL");
    }

    #[test]
    fn thermal_band_inside_green_band_is_green() {
        // 0.29 delta is inside the 0.3 band — GREEN.
        let (color, _) = thermal_band(0.29, 0.0);
        assert_eq!(color, GRN);
    }

    #[test]
    fn thermal_band_inside_yellow_band_is_yellow() {
        // 0.4 delta is inside the 0.3..=0.5 band — YELLOW.
        let (color, _) = thermal_band(0.4, 0.0);
        assert_eq!(color, YEL);
    }

    #[test]
    fn cycle_indicator_highlights_current_phase() {
        let out = cycle_indicator("Learn");
        // The bold+cyan bracket contains 'L'; dim bracket contains the others.
        // Two assertions: L shows bold+cyan, R shows dim.
        assert!(out.contains(&format!("{B}{CYN}L{R}")));
        assert!(out.contains(&format!("{D}R{R}")));
    }

    #[test]
    fn cycle_indicator_unknown_phase_defaults_to_first() {
        // Unknown phase should highlight 'R' (Recognize) per `unwrap_or(0)` fallback.
        let out = cycle_indicator("Nonexistent");
        assert!(out.contains(&format!("{B}{CYN}R{R}")));
    }

    #[test]
    fn render_line_blank_is_empty() {
        assert!(RenderLine::blank().content.is_empty());
    }

    #[test]
    fn render_line_separator_uses_requested_width_of_hlines() {
        let line = RenderLine::separator(10);
        let hline_count = line.content.matches(HLINE).count();
        assert_eq!(hline_count, 10);
    }

    #[test]
    fn render_line_separator_width_zero_is_empty_apart_from_ansi() {
        let line = RenderLine::separator(0);
        assert_eq!(line.content.matches(HLINE).count(), 0);
    }

    // ── P3 stale_tag tests ───────────────────────────────────────────────

    #[test]
    fn stale_tag_below_threshold_returns_none() {
        assert!(stale_tag(0.0, 45.0).is_none());
        assert!(stale_tag(44.9, 45.0).is_none());
    }

    #[test]
    fn stale_tag_at_exact_threshold_returns_none() {
        // Threshold is inclusive of "not stale yet" — exactly 45s is still fresh.
        assert!(stale_tag(45.0, 45.0).is_none());
    }

    #[test]
    fn stale_tag_just_over_threshold_returns_some_with_seconds() {
        let tag = stale_tag(45.1, 45.0).expect("over-threshold returns Some");
        assert!(tag.contains("STALE"));
        assert!(tag.contains("45s"));
    }

    #[test]
    fn stale_tag_uses_yellow_ansi_color_code() {
        let tag = stale_tag(60.0, 45.0).expect("stale");
        assert!(tag.starts_with(YEL), "tag must start with yellow escape");
        assert!(tag.ends_with(R), "tag must end with reset escape");
    }

    #[test]
    fn stale_tag_negative_elapsed_treated_as_fresh() {
        // Defensive guard: a clock-skew race could produce a negative elapsed;
        // below threshold returns None regardless of sign.
        assert!(stale_tag(-5.0, 45.0).is_none());
    }

    #[test]
    fn stale_tag_caps_at_9999_for_extreme_elapsed_without_overflow() {
        // Without the cap, a wildly large elapsed (e.g. from a dormant pane
        // resuming after system sleep) would render a line-breaking tag.
        let tag = stale_tag(1e12, 45.0).expect("stale");
        // Extract the number between "STALE " and "s"
        let start = tag.find("STALE ").expect("STALE prefix") + 6;
        let end = tag[start..].find('s').expect("s suffix");
        let secs_str = &tag[start..start + end];
        let secs: u64 = secs_str.parse().expect("integer seconds");
        assert!(secs <= 9999, "cap enforced; got {secs}");
    }

    #[test]
    fn render_line_new_preserves_content_verbatim() {
        let s = "hello world";
        let line = RenderLine::new(s.to_string());
        assert_eq!(line.content, s);
    }
}
