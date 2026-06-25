use habitat_core::config::ModuleConfig;
use habitat_core::events::{EventCategory, HabitatEvent};
use habitat_core::module::{DataSource, HabitatModule};
use habitat_core::render::*;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG: usize = 50;

pub struct PipeLogEntry {
    pub timestamp: u64,
    pub command: String,
    pub target: String,
    pub outcome: CommandOutcome,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub enum CommandOutcome {
    Pending,
    Success,
    Failure,
    RateLimited,
}

pub struct CmdPipe {
    log: VecDeque<PipeLogEntry>,
    pv2_url: String,
    orac_url: String,
    synthex_url: String,
    last_snapshot: u64,
    snapshot_cooldown_secs: u64,
}

impl CmdPipe {
    #[must_use]
    pub fn new() -> Self {
        Self {
            log: VecDeque::with_capacity(MAX_LOG),
            pv2_url: "http://127.0.0.1:8132".into(),
            orac_url: "http://127.0.0.1:8133".into(),
            synthex_url: "http://127.0.0.1:8090".into(),
            last_snapshot: 0,
            snapshot_cooldown_secs: 60,
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn add_entry(&mut self, entry: PipeLogEntry) {
        self.log.push_front(entry);
        while self.log.len() > MAX_LOG {
            self.log.pop_back();
        }
    }

    fn dispatch(&mut self, command: &str, payload: &str) -> PipeLogEntry {
        let now = Self::now_secs();
        let mut entry = PipeLogEntry {
            timestamp: now,
            command: command.to_string(),
            target: String::new(),
            outcome: CommandOutcome::Pending,
            detail: payload.chars().take(80).collect(),
        };

        match command {
            "snapshot" | "habitat:snapshot" => {
                if now - self.last_snapshot < self.snapshot_cooldown_secs {
                    let remaining = self.snapshot_cooldown_secs - (now - self.last_snapshot);
                    entry.target = format!("{}/bus/cascade", self.pv2_url);
                    entry.outcome = CommandOutcome::RateLimited;
                    entry.detail = format!("cooldown {remaining}s remaining");
                } else {
                    self.last_snapshot = now;
                    entry.target = format!("{}/bus/cascade", self.pv2_url);
                    entry.outcome = CommandOutcome::Success;
                    entry.detail = "snapshot cascade dispatched".into();
                }
            }
            "query" | "habitat:query" => {
                entry.target = format!("{}/sphere/{}", self.pv2_url, payload);
                entry.outcome = CommandOutcome::Success;
            }
            "coherence" | "habitat:coherence" => {
                entry.target = format!("{}/field", self.pv2_url);
                entry.outcome = CommandOutcome::Success;
            }
            "status" | "habitat:status" => {
                entry.target = "internal".into();
                entry.outcome = CommandOutcome::Success;
                entry.detail = format!(
                    "log={} cooldown={}s",
                    self.log.len(),
                    self.snapshot_cooldown_secs
                );
            }
            _ => {
                entry.target = "unknown".into();
                entry.outcome = CommandOutcome::Failure;
                entry.detail = format!("unrecognized command: {command}");
            }
        }

        entry
    }
}

impl Default for CmdPipe {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitatModule for CmdPipe {
    fn id(&self) -> &'static str {
        "cmd_pipe"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn init(&mut self, config: &ModuleConfig) {
        self.pv2_url.clone_from(&config.pv2_url);
        self.orac_url.clone_from(&config.orac_url);
        self.synthex_url.clone_from(&config.synthex_url);
    }

    fn handle_event(&mut self, event: &HabitatEvent) -> bool {
        match event {
            HabitatEvent::PipeCommand { name, payload } => {
                let entry = self.dispatch(name, payload);
                self.add_entry(entry);
                true
            }
            _ => false,
        }
    }

    fn render(&self, rows: usize, cols: usize) -> Vec<RenderLine> {
        let w = cols.min(120);
        let mut lines = Vec::new();

        lines.push(RenderLine::new(format!(
            " {B}{CYN}CMD PIPE{R}  {D}{} entries · snapshot cooldown {}s{R}",
            self.log.len(),
            self.snapshot_cooldown_secs,
        )));
        lines.push(RenderLine::separator(w));

        if self.log.is_empty() {
            lines.push(RenderLine::new(format!(
                " {D}no pipe commands received yet{R}",
            )));
            lines.push(RenderLine::new(format!(
                " {D}usage: zellij pipe -p habitat-plugin.wasm -n <cmd> -- <payload>{R}",
            )));
            lines.push(RenderLine::new(format!(
                " {D}commands: snapshot, query <sphere>, coherence, status{R}",
            )));
        } else {
            let visible = rows.saturating_sub(lines.len() + 1).max(3);
            for entry in self.log.iter().take(visible) {
                let (icon, color) = match entry.outcome {
                    CommandOutcome::Success => (ICON_CHECK, GRN),
                    CommandOutcome::Failure => (ICON_CROSS, RED),
                    CommandOutcome::RateLimited => ("\u{23f1}", YEL),
                    CommandOutcome::Pending => ("\u{25cf}", CYN),
                };

                lines.push(RenderLine::new(format!(
                    " {color}{icon}{R} {B}{:<18}{R} {D}{:<24}{R} {}",
                    truncate(&entry.command, 18),
                    truncate(&entry.target, 24),
                    truncate(&entry.detail, w.saturating_sub(50)),
                )));
            }
        }

        lines
    }

    fn serialize_state(&self) -> Option<String> {
        None
    }
    fn restore_state(&mut self, _state: &str) {}

    fn subscriptions(&self) -> Vec<EventCategory> {
        vec![EventCategory::PipeCommand]
    }

    fn data_sources(&self) -> Vec<DataSource> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_unknown_command_marks_failure_with_detail() {
        let mut p = CmdPipe::new();
        let e = p.dispatch("wombat", "");
        assert_eq!(e.command, "wombat");
        assert_eq!(e.target, "unknown");
        assert!(matches!(e.outcome, CommandOutcome::Failure));
        assert!(e.detail.contains("unrecognized"));
    }

    #[test]
    fn dispatch_snapshot_first_call_succeeds_and_arms_cooldown() {
        let mut p = CmdPipe::new();
        let e1 = p.dispatch("snapshot", "");
        assert!(matches!(e1.outcome, CommandOutcome::Success));
        assert!(e1.target.ends_with("/bus/cascade"));

        // Second immediate call must be rate-limited, not hit the bus.
        let e2 = p.dispatch("snapshot", "");
        assert!(
            matches!(e2.outcome, CommandOutcome::RateLimited),
            "NA S9: snapshot cooldown must prevent spam"
        );
    }

    #[test]
    fn dispatch_query_command_builds_sphere_url() {
        let mut p = CmdPipe::new();
        let e = p.dispatch("query", "alpha-7");
        // Payload is embedded in the path — attribution is preserved.
        assert!(e.target.ends_with("/sphere/alpha-7"));
        assert!(matches!(e.outcome, CommandOutcome::Success));
    }

    #[test]
    fn add_entry_caps_log_at_max_log() {
        let mut p = CmdPipe::new();
        for i in 0..(MAX_LOG + 10) {
            let entry = PipeLogEntry {
                timestamp: i as u64,
                command: format!("cmd_{i}"),
                target: String::new(),
                outcome: CommandOutcome::Success,
                detail: String::new(),
            };
            p.add_entry(entry);
        }
        assert_eq!(p.log.len(), MAX_LOG);
        // Newest at front.
        assert_eq!(
            p.log.front().unwrap().command,
            format!("cmd_{}", MAX_LOG + 9)
        );
    }

    #[test]
    fn handle_event_ignores_non_pipe_events() {
        let mut p = CmdPipe::new();
        let handled = p.handle_event(&HabitatEvent::Tick { tick: 0 });
        assert!(!handled);
        assert_eq!(p.log.len(), 0);
    }

    #[test]
    fn handle_event_pipe_command_records_entry() {
        let mut p = CmdPipe::new();
        let handled = p.handle_event(&HabitatEvent::PipeCommand {
            name: "status".into(),
            payload: String::new(),
        });
        assert!(handled);
        assert_eq!(p.log.len(), 1);
        assert_eq!(p.log.front().unwrap().command, "status");
    }
}
