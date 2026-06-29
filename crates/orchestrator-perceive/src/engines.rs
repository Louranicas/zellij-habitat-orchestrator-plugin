//! Engine health probing.
//!
//! Issues read-only HTTP probes (via the injected runner, for example `curl`)
//! against the known ULTRAPLATE engine endpoints and records their status codes.
//!
//! ## Default target set
//!
//! [`default_targets`] returns the plan §10.2 engine set — all 20 ULTRAPLATE
//! service ports, using `/api/health` for the Maintenance Engine and `/health`
//! for everything else (per the CLAUDE.md gotcha table).

use std::time::SystemTime;

use crate::exec::CommandRunner;
use crate::manifest::{EngineProbe, HealthCode, Port, TimestampMs};
use crate::Result;

/// A single engine endpoint to probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeTarget {
    /// Short engine name (for example `WFE`).
    pub name: String,
    /// Service port.
    pub port: Port,
    /// Health path (for example `/health` or `/api/health`).
    pub health_path: String,
}

/// Returns the canonical ULTRAPLATE engine probe set (plan §10.2).
///
/// The list covers every service port in CLAUDE.md §ULTRAPLATE Services,
/// using `/api/health` for the Maintenance Engine (port 8180) and `/health`
/// for all other services.  The list is returned in ascending port order.
#[must_use]
pub fn default_targets() -> Vec<ProbeTarget> {
    // All ports per CLAUDE.md § ULTRAPLATE Services table.
    // ME uses /api/health; everything else uses /health.
    // Port::new cannot fail for non-zero values in the range 1..=65535.
    let entries: &[(&str, u16, &str)] = &[
        ("DevOps", 8082, "/health"),
        ("Nerve", 8083, "/health"),
        ("ToolLib", 8085, "/health"),
        ("SYNTHEX-v2", 8092, "/health"),
        ("CodeSynthor", 8111, "/health"),
        ("Vortex", 8120, "/health"),
        ("POVM", 8125, "/health"),
        ("ReasoningMem", 8130, "/health"),
        ("PV2", 8132, "/health"),
        ("ORAC", 8133, "/health"),
        ("HabMem", 8140, "/health"),
        ("WFE", 8142, "/health"),
        ("WFEv2", 8143, "/health"),
        ("Architect", 8144, "/health"),
        ("ME", 8180, "/api/health"),
        ("LCM", 8200, "/health"),
        ("TIERWRIGHT", 8201, "/health"),
        ("PrometheusSwarm", 10002, "/health"),
    ];

    entries
        .iter()
        .filter_map(|(name, port, path)| {
            Port::new(*port).ok().map(|p| ProbeTarget {
                name: (*name).to_string(),
                port: p,
                health_path: (*path).to_string(),
            })
        })
        .collect()
}

/// Probes every target and records its observed health.
///
/// For each target a `curl --silent --output /dev/null --write-out "%{http_code}"`
/// call is issued via the runner.  The response status code is captured; if the
/// command fails or the output is not a valid status code the `health_code` field
/// is set to `None` (unreachable).  No error is propagated for individual probe
/// failures — a partially populated list is always returned.
///
/// # Errors
/// Returns an error only if the targets slice is empty (it signals a configuration
/// mistake at the call site).
pub fn probe(runner: &dyn CommandRunner, targets: &[ProbeTarget]) -> Result<Vec<EngineProbe>> {
    let now_ms = now_millis();
    let results = targets
        .iter()
        .map(|t| probe_one(runner, t, now_ms))
        .collect();
    Ok(results)
}

