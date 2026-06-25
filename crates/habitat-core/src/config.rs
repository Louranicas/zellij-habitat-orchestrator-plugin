use std::collections::BTreeMap;

/// Non-fatal configuration warning. WASM plugins log these at init without
/// crashing the dashboard. Each variant carries the offending field name
/// and the invalid raw value (if recoverable for logging).
///
/// See Coding Excellence Charter §2 and Hardening Plan v3 §WS-0 P2.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ConfigWarning {
    /// URL was empty, missing scheme, or not `http`/`https`. Fell back to default.
    InvalidUrl {
        field: &'static str,
        raw: String,
        fell_back_to: String,
    },
    /// Poll interval was outside `[1.0, 300.0]`. Clamped to boundary.
    PollIntervalClamped {
        field: &'static str,
        requested: f64,
        clamped_to: f64,
    },
    /// Poll interval failed to parse as a number. Fell back to default.
    PollIntervalNotNumeric {
        field: &'static str,
        raw: String,
        fell_back_to: f64,
    },
}

/// Minimum permitted poll interval in seconds. Below this, polling would
/// saturate the curl subprocess budget in habitat-wire / habitat-zellij.
pub const POLL_INTERVAL_MIN_SECS: f64 = 1.0;

/// Maximum permitted poll interval in seconds. Above this, data goes stale
/// faster than any `[STALE Xs]` indicator would surface (Hardening Plan §P3).
pub const POLL_INTERVAL_MAX_SECS: f64 = 300.0;

#[derive(Clone, Debug)]
pub struct ModuleConfig {
    pub orac_url: String,
    pub pv2_url: String,
    pub synthex_url: String,
    pub nerve_url: String,
    pub coherence_poll: f64,
    pub health_poll: f64,
    pub governance_poll: f64,
    pub kernel_poll: f64,
    pub sidecar_cli: String,
    pub layout_mode: LayoutMode,
    pub show_consent_states: bool,
    pub show_attribution: bool,
}

#[derive(Clone, Debug, Default)]
pub enum LayoutMode {
    #[default]
    Full,
    Compact,
    Minimal,
}

impl ModuleConfig {
    /// Parse module config from a flat key/value map (Zellij's plugin-config format).
    ///
    /// Returns the populated `ModuleConfig` plus a vector of `ConfigWarning`s for any
    /// fields that failed validation (empty URL, invalid scheme, out-of-range poll
    /// interval, non-numeric poll interval). A non-empty warning vector does NOT mean
    /// parsing failed — every warning corresponds to a recovered fallback so the plugin
    /// can always boot. Callers SHOULD log warnings at startup.
    ///
    /// Hardening Plan v3 §WS-0 P2 — consent-sovereignty constraint: config failure must
    /// never crash the plugin (WASM unrecoverable), only degrade to documented defaults.
    #[must_use]
    pub fn from_btree(config: &BTreeMap<String, String>) -> (Self, Vec<ConfigWarning>) {
        let mut warnings = Vec::new();

        let orac_url = validated_url(
            config.get("orac_url"),
            "orac_url",
            "http://127.0.0.1:8133",
            &mut warnings,
        );
        let pv2_url = validated_url(
            config.get("pv2_url"),
            "pv2_url",
            "http://127.0.0.1:8132",
            &mut warnings,
        );
        let synthex_url = validated_url(
            config.get("synthex_url"),
            "synthex_url",
            "http://127.0.0.1:8090",
            &mut warnings,
        );
        let nerve_url = validated_url(
            config.get("nerve_url"),
            "nerve_url",
            "http://127.0.0.1:8083",
            &mut warnings,
        );

        let coherence_poll = validated_poll(
            config.get("coherence_poll"),
            "coherence_poll",
            2.0,
            &mut warnings,
        );
        let health_poll =
            validated_poll(config.get("health_poll"), "health_poll", 5.0, &mut warnings);
        let governance_poll = validated_poll(
            config.get("governance_poll"),
            "governance_poll",
            10.0,
            &mut warnings,
        );
        let kernel_poll =
            validated_poll(config.get("kernel_poll"), "kernel_poll", 5.0, &mut warnings);
        let sidecar_cli = config
            .get("sidecar_cli")
            .cloned()
            .unwrap_or_else(|| "/home/louranicas/.local/bin/orch-kernelctl".into());

        let layout_mode = match config.get("layout_mode").map(String::as_str) {
            Some("compact") => LayoutMode::Compact,
            Some("minimal") => LayoutMode::Minimal,
            _ => LayoutMode::Full,
        };

        let show_consent_states = config
            .get("show_consent_states")
            .is_none_or(|v| v == "true");
        let show_attribution = config.get("show_attribution").is_none_or(|v| v == "true");

        (
            Self {
                orac_url,
                pv2_url,
                synthex_url,
                nerve_url,
                coherence_poll,
                health_poll,
                governance_poll,
                kernel_poll,
                sidecar_cli,
                layout_mode,
                show_consent_states,
                show_attribution,
            },
            warnings,
        )
    }
}

