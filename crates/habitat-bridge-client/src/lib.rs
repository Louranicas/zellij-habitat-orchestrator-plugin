use habitat_core::events::HabitatEvent;
use habitat_core::module::{CommandSource, DataSource};
use std::collections::BTreeMap;

/// Callback invoked by `BridgeClient::poll_due` for each ready endpoint.
/// Shape mirrors zellij-tile's `run_command(argv, context)`; the production
/// wiring passes a closure that forwards into the plugin runtime, while tests
/// substitute a capturing closure to assert scheduling behaviour.
pub type RunCommandFn<'a> = dyn Fn(&[&str], BTreeMap<String, String>) + 'a;

pub struct ScheduledEndpoint {
    /// The exact argv the host execs. For a [`DataSource`] this is the curl
    /// envelope; for a [`CommandSource`] it is the raw local-command argv.
    pub argv: Vec<String>,
    pub interval_secs: f64,
    pub tag: String,
    pub module_id: String,
    pub last_poll: f64,
}

pub struct BridgeClient {
    endpoints: Vec<ScheduledEndpoint>,
    elapsed: f64,
    stagger_idx: usize,
    stagger_complete: bool,
}

impl BridgeClient {
    #[must_use]
    pub fn new() -> Self {
        Self {
            endpoints: Vec::new(),
            elapsed: 0.0,
            stagger_idx: 0,
            stagger_complete: false,
        }
    }

    pub fn register_sources(&mut self, sources: Vec<DataSource>) {
        for src in sources {
            // DataSource is an HTTP poll — wrap the URL in the pinned curl envelope.
            self.endpoints.push(ScheduledEndpoint {
                argv: vec![
                    "curl".into(),
                    "-s".into(),
                    "--max-time".into(),
                    "2".into(),
                    "--connect-timeout".into(),
                    "1".into(),
                    src.url,
                ],
                interval_secs: src.interval_secs,
                tag: src.tag,
                module_id: src.module_id,
                last_poll: 0.0,
            });
        }
    }

    /// Register local-command sources (raw argv, no curl wrapper). They share the
    /// same stagger + interval scheduler and route results through `handle_result`
    /// exactly like [`Self::register_sources`]. `argv[0]` must be an absolute path.
    pub fn register_command_sources(&mut self, sources: Vec<CommandSource>) {
        for src in sources {
            self.endpoints.push(ScheduledEndpoint {
                argv: src.argv,
                interval_secs: src.interval_secs,
                tag: src.tag,
                module_id: src.module_id,
                last_poll: 0.0,
            });
        }
    }

    pub fn poll_due(&mut self, poll_secs: f64, run_command_fn: &RunCommandFn<'_>) {
        self.elapsed += poll_secs;

        if !self.stagger_complete {
            if self.stagger_idx < self.endpoints.len() {
                let ep = &mut self.endpoints[self.stagger_idx];
                ep.last_poll = self.elapsed;
                let ctx = BTreeMap::from([
                    ("t".into(), ep.tag.clone()),
                    ("mod".into(), ep.module_id.clone()),
                ]);
                let argv: Vec<&str> = ep.argv.iter().map(String::as_str).collect();
                (run_command_fn)(&argv, ctx);
                self.stagger_idx += 1;
            }
            if self.stagger_idx >= self.endpoints.len() {
                self.stagger_complete = true;
            }
            return;
        }

        for ep in &mut self.endpoints {
            if self.elapsed - ep.last_poll >= ep.interval_secs {
                ep.last_poll = self.elapsed;
                let ctx = BTreeMap::from([
                    ("t".into(), ep.tag.clone()),
                    ("mod".into(), ep.module_id.clone()),
                ]);
                let argv: Vec<&str> = ep.argv.iter().map(String::as_str).collect();
                (run_command_fn)(&argv, ctx);
            }
        }
    }

    #[must_use]
    pub fn handle_result(
        &self,
        exit_code: Option<i32>,
        stdout: &[u8],
        context: &BTreeMap<String, String>,
    ) -> Option<HabitatEvent> {
        let tag = context.get("t")?.clone();
        let module_id = context.get("mod")?.clone();

        if exit_code != Some(0) || stdout.is_empty() {
            return Some(HabitatEvent::BridgeError { module_id, tag });
        }

        let out = String::from_utf8_lossy(stdout);
        if let Ok(data) = serde_json::from_str(&out) {
            Some(HabitatEvent::BridgeData {
                module_id,
                tag,
                data,
            })
        } else {
            // Plain-text response (e.g. Nerve returns "ok"). Still a successful
            // response from the server — wrap as a String Value so health probes
            // see it as "up".
            let text = out.trim().to_string();
            Some(HabitatEvent::BridgeData {
                module_id,
                tag,
                data: serde_json::Value::String(text),
            })
        }
    }
}

