# Event System & Module Trait

> Back to: [[MOC]] · [[Architecture Schematics]] · [[notes/Bridge Client & Polling Engine]]
> Source: `crates/habitat-core/src/events.rs` · `crates/habitat-core/src/module.rs`

The event system and module trait are the **internal wiring language** of the
plugin. Every data flow — from a curl result landing, to a key press, to a
Zellij pipe command — is converted into a `HabitatEvent` and dispatched to
modules that have subscribed to its category.

---

## `HabitatEvent` — the five variants

```rust
pub enum HabitatEvent {
    BridgeData   { module_id: String, tag: String, data: Value },
    BridgeError  { module_id: String, tag: String },
    Tick         { tick: u64 },
    KeyPress     { key: char },
    PipeCommand  { name: String, payload: String },
}
```

| Variant | Origin | Consumer |
|---|---|---|
| `BridgeData` | `BridgeClient::handle_result` on clean stdout | Modules matched by `module_id` + `tag` |
| `BridgeError` | `handle_result` on non-zero exit or empty stdout | Same — modules show stale/error state |
| `Tick` | Zellij `Event::Timer` (~1s) | `BridgeClient::poll_due`; drives the scheduler |
| `KeyPress` | `Event::Key` (filtered to `BareKey::Char`) | `event_feed` (j/k/g scroll), `cmd_pipe` (r refresh) |
| `PipeCommand` | `Event::PipeMessage` from `zellij pipe` CLI | `cmd_pipe`, `fiber_cockpit`, `campaign_attention` |

## `EventCategory` — the subscription unit

```rust
pub enum EventCategory {
    BridgeResponse,  // BridgeData + BridgeError (modules react to both uniformly)
    KeyPress,
    Tick,
    PipeCommand,
}
```

`BridgeData` and `BridgeError` share `EventCategory::BridgeResponse` so a
module's match arm handles both without a second subscription. This is the
reason the variant distinction (data vs error) exists at the event level but
NOT at the category level. A module that only subscribed to
`BridgeResponse::BridgeData` would miss errors and render stale state as live.

**Test guard from the source** (events.rs):
> "Both [KeyPress and PipeCommand] are user-driven but must route differently —
> `event_feed` subscribes to `KeyPress`, `cmd_pipe` subscribes to
> `PipeCommand`. Conflating them would cause hotkeys to fire as pipe commands."

---

## `HabitatModule` trait

```rust
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
    fn command_sources(&self) -> Vec<CommandSource> { Vec::new() }
}
```

Every module is a `dyn HabitatModule` stored in a `Vec` inside
`HabitatDashboard`. The main event loop calls:
1. `bridge.poll_due(secs, &run_command_fn)` — fires due curl/helper processes
2. `bridge.handle_result(...)` → `HabitatEvent` → dispatched to each module
   that subscribed to the event's category
3. On Zellij render: `module.render(rows, cols)` → `Vec<RenderLine>`

### `command_sources()` — the D11 additive extension

Added in S1007594 as a **default-impl additive extension**. The 7 pre-existing
core curl-polling modules need no change (`Vec::new()` is the default). Only
the 3 D11 witnesses override it:

- `fiber_cockpit` — `/home/louranicas/.../bin/fiber-cockpit-snapshot` @ 30s
- `sphere_warden` — `/home/louranicas/.../bin/zj-sphere-warden` @ 30s
- `campaign_attention` — shares `fiber_cockpit`'s snapshot tag (no own source)

**Absolute path invariant:** `argv[0]` MUST be an absolute path. `run_command`
execs directly — no shell, no `$PATH` expansion.

### `serialize_state` / `restore_state`

Modules serialise their internal state on Zellij `q`/`Esc` close event. On
next `LaunchOrFocusPlugin` start, the serialised string is passed back via
`restore_state`. This gives **hot-reload fidelity** — scroll position,
selected campaign, last snapshot — without any disk write other than Zellij's
own plugin-state cache.

---

## `DataSource` vs `CommandSource`

```rust
pub struct DataSource {
    pub url: String,              // HTTP endpoint (http:// or https://)
    pub interval_secs: f64,       // must be in [1.0, 300.0]
    pub tag: String,              // routing key — unique within module
    pub module_id: String,        // identifies the owning module
}

pub struct CommandSource {
    pub argv: Vec<String>,        // argv[0] MUST be absolute path
    pub interval_secs: f64,
    pub tag: String,
    pub module_id: String,
}
```

Both types arrive as `HabitatEvent::BridgeData { tag, .. }` — the module
handles them identically. The distinction is only in how the bridge client
wraps the call (curl envelope vs raw exec).

The actuation boundary is preserved: a `CommandSource` argv is READ (or a
helper that is itself arming-gated); modules never gain a write verb by
declaring one.

---

## See also

- [[notes/Bridge Client & Polling Engine]] — how events flow from poll to module
- [[Dashboard Modules]] — the 10 modules and their subscriptions
- [[notes/Render Primitives & Visual Language]] — what `Vec<RenderLine>` looks like
