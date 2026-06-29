use habitat_bridge_client::BridgeClient;
use habitat_core::config::ModuleConfig;
use habitat_core::events::HabitatEvent;
use habitat_core::module::HabitatModule;
use habitat_core::render::{fmt_num, RenderLine, B, CYN, D, R};
use habitat_modules::bridge_health::BridgeHealth;
use habitat_modules::campaign_attention::CampaignAttention;
use habitat_modules::cmd_pipe::CmdPipe;
use habitat_modules::coherence_gauge::CoherenceGauge;
use habitat_modules::event_feed::EventFeed;
use habitat_modules::fiber_cockpit::FiberCockpit;
use habitat_modules::fleet_view::FleetView;
use habitat_modules::na_panel::NaPanel;
use habitat_modules::orchestrator_kernel::OrchestratorKernel;
use habitat_modules::orchestrator_witness::OrchestratorWitness;
use habitat_modules::session_timer::SessionTimer;
use habitat_modules::sphere_warden::SphereWarden;
use habitat_plugin::kernel_pipe::{
    response_from_sidecar, schema_invalid_response, sidecar_invalid_response,
    sidecar_submit_failed_response, use_sidecar_submit_response,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use zellij_tile::prelude::*;

const POLL_SECS: f64 = 5.0;
const VERSION: &str = "0.1.3";
struct HabitatDashboard {
    modules: Vec<Box<dyn HabitatModule>>,
    bridge: BridgeClient,
    config: ModuleConfig,
    tick: u64,
    state_cache: HashMap<String, String>,
    active_modules: Vec<String>,
}

impl Default for HabitatDashboard {
    fn default() -> Self {
        let (config, _warnings) = ModuleConfig::from_btree(&BTreeMap::new());
        Self {
            modules: Vec::new(),
            bridge: BridgeClient::new(),
            config,
            tick: 0,
            state_cache: HashMap::new(),
            active_modules: Vec::new(),
        }
    }
}

impl HabitatDashboard {
    fn init_modules(&mut self) {
        let module_list = self.active_modules.clone();
        for id in &module_list {
            let module: Option<Box<dyn HabitatModule>> = match id.as_str() {
                "fleet_view" => Some(Box::new(FleetView::new())),
                "bridge_health" => Some(Box::new(BridgeHealth::new())),
                "coherence_gauge" => Some(Box::new(CoherenceGauge::new())),
                "event_feed" => Some(Box::new(EventFeed::new())),
                "cmd_pipe" => Some(Box::new(CmdPipe::new())),
                "na_panel" => Some(Box::new(NaPanel::new())),
                "session_timer" => Some(Box::new(SessionTimer::new())),
                "fiber_cockpit" => Some(Box::new(FiberCockpit::new())),
                "campaign_attention" => Some(Box::new(CampaignAttention::new())),
                "sphere_warden" => Some(Box::new(SphereWarden::new())),
                "orchestrator_kernel" => Some(Box::new(OrchestratorKernel::new())),
                "orchestrator_witness" => Some(Box::new(OrchestratorWitness::new())),
                _ => None,
            };
            if let Some(mut m) = module {
                m.init(&self.config);
                if let Some(state) = self.state_cache.get(m.id()) {
                    m.restore_state(state);
                }
                self.bridge.register_sources(m.data_sources());
                self.bridge.register_command_sources(m.command_sources());
                self.modules.push(m);
            }
        }
    }

    fn dispatch_event(&mut self, event: &HabitatEvent) -> bool {
        let mut needs_render = false;
        let cat = event.category();
        for module in &mut self.modules {
            if module.subscriptions().contains(&cat) {
                needs_render |= module.handle_event(event);
            }
        }
        needs_render
    }

    fn snapshot_state(&mut self) {
        for module in &self.modules {
            if let Some(state) = module.serialize_state() {
                self.state_cache.insert(module.id().to_string(), state);
            }
        }
    }

    fn handle_kernel_run_result(
        exit_code: Option<i32>,
        stdout: &[u8],
        stderr: &[u8],
        context: &BTreeMap<String, String>,
    ) -> bool {
        let Some(pipe_id) = context.get("kernel_pipe_id") else {
            return false;
        };
        let trace_id = context.get("kernel_trace_id").cloned().unwrap_or_default();
        let stdout_text = String::from_utf8_lossy(stdout).trim().to_string();
        let stderr_text = String::from_utf8_lossy(stderr).trim().to_string();
        let response = if exit_code == Some(0) {
            match serde_json::from_str::<Value>(&stdout_text) {
                Ok(sidecar) => response_from_sidecar(&trace_id, &sidecar),
                Err(_) => sidecar_invalid_response(&trace_id),
            }
        } else {
            sidecar_submit_failed_response(&trace_id, &stderr_text)
        };
        cli_pipe_output(pipe_id, &response.to_string());
        unblock_cli_pipe_input(pipe_id);
        true
    }

    fn handle_kernel_pipe(&self, pipe_message: &PipeMessage, payload: &str) -> bool {
        let parsed: Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            Err(err) => {
                if let PipeSource::Cli(ref pipe_id) = pipe_message.source {
                    let response = schema_invalid_response(&format!("SCHEMA_INVALID: {err}"));
                    cli_pipe_output(pipe_id, &response.to_string());
                    unblock_cli_pipe_input(pipe_id);
                }
                return true;
            }
        };
        let trace_id = parsed
            .get("trace_id")
            .and_then(Value::as_str)
            .unwrap_or("kernel-pipe")
            .to_string();

        let PipeSource::Cli(ref pipe_id) = pipe_message.source else {
            return true;
        };

        let response = use_sidecar_submit_response(&trace_id);
        cli_pipe_output(pipe_id, &response.to_string());
        unblock_cli_pipe_input(pipe_id);
        true
    }
}