impl Default for BridgeClient {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────────────────────────────────────────
// SnapshotClient — shared aggregate consumer
// ──────────────────────────────────────────────────────────────

/// Fans out a single ORAC `/snapshot` response into the same module-tagged
/// `BridgeData` events that `BridgeClient::handle_result` produces. Lets the
/// plugin trade N parallel curls per cycle for one curl + local routing.
///
/// Expected shape of the JSON body:
/// ```json
/// {
///   "orac": { ... },
///   "pv2": { ... },
///   "thermal": { ... },
///   "services": [{"port": 8082, "up": true}, ...]
/// }
/// ```
///
/// Only sub-objects the caller registers a route for are emitted — an aggregate
/// with a `null` `thermal` sub-object simply skips the thermal module's event.
/// That lets the aggregate degrade gracefully when a downstream service is
/// unreachable without the plugin going blind.
pub struct SnapshotClient {
    /// URL of the aggregate endpoint, e.g. `http://127.0.0.1:8133/snapshot`.
    pub url: String,
    /// `(sub_object_key, module_id, tag)` — the keys to pluck out of the
    /// aggregate body and the `BridgeData` tag/module they should arrive as.
    routes: Vec<SnapshotRoute>,
}

/// One fan-out rule: pluck `subkey` from the snapshot body and deliver it as a
/// `BridgeData { module_id, tag }` event.
#[derive(Clone, Debug)]
pub struct SnapshotRoute {
    pub subkey: &'static str,
    pub module_id: String,
    pub tag: String,
}

impl SnapshotClient {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            routes: Vec::new(),
        }
    }

    /// Register one sub-object route. Chainable.
    #[must_use]
    pub fn route(
        mut self,
        subkey: &'static str,
        module_id: impl Into<String>,
        tag: impl Into<String>,
    ) -> Self {
        self.routes.push(SnapshotRoute {
            subkey,
            module_id: module_id.into(),
            tag: tag.into(),
        });
        self
    }

    /// Fan out a received snapshot body into one `HabitatEvent` per registered
    /// route whose subkey is present and non-null.
    ///
    /// Returns the fanned-out events. Callers emit them into the plugin's
    /// event loop exactly as they would a real bridge response.
    #[must_use]
    pub fn fanout(&self, body: &serde_json::Value) -> Vec<HabitatEvent> {
        self.routes
            .iter()
            .filter_map(|r| {
                let data = body.get(r.subkey)?;
                if data.is_null() {
                    return None;
                }
                Some(HabitatEvent::BridgeData {
                    module_id: r.module_id.clone(),
                    tag: r.tag.clone(),
                    data: data.clone(),
                })
            })
            .collect()
    }

    /// Build the curl argv the plugin passes to `run_command`. Matches the
    /// same envelope `BridgeClient::poll_due` uses so ORAC logs are consistent.
    #[must_use]
    pub fn argv(&self) -> Vec<&str> {
        vec![
            "curl",
            "-s",
            "--max-time",
            "2",
            "--connect-timeout",
            "1",
            &self.url,
        ]
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use serde_json::json;

    fn body() -> serde_json::Value {
        json!({
            "orac": {"ralph_gen": 26068, "field_r": 0.0},
            "pv2": {"r": 0.95, "K": 2.5},
            "thermal": {"temperature": 0.273, "target": 0.5},
            "services": [{"port": 8082, "up": true}],
            "nerve": null
        })
    }

    #[test]
    fn fanout_emits_one_event_per_registered_subkey() {
        let client = SnapshotClient::new("http://127.0.0.1:8133/snapshot")
            .route("orac", "fleet_view", "orac_health")
            .route("pv2", "coherence_gauge", "pv2_field")
            .route("thermal", "bridge_health", "synthex_thermal");
        let events = client.fanout(&body());
        assert_eq!(events.len(), 3);
        // Events carry the configured (module_id, tag) pair unchanged.
        let tags: Vec<&str> = events
            .iter()
            .map(|e| match e {
                HabitatEvent::BridgeData { tag, .. } => tag.as_str(),
                _ => "",
            })
            .collect();
        assert!(tags.contains(&"orac_health"));
        assert!(tags.contains(&"pv2_field"));
        assert!(tags.contains(&"synthex_thermal"));
    }

    #[test]
    fn fanout_skips_null_subobjects() {
        // `nerve` is null in the fixture — registering a route for it should
        // drop the event rather than deliver an empty-object event that the
        // downstream module would silently-default.
        let client = SnapshotClient::new("http://any")
            .route("nerve", "bridge_health", "nerve_health")
            .route("orac", "fleet_view", "orac_health");
        let events = client.fanout(&body());
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn fanout_skips_missing_subkeys() {
        let client = SnapshotClient::new("http://any").route("nonexistent_subkey", "x", "x");
        assert!(client.fanout(&body()).is_empty());
    }

    #[test]
    fn fanout_preserves_data_verbatim() {
        let client = SnapshotClient::new("http://any").route("pv2", "coherence_gauge", "pv2_field");
        let events = client.fanout(&body());
        match &events[0] {
            HabitatEvent::BridgeData { data, .. } => {
                assert!((data["r"].as_f64().unwrap() - 0.95).abs() < 1e-9);
                assert!((data["K"].as_f64().unwrap() - 2.5).abs() < 1e-9);
            }
            _ => panic!("expected BridgeData"),
        }
    }

    #[test]
    fn argv_matches_bridge_client_envelope() {
        let client = SnapshotClient::new("http://x/snapshot");
        let argv = client.argv();
        assert_eq!(argv[0], "curl");
        assert_eq!(argv[1], "-s");
        assert!(argv.contains(&"--max-time"));
        assert!(argv.contains(&"--connect-timeout"));
        assert_eq!(argv.last().copied(), Some("http://x/snapshot"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn ctx(tag: &str, module_id: &str) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("t".into(), tag.into());
        m.insert("mod".into(), module_id.into());
        m
    }

    fn src(url: &str, interval: f64, tag: &str, module_id: &str) -> DataSource {
        DataSource {
            url: url.into(),
            interval_secs: interval,
            tag: tag.into(),
            module_id: module_id.into(),
        }
    }

    // --- handle_result ---

    #[test]
    fn handle_result_nonzero_exit_yields_bridge_error() {
        let b = BridgeClient::new();
        let ev = b.handle_result(Some(7), b"body", &ctx("orac_health", "fleet_view"));
        match ev {
            Some(HabitatEvent::BridgeError { module_id, tag }) => {
                assert_eq!(module_id, "fleet_view");
                assert_eq!(tag, "orac_health");
            }
            other => panic!("expected BridgeError, got {other:?}"),
        }
    }

    #[test]
    fn handle_result_empty_stdout_yields_bridge_error_even_on_exit_zero() {
        // curl returning 0 but empty body (e.g. connection drop) is an error.
        let b = BridgeClient::new();
        let ev = b.handle_result(Some(0), b"", &ctx("pv2_field", "coherence_gauge"));
        assert!(matches!(ev, Some(HabitatEvent::BridgeError { .. })));
    }

    #[test]
    fn handle_result_valid_json_yields_bridge_data() {
        let b = BridgeClient::new();
        let ev = b.handle_result(
            Some(0),
            b"{\"status\":\"healthy\"}",
            &ctx("orac_health", "fleet_view"),
        );
        match ev {
            Some(HabitatEvent::BridgeData {
                tag,
                module_id,
                data,
            }) => {
                assert_eq!(tag, "orac_health");
                assert_eq!(module_id, "fleet_view");
                assert!(data.is_object());
                assert_eq!(data["status"], "healthy");
            }
            other => panic!("expected BridgeData, got {other:?}"),
        }
    }

    #[test]
    fn handle_result_plaintext_body_wraps_as_string_value() {
        // Nerve returns plain "ok" — the plugin must treat this as a successful
        // health probe, not an error. Wire protocol trap #10 in README.md.
        let b = BridgeClient::new();
        let ev = b.handle_result(Some(0), b"ok\n", &ctx("nerve_health", "bridge_health"));
        match ev {
            Some(HabitatEvent::BridgeData { data, .. }) => {
                assert_eq!(data.as_str(), Some("ok"));
            }
            other => panic!("expected BridgeData with string wrap, got {other:?}"),
        }
    }

    #[test]
    fn handle_result_missing_tag_context_returns_none() {
        // If context is malformed, the event must be dropped silently — never
        // synthesise a tag. Silent-default + unknown-tag would route data to the
        // wrong module.
        let b = BridgeClient::new();
        let mut ctx = BTreeMap::new();
        ctx.insert("mod".into(), "fleet_view".into());
        // No "t" key — caller forgot the tag.
        assert!(b.handle_result(Some(0), b"{}", &ctx).is_none());
    }

    #[test]
    fn handle_result_missing_module_id_context_returns_none() {
        let b = BridgeClient::new();
        let mut ctx = BTreeMap::new();
        ctx.insert("t".into(), "orac_health".into());
        assert!(b.handle_result(Some(0), b"{}", &ctx).is_none());
    }

    #[test]
    fn handle_result_none_exit_code_yields_bridge_error() {
        // `Some(0)` is required for success; `None` (spawn-level failure) must not
        // be treated as "body-less success".
        let b = BridgeClient::new();
        let ev = b.handle_result(None, b"", &ctx("x", "y"));
        assert!(matches!(ev, Some(HabitatEvent::BridgeError { .. })));
    }

    // --- poll_due stagger + scheduling ---

    #[test]
    fn poll_due_first_tick_polls_first_endpoint_only() {
        // Quiet-connect (S8): stagger polls one-per-tick on cold start, not 11 parallel.
        let mut b = BridgeClient::new();
        b.register_sources(vec![
            src("http://a/1", 5.0, "a", "m1"),
            src("http://b/1", 5.0, "b", "m1"),
            src("http://c/1", 5.0, "c", "m1"),
        ]);

        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        b.poll_due(1.0, &|argv, _ctx| {
            calls
                .borrow_mut()
                .push(argv.last().copied().unwrap_or("").to_string());
        });

        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(calls.borrow()[0], "http://a/1");
    }

    #[test]
    fn poll_due_completes_stagger_over_successive_ticks() {
        let mut b = BridgeClient::new();
        b.register_sources(vec![
            src("http://a/1", 5.0, "a", "m1"),
            src("http://b/1", 5.0, "b", "m1"),
        ]);

        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let closure = |argv: &[&str], _ctx: BTreeMap<String, String>| {
            calls
                .borrow_mut()
                .push(argv.last().copied().unwrap_or("").to_string());
        };
        b.poll_due(1.0, &closure);
        b.poll_due(1.0, &closure);

        assert_eq!(
            *calls.borrow(),
            vec!["http://a/1".to_string(), "http://b/1".to_string()],
            "stagger must complete within endpoint-count ticks"
        );
    }

    #[test]
    fn poll_due_after_stagger_fires_by_interval_not_every_tick() {
        let mut b = BridgeClient::new();
        b.register_sources(vec![src("http://a/1", 5.0, "a", "m1")]);

        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let closure = |argv: &[&str], _ctx: BTreeMap<String, String>| {
            calls
                .borrow_mut()
                .push(argv.last().copied().unwrap_or("").to_string());
        };

        // Stagger tick — fires once. elapsed=1.0, last_poll=1.0.
        b.poll_due(1.0, &closure);
        assert_eq!(calls.borrow().len(), 1);

        // Two 1s ticks — elapsed=3.0, diff=2.0, still < 5.0, no fire.
        b.poll_due(1.0, &closure);
        b.poll_due(1.0, &closure);
        assert_eq!(calls.borrow().len(), 1, "interval not elapsed");

        // Bump elapsed to 6.0 → diff 5.0 ≥ 5.0 interval → one more fire.
        b.poll_due(3.0, &closure);
        assert_eq!(calls.borrow().len(), 2);
    }

    #[test]
    fn poll_due_propagates_tag_and_module_id_via_context() {
        let mut b = BridgeClient::new();
        b.register_sources(vec![src("http://x", 5.0, "my_tag", "my_module")]);
        let captured: RefCell<Option<BTreeMap<String, String>>> = RefCell::new(None);
        b.poll_due(1.0, &|_argv, ctx| {
            *captured.borrow_mut() = Some(ctx);
        });
        let ctx = captured.borrow().clone().expect("closure ran");
        assert_eq!(ctx.get("t"), Some(&"my_tag".to_string()));
        assert_eq!(ctx.get("mod"), Some(&"my_module".to_string()));
    }

    #[test]
    fn poll_due_argv_head_matches_curl_with_timeout_flags() {
        // The curl invocation shape is a contract — removing `--max-time` would
        // let a hanging endpoint block the plugin indefinitely. Pinned in test.
        let mut b = BridgeClient::new();
        b.register_sources(vec![src("http://x", 5.0, "t", "m")]);
        let argv_captured: RefCell<Vec<String>> = RefCell::new(Vec::new());
        b.poll_due(1.0, &|argv, _ctx| {
            *argv_captured.borrow_mut() = argv.iter().map(|s| (*s).to_string()).collect();
        });
        let argv = argv_captured.borrow();
        assert_eq!(argv[0], "curl");
        assert_eq!(argv[1], "-s");
        assert!(argv.contains(&"--max-time".into()));
        assert!(argv.contains(&"--connect-timeout".into()));
        assert_eq!(argv.last(), Some(&"http://x".to_string()));
    }

    #[test]
    fn poll_due_with_no_endpoints_does_nothing() {
        let mut b = BridgeClient::new();
        let calls = RefCell::new(0_usize);
        b.poll_due(5.0, &|_argv, _ctx| {
            *calls.borrow_mut() += 1;
        });
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn bridge_client_default_matches_new() {
        let a = BridgeClient::new();
        let b = BridgeClient::default();
        assert_eq!(a.endpoints.len(), b.endpoints.len());
    }

    #[test]
    fn register_sources_is_additive_across_calls() {
        let mut b = BridgeClient::new();
        b.register_sources(vec![src("http://a", 5.0, "a", "m1")]);
        b.register_sources(vec![src("http://b", 5.0, "b", "m1")]);
        assert_eq!(b.endpoints.len(), 2);
    }

    // --- command sources (D11 self-poll substrate) ---

    fn cmd(argv: &[&str], interval: f64, tag: &str, module_id: &str) -> CommandSource {
        CommandSource {
            argv: argv.iter().map(|s| (*s).to_string()).collect(),
            interval_secs: interval,
            tag: tag.into(),
            module_id: module_id.into(),
        }
    }

    #[test]
    fn command_source_fires_raw_argv_not_curl_wrapped() {
        // A CommandSource must exec its argv verbatim — NOT inside the curl envelope.
        let mut b = BridgeClient::new();
        b.register_command_sources(vec![cmd(&["/abs/helper", "--once"], 5.0, "snap", "m")]);
        let argv_captured: RefCell<Vec<String>> = RefCell::new(Vec::new());
        b.poll_due(1.0, &|argv, _ctx| {
            *argv_captured.borrow_mut() = argv.iter().map(|s| (*s).to_string()).collect();
        });
        let argv = argv_captured.borrow();
        assert_eq!(
            argv[0], "/abs/helper",
            "command source execs its own argv head"
        );
        assert_eq!(argv.last(), Some(&"--once".to_string()));
        assert!(
            !argv.contains(&"curl".to_string()),
            "must NOT be curl-wrapped"
        );
    }

    #[test]
    fn command_source_routes_tag_and_module_via_context() {
        let mut b = BridgeClient::new();
        b.register_command_sources(vec![cmd(&["/h"], 5.0, "sphere_warden", "sphere_warden")]);
        let captured: RefCell<Option<BTreeMap<String, String>>> = RefCell::new(None);
        b.poll_due(1.0, &|_argv, ctx| {
            *captured.borrow_mut() = Some(ctx);
        });
        let ctx = captured.borrow().clone().expect("ran");
        assert_eq!(ctx.get("t"), Some(&"sphere_warden".to_string()));
        assert_eq!(ctx.get("mod"), Some(&"sphere_warden".to_string()));
    }

    #[test]
    fn data_and_command_sources_share_one_scheduler() {
        // Both kinds live in one endpoints vec and stagger together.
        let mut b = BridgeClient::new();
        b.register_sources(vec![src("http://a", 5.0, "a", "m")]);
        b.register_command_sources(vec![cmd(&["/h"], 5.0, "b", "m")]);
        assert_eq!(b.endpoints.len(), 2);
        let heads: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let closure = |argv: &[&str], _c: BTreeMap<String, String>| {
            heads
                .borrow_mut()
                .push(argv.first().copied().unwrap_or("").to_string());
        };
        b.poll_due(1.0, &closure); // stagger: endpoint 0 (curl)
        b.poll_due(1.0, &closure); // stagger: endpoint 1 (/h)
        assert_eq!(*heads.borrow(), vec!["curl".to_string(), "/h".to_string()]);
    }

    #[test]
    fn command_source_result_routes_through_handle_result_as_bridge_data() {
        // End-to-end: a command source's JSON stdout becomes BridgeData on its tag.
        let b = BridgeClient::new();
        let ev = b.handle_result(
            Some(0),
            b"{\"v\":1}",
            &ctx("fiber_snapshot", "fiber_cockpit"),
        );
        match ev {
            Some(HabitatEvent::BridgeData {
                tag,
                module_id,
                data,
            }) => {
                assert_eq!(tag, "fiber_snapshot");
                assert_eq!(module_id, "fiber_cockpit");
                assert_eq!(data["v"], 1);
            }
            other => panic!("expected BridgeData, got {other:?}"),
        }
    }
}
