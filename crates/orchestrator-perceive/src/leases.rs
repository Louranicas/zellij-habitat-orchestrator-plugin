//! Lease observation.
//!
//! Reads `kv-lease status` for a set of known resources and records owner,
//! expiry, and the monotonic fence. The fence is minted elsewhere (by `kv-lease`
//! and `dcg-admit` in P2); this module is strictly read-only with respect to it.
//!
//! ## Output format
//!
//! `kv-lease status <resource>` prints a JSON object on stdout:
//! ```json
//! {"owner":"…","nonce":"…","claimed_at":1234,"expires":1299,"note":"…","fence":42}
//! ```
//! Fields `note` and `fence` may be absent (pre-P2 leases); missing `fence` is
//! reported as `0`.  A non-zero exit status or missing `owner` means the resource
//! is not currently leased — it is omitted from the result rather than reported
//! as an error.

use crate::exec::CommandRunner;
use crate::manifest::{LeaseObservation, TimestampMs};
use crate::Result;

/// Path to the `kv-lease` binary (absolute, per D11 rule).
const KV_LEASE_PATH: &str = "/home/louranicas/.local/bin/kv-lease";

/// Reads the lease state for each named resource.
///
/// Resources that are not currently leased (non-zero exit from `kv-lease status`
/// or a missing `owner` field) are silently omitted — an empty slice is a valid
/// result when no leases are held.
///
/// # Errors
/// This function never propagates individual probe errors; it always returns
/// `Ok(Vec<…>)` even if every probe fails, so the assembler can still produce a
/// partial snapshot from the other sources.
pub fn observe(runner: &dyn CommandRunner, resources: &[String]) -> Result<Vec<LeaseObservation>> {
    let observations = resources
        .iter()
        .filter_map(|resource| probe_lease(runner, resource))
        .collect();
    Ok(observations)
}

fn probe_lease(runner: &dyn CommandRunner, resource: &str) -> Option<LeaseObservation> {
    let out = runner
        .run(&[
            KV_LEASE_PATH.to_string(),
            "status".to_string(),
            resource.to_string(),
            "--json".to_string(),
        ])
        .ok()?;

    if out.status != 0 || out.stdout.trim().is_empty() {
        return None;
    }

    parse_lease_json(resource, &out.stdout)
}

