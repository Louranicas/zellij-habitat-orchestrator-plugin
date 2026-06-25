use crate::config::ModuleConfig;
use crate::events::{EventCategory, HabitatEvent};
use crate::render::RenderLine;

pub struct DataSource {
    pub url: String,
    pub interval_secs: f64,
    pub tag: String,
    pub module_id: String,
}

/// A scheduled command source — like [`DataSource`] but for an ARBITRARY argv the
/// host execs directly (no curl wrapper). Lets a module self-poll a local CLI/helper
/// (e.g. `bin/fiber-cockpit-snapshot`) on an interval; the result arrives through the
/// same `BridgeData { tag }` path a `DataSource` uses. Approved S1007594 (D11) as the
/// additive substrate for the witness self-poll + the sphere-warden status poll.
///
/// The actuation boundary is preserved at a higher layer: a `CommandSource` argv is a
/// READ (or a helper that is itself arming-gated) — modules never gain a write verb by
/// declaring one. `argv[0]` MUST be an absolute path: the host execs it directly with
/// no shell, so `$PATH`/`~` are not expanded.
pub struct CommandSource {
    pub argv: Vec<String>,
    pub interval_secs: f64,
    pub tag: String,
    pub module_id: String,
}

pub trait HabitatModule: Send {
    fn id(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn init(&mut self, config: &ModuleConfig);
    fn handle_event(&mut self, event: &HabitatEvent) -> bool;
    fn render(&self, rows: usize, cols: usize) -> Vec<RenderLine>;
    fn serialize_state(&self) -> Option<String>;
    fn restore_state(&mut self, state: &str);
    fn subscriptions(&self) -> Vec<EventCategory>;
    fn data_sources(&self) -> Vec<DataSource>;

    /// Scheduled local-command sources this module wants polled (default: none).
    ///
    /// Additive with a default impl so the 7 pre-existing modules need no change.
    /// Results are delivered as `HabitatEvent::BridgeData { tag }` exactly like
    /// [`HabitatModule::data_sources`], so a module handles both uniformly.
    fn command_sources(&self) -> Vec<CommandSource> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_source_url_and_tag_round_trip_verbatim() {
        let ds = DataSource {
            url: "http://127.0.0.1:8133/health".into(),
            interval_secs: 5.0,
            tag: "orac_health".into(),
            module_id: "fleet_view".into(),
        };
        // Construction preserves fields; the bridge client depends on exact strings.
        assert_eq!(ds.url, "http://127.0.0.1:8133/health");
        assert_eq!(ds.tag, "orac_health");
        assert_eq!(ds.module_id, "fleet_view");
        assert!((ds.interval_secs - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn command_source_carries_argv_and_routing_fields() {
        let cs = CommandSource {
            argv: vec!["/abs/helper".into(), "--once".into()],
            interval_secs: 30.0,
            tag: "sphere_warden".into(),
            module_id: "sphere_warden".into(),
        };
        assert_eq!(
            cs.argv,
            vec!["/abs/helper".to_string(), "--once".to_string()]
        );
        assert_eq!(cs.tag, "sphere_warden");
        assert_eq!(cs.module_id, "sphere_warden");
        assert!((cs.interval_secs - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn command_sources_trait_default_is_empty() {
        // The default impl means the 7 pre-existing modules opt out for free.
        struct Bare;
        impl HabitatModule for Bare {
            fn id(&self) -> &'static str {
                "bare"
            }
            fn version(&self) -> &'static str {
                "0.0.0"
            }
            fn init(&mut self, _c: &ModuleConfig) {}
            fn handle_event(&mut self, _e: &HabitatEvent) -> bool {
                false
            }
            fn render(&self, _r: usize, _c: usize) -> Vec<RenderLine> {
                Vec::new()
            }
            fn serialize_state(&self) -> Option<String> {
                None
            }
            fn restore_state(&mut self, _s: &str) {}
            fn subscriptions(&self) -> Vec<EventCategory> {
                Vec::new()
            }
            fn data_sources(&self) -> Vec<DataSource> {
                Vec::new()
            }
        }
        assert!(Bare.command_sources().is_empty());
    }
}
