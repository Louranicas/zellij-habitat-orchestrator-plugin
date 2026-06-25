use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventCategory {
    BridgeResponse,
    KeyPress,
    Tick,
    PipeCommand,
}

#[derive(Clone, Debug)]
pub enum HabitatEvent {
    BridgeData {
        module_id: String,
        tag: String,
        data: Value,
    },
    BridgeError {
        module_id: String,
        tag: String,
    },
    Tick {
        tick: u64,
    },
    KeyPress {
        key: char,
    },
    PipeCommand {
        name: String,
        payload: String,
    },
}

impl HabitatEvent {
    #[must_use]
    pub fn category(&self) -> EventCategory {
        match self {
            Self::BridgeData { .. } | Self::BridgeError { .. } => EventCategory::BridgeResponse,
            Self::Tick { .. } => EventCategory::Tick,
            Self::KeyPress { .. } => EventCategory::KeyPress,
            Self::PipeCommand { .. } => EventCategory::PipeCommand,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_data_categorises_as_bridge_response() {
        let ev = HabitatEvent::BridgeData {
            module_id: "fleet_view".into(),
            tag: "orac_health".into(),
            data: Value::Null,
        };
        assert_eq!(ev.category(), EventCategory::BridgeResponse);
    }

    #[test]
    fn bridge_error_also_categorises_as_bridge_response() {
        // Errors share the category so subscribers can react to both in one match.
        let ev = HabitatEvent::BridgeError {
            module_id: "fleet_view".into(),
            tag: "orac_health".into(),
        };
        assert_eq!(ev.category(), EventCategory::BridgeResponse);
    }

    #[test]
    fn tick_categorises_distinctly_from_bridge() {
        let ev = HabitatEvent::Tick { tick: 42 };
        assert_eq!(ev.category(), EventCategory::Tick);
        assert_ne!(ev.category(), EventCategory::BridgeResponse);
    }

    #[test]
    fn keypress_categorises_distinctly_from_pipe() {
        // Both are user-driven but must route differently — event_feed subscribes to
        // KeyPress, cmd_pipe subscribes to PipeCommand. Conflating them would cause
        // hotkeys to fire as pipe commands.
        let k = HabitatEvent::KeyPress { key: 'j' };
        let p = HabitatEvent::PipeCommand {
            name: "snapshot".into(),
            payload: String::new(),
        };
        assert_eq!(k.category(), EventCategory::KeyPress);
        assert_eq!(p.category(), EventCategory::PipeCommand);
        assert_ne!(k.category(), p.category());
    }
}