fn parse_lease_json(resource: &str, raw: &str) -> Option<LeaseObservation> {
    let v: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;

    let owner = v.get("owner")?.as_str()?.to_string();
    if owner.is_empty() {
        return None;
    }

    let expires_ms = v
        .get("expires")
        .and_then(serde_json::Value::as_u64)
        .map_or_else(|| TimestampMs::from_millis(0), TimestampMs::from_millis);

    let fence = v
        .get("fence")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    Some(LeaseObservation {
        resource: resource.to_string(),
        owner,
        expires_ms,
        fence,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::exec::{CommandOutput, CommandRunner};

    struct FakeLeaser {
        responses: std::collections::HashMap<String, (i32, String)>,
    }

    impl FakeLeaser {
        fn new() -> Self {
            Self {
                responses: std::collections::HashMap::new(),
            }
        }

        fn with(mut self, resource: &str, status: i32, stdout: &str) -> Self {
            self.responses
                .insert(resource.to_string(), (status, stdout.to_string()));
            self
        }
    }

    impl CommandRunner for FakeLeaser {
        fn run(&self, argv: &[String]) -> crate::Result<CommandOutput> {
            // Extract resource from argv: [kv-lease, status, <resource>]
            let resource = argv.get(2).cloned().unwrap_or_default();
            let (status, stdout) = self
                .responses
                .get(&resource)
                .cloned()
                .unwrap_or((1, String::new()));
            Ok(CommandOutput {
                status,
                stdout,
                stderr: String::new(),
            })
        }
    }

    fn lease_json(owner: &str, expires: u64, fence: u64) -> String {
        format!(
            r#"{{"owner":"{owner}","nonce":"abc","claimed_at":1000,"expires":{expires},"note":"","fence":{fence}}}"#
        )
    }

    // -----------------------------------------------------------------------
    // parse_lease_json tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_lease_json_happy_path() {
        let raw = lease_json("user-a", 9999, 5);
        let obs = parse_lease_json("factory.some-resource", &raw).unwrap();
        assert_eq!(obs.resource, "factory.some-resource");
        assert_eq!(obs.owner, "user-a");
        assert_eq!(obs.expires_ms.get(), 9999);
        assert_eq!(obs.fence, 5);
    }

    #[test]
    fn parse_lease_json_missing_fence_defaults_to_zero() {
        let raw = r#"{"owner":"user-b","nonce":"n","claimed_at":100,"expires":200,"note":""}"#;
        let obs = parse_lease_json("res", raw).unwrap();
        assert_eq!(obs.fence, 0);
    }

    #[test]
    fn parse_lease_json_empty_owner_returns_none() {
        let raw = r#"{"owner":"","nonce":"n","claimed_at":100,"expires":200,"note":"","fence":1}"#;
        assert!(parse_lease_json("res", raw).is_none());
    }

    #[test]
    fn parse_lease_json_missing_owner_returns_none() {
        let raw = r#"{"nonce":"n","claimed_at":100,"expires":200,"note":""}"#;
        assert!(parse_lease_json("res", raw).is_none());
    }

    #[test]
    fn parse_lease_json_invalid_json_returns_none() {
        assert!(parse_lease_json("res", "not json").is_none());
    }

    #[test]
    fn parse_lease_json_missing_expires_defaults_to_zero() {
        let raw = r#"{"owner":"x","nonce":"n","claimed_at":0,"note":"","fence":0}"#;
        let obs = parse_lease_json("res", raw).unwrap();
        assert_eq!(obs.expires_ms.get(), 0);
    }

    // -----------------------------------------------------------------------
    // observe() tests
    // -----------------------------------------------------------------------

    #[test]
    fn observe_empty_resources_returns_empty() {
        let runner = FakeLeaser::new();
        let result = observe(&runner, &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn observe_held_lease_is_included() {
        let runner = FakeLeaser::new().with(
            "factory.some-campaign",
            0,
            &lease_json("fiber-1", 9999, 3),
        );
        let result = observe(
            &runner,
            &["factory.some-campaign".to_string()],
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].owner, "fiber-1");
        assert_eq!(result[0].fence, 3);
    }

    #[test]
    fn observe_unlocked_resource_is_omitted() {
        let runner = FakeLeaser::new().with("factory.free-resource", 1, "");
        let result = observe(
            &runner,
            &["factory.free-resource".to_string()],
        )
        .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn observe_mixed_held_and_free() {
        let runner = FakeLeaser::new()
            .with("factory.held", 0, &lease_json("owner-1", 1234, 1))
            .with("factory.free", 1, "");
        let result = observe(
            &runner,
            &[
                "factory.held".to_string(),
                "factory.free".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].resource, "factory.held");
    }

    #[test]
    fn observe_multiple_held_leases() {
        let runner = FakeLeaser::new()
            .with("res-a", 0, &lease_json("owner-a", 100, 1))
            .with("res-b", 0, &lease_json("owner-b", 200, 2))
            .with("res-c", 0, &lease_json("owner-c", 300, 3));
        let result = observe(
            &runner,
            &[
                "res-a".to_string(),
                "res-b".to_string(),
                "res-c".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn observe_command_failure_omits_resource() {
        struct AlwaysFails;
        impl CommandRunner for AlwaysFails {
            fn run(&self, _: &[String]) -> crate::Result<CommandOutput> {
                Err(crate::error::PerceiveError::Subprocess {
                    command: "kv-lease".to_string(),
                    detail: "not found".to_string(),
                })
            }
        }
        let result = observe(&AlwaysFails, &["res".to_string()]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn observe_resource_name_preserved_in_output() {
        let runner =
            FakeLeaser::new().with("my.special.resource", 0, &lease_json("u", 0, 0));
        let result = observe(&runner, &["my.special.resource".to_string()]).unwrap();
        assert_eq!(result[0].resource, "my.special.resource");
    }

    #[test]
    fn observe_fence_value_captured() {
        let runner = FakeLeaser::new()
            .with("r", 0, &lease_json("owner", 1000, 42));
        let result = observe(&runner, &["r".to_string()]).unwrap();
        assert_eq!(result[0].fence, 42);
    }

    #[test]
    fn observe_expires_value_captured_in_ms() {
        let runner =
            FakeLeaser::new().with("r", 0, &lease_json("owner", 1_700_000_000_000_u64, 0));
        let result = observe(&runner, &["r".to_string()]).unwrap();
        assert_eq!(result[0].expires_ms.get(), 1_700_000_000_000_u64);
    }
}