/// Validate a URL: must be non-empty and start with `http://` or `https://`.
/// Returns the validated URL or the fallback, pushing a `ConfigWarning` on failure.
fn validated_url(
    raw: Option<&String>,
    field: &'static str,
    fallback: &str,
    warnings: &mut Vec<ConfigWarning>,
) -> String {
    match raw {
        Some(v) if v.is_empty() => {
            warnings.push(ConfigWarning::InvalidUrl {
                field,
                raw: String::new(),
                fell_back_to: fallback.into(),
            });
            fallback.into()
        }
        Some(v) if !v.starts_with("http://") && !v.starts_with("https://") => {
            warnings.push(ConfigWarning::InvalidUrl {
                field,
                raw: v.clone(),
                fell_back_to: fallback.into(),
            });
            fallback.into()
        }
        Some(v) => v.clone(),
        None => fallback.into(),
    }
}

/// Validate + clamp a poll interval. Returns a finite value in `[POLL_INTERVAL_MIN_SECS,
/// POLL_INTERVAL_MAX_SECS]`. Pushes a `ConfigWarning` on parse failure or clamp.
fn validated_poll(
    raw: Option<&String>,
    field: &'static str,
    fallback: f64,
    warnings: &mut Vec<ConfigWarning>,
) -> f64 {
    let Some(raw_str) = raw else {
        return fallback;
    };
    let Ok(parsed) = raw_str.parse::<f64>() else {
        warnings.push(ConfigWarning::PollIntervalNotNumeric {
            field,
            raw: raw_str.clone(),
            fell_back_to: fallback,
        });
        return fallback;
    };
    if !parsed.is_finite() {
        warnings.push(ConfigWarning::PollIntervalNotNumeric {
            field,
            raw: raw_str.clone(),
            fell_back_to: fallback,
        });
        return fallback;
    }
    let clamped = parsed.clamp(POLL_INTERVAL_MIN_SECS, POLL_INTERVAL_MAX_SECS);
    if (clamped - parsed).abs() > f64::EPSILON {
        warnings.push(ConfigWarning::PollIntervalClamped {
            field,
            requested: parsed,
            clamped_to: clamped,
        });
    }
    clamped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn defaults_point_to_canonical_habitat_localhost_ports() {
        let (c, warnings) = ModuleConfig::from_btree(&empty());
        assert_eq!(c.orac_url, "http://127.0.0.1:8133");
        assert_eq!(c.pv2_url, "http://127.0.0.1:8132");
        assert_eq!(c.synthex_url, "http://127.0.0.1:8090");
        assert_eq!(c.nerve_url, "http://127.0.0.1:8083");
        assert!(warnings.is_empty(), "no warnings for defaults");
    }

    #[test]
    fn default_poll_intervals_match_module_budgets() {
        let (c, warnings) = ModuleConfig::from_btree(&empty());
        assert!((c.coherence_poll - 2.0).abs() < f64::EPSILON);
        assert!((c.health_poll - 5.0).abs() < f64::EPSILON);
        assert!((c.governance_poll - 10.0).abs() < f64::EPSILON);
        assert!((c.kernel_poll - 5.0).abs() < f64::EPSILON);
        assert!(warnings.is_empty());
    }

    #[test]
    fn default_layout_mode_is_full() {
        let (c, _) = ModuleConfig::from_btree(&empty());
        assert!(matches!(c.layout_mode, LayoutMode::Full));
    }

    #[test]
    fn layout_mode_compact_parses_from_string() {
        let mut m = empty();
        m.insert("layout_mode".into(), "compact".into());
        let (c, _) = ModuleConfig::from_btree(&m);
        assert!(matches!(c.layout_mode, LayoutMode::Compact));
    }

    #[test]
    fn layout_mode_minimal_parses_from_string() {
        let mut m = empty();
        m.insert("layout_mode".into(), "minimal".into());
        let (c, _) = ModuleConfig::from_btree(&m);
        assert!(matches!(c.layout_mode, LayoutMode::Minimal));
    }

    #[test]
    fn unknown_layout_mode_falls_back_to_full() {
        let mut m = empty();
        m.insert("layout_mode".into(), "wombat".into());
        let (c, _) = ModuleConfig::from_btree(&m);
        assert!(matches!(c.layout_mode, LayoutMode::Full));
    }

    #[test]
    fn malformed_poll_interval_falls_back_to_default_with_warning() {
        let mut m = empty();
        m.insert("coherence_poll".into(), "not-a-number".into());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert!((c.coherence_poll - 2.0).abs() < f64::EPSILON);
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0],
            ConfigWarning::PollIntervalNotNumeric {
                field: "coherence_poll",
                ..
            }
        ));
    }

    #[test]
    fn show_consent_states_false_parses_to_false() {
        let mut m = empty();
        m.insert("show_consent_states".into(), "false".into());
        let (c, _) = ModuleConfig::from_btree(&m);
        assert!(!c.show_consent_states);
    }

    #[test]
    fn show_attribution_default_is_true_preserving_na_contract() {
        let (c, _) = ModuleConfig::from_btree(&empty());
        assert!(c.show_attribution);
    }

    // P2 validation tests ──────────────────────────────────────────────

    #[test]
    fn empty_url_falls_back_and_emits_invalid_url_warning() {
        let mut m = empty();
        m.insert("orac_url".into(), String::new());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert_eq!(c.orac_url, "http://127.0.0.1:8133");
        assert!(matches!(
            warnings[0],
            ConfigWarning::InvalidUrl {
                field: "orac_url",
                ..
            }
        ));
    }

    #[test]
    fn non_http_url_scheme_falls_back_and_emits_invalid_url_warning() {
        let mut m = empty();
        m.insert("pv2_url".into(), "ftp://127.0.0.1:8132".into());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert_eq!(c.pv2_url, "http://127.0.0.1:8132");
        assert!(matches!(
            warnings[0],
            ConfigWarning::InvalidUrl {
                field: "pv2_url",
                ..
            }
        ));
    }

    #[test]
    fn https_url_scheme_is_accepted_without_warning() {
        let mut m = empty();
        m.insert(
            "synthex_url".into(),
            "https://synthex.example.com:8090".into(),
        );
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert_eq!(c.synthex_url, "https://synthex.example.com:8090");
        assert!(warnings.is_empty());
    }

    #[test]
    fn poll_interval_zero_clamps_up_to_min_with_warning() {
        let mut m = empty();
        m.insert("coherence_poll".into(), "0.0".into());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert!((c.coherence_poll - POLL_INTERVAL_MIN_SECS).abs() < f64::EPSILON);
        assert!(matches!(
            warnings[0],
            ConfigWarning::PollIntervalClamped {
                field: "coherence_poll",
                clamped_to,
                ..
            } if (clamped_to - POLL_INTERVAL_MIN_SECS).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn poll_interval_above_max_clamps_down_with_warning() {
        let mut m = empty();
        m.insert("health_poll".into(), "999.0".into());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert!((c.health_poll - POLL_INTERVAL_MAX_SECS).abs() < f64::EPSILON);
        assert!(matches!(
            warnings[0],
            ConfigWarning::PollIntervalClamped {
                field: "health_poll",
                clamped_to,
                ..
            } if (clamped_to - POLL_INTERVAL_MAX_SECS).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn negative_poll_interval_clamps_to_min_with_warning() {
        let mut m = empty();
        m.insert("governance_poll".into(), "-5.0".into());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert!((c.governance_poll - POLL_INTERVAL_MIN_SECS).abs() < f64::EPSILON);
        assert!(matches!(
            warnings[0],
            ConfigWarning::PollIntervalClamped {
                field: "governance_poll",
                ..
            }
        ));
    }

    #[test]
    fn valid_in_range_poll_interval_passes_through_without_warning() {
        let mut m = empty();
        m.insert("coherence_poll".into(), "2.5".into());
        m.insert("health_poll".into(), "7.5".into());
        m.insert("governance_poll".into(), "15.0".into());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert!((c.coherence_poll - 2.5).abs() < f64::EPSILON);
        assert!((c.health_poll - 7.5).abs() < f64::EPSILON);
        assert!((c.governance_poll - 15.0).abs() < f64::EPSILON);
        assert!(warnings.is_empty());
    }

    #[test]
    fn multiple_invalid_fields_accumulate_warnings_independently() {
        let mut m = empty();
        m.insert("orac_url".into(), String::new()); // invalid
        m.insert("pv2_url".into(), "tcp://x".into()); // invalid
        m.insert("coherence_poll".into(), "-1.0".into()); // clamp
        m.insert("health_poll".into(), "1000.0".into()); // clamp
        m.insert("governance_poll".into(), "bogus".into()); // not numeric
        let (_c, warnings) = ModuleConfig::from_btree(&m);
        assert_eq!(warnings.len(), 5, "all 5 issues surface independently");
    }

    #[test]
    fn nan_poll_interval_is_rejected_not_silently_passed() {
        let mut m = empty();
        m.insert("coherence_poll".into(), "NaN".into());
        let (c, warnings) = ModuleConfig::from_btree(&m);
        assert!((c.coherence_poll - 2.0).abs() < f64::EPSILON);
        assert!(matches!(
            warnings[0],
            ConfigWarning::PollIntervalNotNumeric {
                field: "coherence_poll",
                ..
            }
        ));
    }
}