register_plugin!(HabitatDashboard);

impl ZellijPlugin for HabitatDashboard {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        let (config, warnings) = ModuleConfig::from_btree(&configuration);
        self.config = config;
        for w in &warnings {
            // Warnings go to Zellij's plugin stderr via eprintln, visible in
            // /tmp/zellij-*/zellij-log/zellij.log. See Hardening Plan §WS-0 P2.
            eprintln!("habitat-zellij config warning: {w:?}");
        }

        // Default dashboard surface: fleet vitals + 15-service bridge health, then
        // the three D11 agentic-factory witnesses (S1007594) so the end-to-end
        // factory stack — hopf fibers/campaigns (fiber_cockpit), ambient lease/arm
        // alerts (campaign_attention), and live pane↔PV2 sphere coverage
        // (sphere_warden, observe-only) — renders on the main dashboard by default.
        // All three are read-only self-pollers (absolute-path host helpers); a
        // layout may still override `modules` to trim the surface.
        let modules_str = configuration.get("modules").cloned().unwrap_or_else(|| {
            if configuration
                .get("role")
                .is_some_and(|role| role == "orchestrator_kernel")
            {
                "orchestrator_kernel,bridge_health,coherence_gauge,orchestrator_witness".into()
            } else {
                "fleet_view,bridge_health,fiber_cockpit,campaign_attention,sphere_warden".into()
            }
        });

        self.active_modules = modules_str
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        request_permission(&[
            PermissionType::RunCommands,
            PermissionType::ReadApplicationState,
            PermissionType::ReadCliPipes,
        ]);

        subscribe(&[
            EventType::Timer,
            EventType::RunCommandResult,
            EventType::Key,
        ]);

        self.init_modules();
        set_timeout(POLL_SECS);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Timer(_) => {
                self.tick += 1;
                let tick_event = HabitatEvent::Tick { tick: self.tick };
                self.dispatch_event(&tick_event);

                self.bridge.poll_due(POLL_SECS, &|args, ctx| {
                    run_command(args, ctx);
                });

                if self.tick.is_multiple_of(60) {
                    self.snapshot_state();
                }

                set_timeout(POLL_SECS);
                true
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                if Self::handle_kernel_run_result(exit_code, &stdout, &stderr, &context) {
                    return true;
                }
                if let Some(event) = self.bridge.handle_result(exit_code, &stdout, &context) {
                    self.dispatch_event(&event)
                } else {
                    false
                }
            }
            Event::Key(key) => match key.bare_key {
                BareKey::Char('q') | BareKey::Esc => {
                    self.snapshot_state();
                    close_focus();
                    true
                }
                BareKey::Char('r') => {
                    self.bridge.poll_due(0.0, &|args, ctx| {
                        run_command(args, ctx);
                    });
                    true
                }
                BareKey::Char(c) => {
                    let event = HabitatEvent::KeyPress { key: c };
                    self.dispatch_event(&event)
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        let payload = pipe_message.payload.clone().unwrap_or_default();
        if pipe_message.name == "kernel" {
            return self.handle_kernel_pipe(&pipe_message, &payload);
        }

        let event = HabitatEvent::PipeCommand {
            name: pipe_message.name.clone(),
            payload: payload.clone(),
        };
        let result = self.dispatch_event(&event);

        if let PipeSource::Cli(ref pipe_id) = pipe_message.source {
            let response = match pipe_message.name.as_str() {
                "status" => {
                    format!(
                        "tick={} modules={}",
                        self.tick,
                        self.active_modules.join(",")
                    )
                }
                _ => "ok".to_string(),
            };
            cli_pipe_output(pipe_id, &response);
            unblock_cli_pipe_input(pipe_id);
        }
        result
    }

    fn render(&mut self, rows: usize, cols: usize) {
        let w = cols.min(120);

        println!(
            "{B}{CYN} HABITAT{R} {D}v{VERSION}{R}  {D}tick {}{R}",
            fmt_num(self.tick),
        );
        println!("{}", RenderLine::separator(w).content);

        let mut used_rows = 2;
        for module in &self.modules {
            let lines = module.render(rows.saturating_sub(used_rows + 2), cols);
            for line in &lines {
                println!("{}", line.content);
            }
            used_rows += lines.len();
            if used_rows + 3 >= rows {
                break;
            }
        }

        let remaining = rows.saturating_sub(used_rows + 1);
        for _ in 0..remaining {
            println!();
        }

        println!(
            "{D}[r]efresh [q]uit  {CYN}{}{R}/{D}{}{R} modules{R}",
            self.modules.len(),
            self.active_modules.len(),
        );
    }
}