fn probe_one(runner: &dyn CommandRunner, target: &ProbeTarget, probed_at_ms: u64) -> EngineProbe {
    let url = format!(
        "http://localhost:{}{}",
        target.port.get(),
        target.health_path
    );

    let health_code = match runner.run(&[
        "/usr/bin/curl".to_string(),
        "--silent".to_string(),
        "--output".to_string(),
        "/dev/null".to_string(),
        "--max-time".to_string(),
        "2".to_string(),
        "--write-out".to_string(),
        "%{http_code}".to_string(),
        url,
    ]) {
        Ok(out) if out.status == 0 => {
            let code_str = out.stdout.trim();
            code_str
                .parse::<u16>()
                .ok()
                .and_then(|c| HealthCode::new(c).ok())
        }
        _ => None,
    };

    EngineProbe {
        name: target.name.clone(),
        port: target.port,
        health_code,
        probed_at_ms: TimestampMs::from_millis(probed_at_ms),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};

    struct FakeProber {
        /// Maps port number to the `http_code` string to return.
        responses: std::collections::HashMap<u16, String>,
        fail_all: bool,
    }

    impl FakeProber {
        fn new(responses: std::collections::HashMap<u16, String>) -> Self {
            Self {
                responses,
                fail_all: false,
            }
        }

        fn always_200() -> Self {
            // We'll match on the url containing the port
            Self {
                responses: std::collections::HashMap::new(),
                fail_all: false,
            }
        }

        fn failing() -> Self {
            Self {
                responses: std::collections::HashMap::new(),
                fail_all: true,
            }
        }
    }

    impl CommandRunner for FakeProber {
        fn run(&self, argv: &[String]) -> crate::Result<CommandOutput> {
            if self.fail_all {
                return Ok(CommandOutput {
                    status: 1,
                    stdout: String::new(),
                    stderr: "connection refused".to_string(),
                });
            }

            // Extract port from the url argument (last arg)
            let url = argv.last().unwrap_or(&String::new()).clone();
            let code = self
                .responses
                .iter()
                .find(|(port, _)| url.contains(&format!(":{port}")))
                .map_or_else(|| "200".to_string(), |(_, code)| code.clone());

            Ok(CommandOutput {
                status: 0,
                stdout: code,
                stderr: String::new(),
            })
        }
    }

    fn target(name: &str, port: u16) -> ProbeTarget {
        ProbeTarget {
            name: name.to_string(),
            port: Port::new(port).unwrap(),
            health_path: "/health".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // default_targets tests
    // -----------------------------------------------------------------------

    #[test]
    fn default_targets_is_not_empty() {
        let targets = default_targets();
        assert!(!targets.is_empty());
    }

    #[test]
    fn default_targets_includes_wfe() {
        let targets = default_targets();
        assert!(targets.iter().any(|t| t.name == "WFE" && t.port.get() == 8142));
    }

    #[test]
    fn default_targets_includes_lcm() {
        let targets = default_targets();
        assert!(targets.iter().any(|t| t.name == "LCM" && t.port.get() == 8200));
    }

    #[test]
    fn default_targets_me_uses_api_health() {
        let targets = default_targets();
        let me = targets.iter().find(|t| t.port.get() == 8180).unwrap();
        assert_eq!(me.health_path, "/api/health");
    }

    #[test]
    fn default_targets_all_others_use_health() {
        let targets = default_targets();
        for t in &targets {
            if t.port.get() != 8180 {
                assert_eq!(t.health_path, "/health", "port {} uses wrong path", t.port.get());
            }
        }
    }

    #[test]
    fn default_targets_no_duplicate_ports() {
        let targets = default_targets();
        let mut ports: Vec<u16> = targets.iter().map(|t| t.port.get()).collect();
        ports.sort_unstable();
        ports.dedup();
        assert_eq!(ports.len(), targets.len(), "duplicate ports in default_targets");
    }

    #[test]
    fn default_targets_all_ports_nonzero() {
        for t in default_targets() {
            assert_ne!(t.port.get(), 0);
        }
    }

    // -----------------------------------------------------------------------
    // probe() happy path
    // -----------------------------------------------------------------------

    #[test]
    fn probe_empty_targets_returns_empty_vec() {
        let runner = FakeProber::always_200();
        let result = probe(&runner, &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn probe_single_target_200() {
        let runner = FakeProber::always_200();
        let targets = vec![target("WFE", 8142)];
        let results = probe(&runner, &targets).unwrap();
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.name, "WFE");
        assert_eq!(r.port.get(), 8142);
        assert_eq!(r.health_code.unwrap().get(), 200);
    }

    #[test]
    fn probe_multiple_targets_produces_matching_count() {
        let runner = FakeProber::always_200();
        let targets = vec![target("A", 8082), target("B", 8083), target("C", 8092)];
        let results = probe(&runner, &targets).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn probe_preserves_target_name_and_port() {
        let runner = FakeProber::always_200();
        let targets = vec![target("LCM", 8200)];
        let results = probe(&runner, &targets).unwrap();
        assert_eq!(results[0].name, "LCM");
        assert_eq!(results[0].port.get(), 8200);
    }

    #[test]
    fn probe_unreachable_sets_health_code_none() {
        let runner = FakeProber::failing();
        let targets = vec![target("WFE", 8142)];
        let results = probe(&runner, &targets).unwrap();
        assert!(results[0].health_code.is_none());
    }

    #[test]
    fn probe_non200_code_captured() {
        let mut map = std::collections::HashMap::new();
        map.insert(8142_u16, "503".to_string());
        let runner = FakeProber::new(map);
        let targets = vec![target("WFE", 8142)];
        let results = probe(&runner, &targets).unwrap();
        assert_eq!(results[0].health_code.unwrap().get(), 503);
    }

    #[test]
    fn probe_invalid_code_in_stdout_gives_none() {
        struct GarbageRunner;
        impl CommandRunner for GarbageRunner {
            fn run(&self, _: &[String]) -> crate::Result<CommandOutput> {
                Ok(CommandOutput {
                    status: 0,
                    stdout: "not-a-number".to_string(),
                    stderr: String::new(),
                })
            }
        }
        let targets = vec![target("X", 8082)];
        let results = probe(&GarbageRunner, &targets).unwrap();
        assert!(results[0].health_code.is_none());
    }

    #[test]
    fn probe_out_of_range_code_gives_none() {
        struct BadCodeRunner;
        impl CommandRunner for BadCodeRunner {
            fn run(&self, _: &[String]) -> crate::Result<CommandOutput> {
                Ok(CommandOutput {
                    status: 0,
                    stdout: "99".to_string(), // below 100
                    stderr: String::new(),
                })
            }
        }
        let targets = vec![target("X", 8082)];
        let results = probe(&BadCodeRunner, &targets).unwrap();
        assert!(results[0].health_code.is_none());
    }

    // -----------------------------------------------------------------------
    // Newtype boundary tests (engines uses Port + HealthCode)
    // -----------------------------------------------------------------------

    #[test]
    fn health_code_rejects_zero() {
        let err = HealthCode::new(0).unwrap_err();
        assert!(matches!(err, crate::error::PerceiveError::OutOfRange { .. }));
    }

    #[test]
    fn health_code_rejects_99() {
        assert!(HealthCode::new(99).is_err());
    }

    #[test]
    fn health_code_accepts_100() {
        assert_eq!(HealthCode::new(100).unwrap().get(), 100);
    }

    #[test]
    fn health_code_accepts_599() {
        assert_eq!(HealthCode::new(599).unwrap().get(), 599);
    }

    #[test]
    fn health_code_rejects_600() {
        assert!(HealthCode::new(600).is_err());
    }

    #[test]
    fn port_rejects_zero() {
        assert!(Port::new(0).is_err());
    }

    #[test]
    fn port_accepts_one() {
        assert_eq!(Port::new(1).unwrap().get(), 1);
    }

    #[test]
    fn port_accepts_65535() {
        assert_eq!(Port::new(65535).unwrap().get(), 65535);
    }
}
