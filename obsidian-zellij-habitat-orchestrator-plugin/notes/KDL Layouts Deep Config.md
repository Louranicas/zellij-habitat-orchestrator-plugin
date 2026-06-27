# KDL Layouts Deep Config

> Back to: [[MOC]] · [[Command Surface]]
> Source: `layouts/*.kdl`

4 KDL layout files ship with the plugin. Each is a self-contained Zellij
layout that pre-configures the plugin with a specific module set and screen
proportion.

---

## `habitat-fleet.kdl` — full dashboard

```kdl
layout {
  tab name="Habitat" focus=true {
    pane split_direction="vertical" {
      pane size="50%" {
        plugin location="file:~/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm" {
          modules "fleet_view,coherence_gauge,bridge_health,event_feed,na_panel,session_timer,cmd_pipe"
          orac_url   "http://127.0.0.1:8133"
          pv2_url    "http://127.0.0.1:8132"
          synthex_url "http://127.0.0.1:8090"
          nerve_url  "http://127.0.0.1:8083"
          coherence_poll  "2.0"
          health_poll     "5.0"
          governance_poll "10.0"
          layout_mode "full"
          show_consent_states "true"
          show_attribution    "true"
        }
      }
      pane size="50%" name="Terminal" {}
    }
  }
}
```

**Modules:** 7 core curl-pollers. **No** D11 witnesses — those are always
opt-in, never in layouts.
**Pane split:** 50% plugin / 50% terminal.
**Show-attribution:** surfaces the originating module for each render row (S1
NA compliance).

---

## `habitat-compact.kdl` — lean 3-module view

```kdl
modules "fleet_view,coherence_gauge,bridge_health"
size="30%"
layout_mode "compact"
```

**Pane split:** 30% plugin / 70% terminal. Designed for developers who want
the health grid peripheral while coding.

---

## `habitat-minimal.kdl` — single-module footer

```kdl
modules "coherence_gauge"
size="4"            // 4 rows, horizontal split
layout_mode "minimal"
```

4-row pane at the top, terminal below. The `coherence_gauge` in minimal mode
renders a single status line: `r=0.93 K=2.5 3 spheres`. Useful as a
always-visible field indicator without consuming significant screen space.

---

## `factory-witness.kdl` — D11 factory-status witness

The most different layout — no `habitat-plugin.wasm`. Instead it runs three
`bash` panes polling the factory CLI tools:

```kdl
tab name="Factory Witness" {
  pane split_direction="vertical" {
    pane command="bash" {
      args "-lc" "while true; do clear; bin/factory-status --json | jq '{verdict,mode,modules:(.modules|length)}'; sleep 10; done"
    }
    pane split_direction="horizontal" {
      pane command="bash" {
        args "-lc" "while true; do clear; bin/factory-wiring --json | jq '{verdict,connected:.summary.connected,optional_down:.summary.optional_down}'; sleep 15; done"
      }
      pane command="bash" {
        args "-lc" "while true; do clear; bin/factory-proof-seal --json | jq '{verdict,findings:(.findings|length)}'; sleep 20; done"
      }
    }
  }
}
```

**Panels:**
| Poll | Shows |
|---|---|
| `factory-status` @ 10s | verdict · mode · module count |
| `factory-wiring` @ 15s | verdict · connected count · optional-down count |
| `factory-proof-seal` @ 20s | verdict · finding count |

**Note:** Uses `default_tab_template` with tab-bar and status-bar — the 3
other layouts omit this (they don't need it in a standalone session).

---

## Config key reference

All plugin config keys set in KDL are parsed by
`ModuleConfig::from_btree(BTreeMap)` in `habitat-core/src/config.rs`.

| Key | Type | Default | Clamped |
|---|---|---|---|
| `modules` | comma-list | `"fleet_view,bridge_health,..."` | — |
| `orac_url` | http/https URL | `http://127.0.0.1:8133` | falls back on bad URL |
| `pv2_url` | http/https URL | `http://127.0.0.1:8132` | falls back on bad URL |
| `synthex_url` | http/https URL | `http://127.0.0.1:8090` | falls back on bad URL |
| `nerve_url` | http/https URL | `http://127.0.0.1:8083` | falls back on bad URL |
| `coherence_poll` | float (secs) | `2.0` | `[1.0, 300.0]` |
| `health_poll` | float (secs) | `5.0` | `[1.0, 300.0]` |
| `governance_poll` | float (secs) | `10.0` | `[1.0, 300.0]` |
| `kernel_poll` | float (secs) | `30.0` | `[1.0, 300.0]` |
| `sidecar_cli` | path | `orch-kernelctl` | — |
| `layout_mode` | `full`/`compact`/`minimal` | `full` | — |
| `show_consent_states` | bool | `false` | — |
| `show_attribution` | bool | `false` | — |

Bad URL → `ConfigWarning::InvalidUrl` + default used. Out-of-range poll →
`ConfigWarning::PollIntervalClamped`. Non-numeric → `ConfigWarning::PollIntervalNotNumeric`.
Plugin always boots regardless of warnings.

---

## `Alt Shift h` launcher (not in layouts)

Configured in `~/.config/zellij/config.kdl` (external to repo):
```kdl
bind "Alt Shift h" { LaunchOrFocusPlugin "file:~/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm" floating=true; }
```

This is the live keybind (Task 2 from PLAN.md). `Alt h` was already taken by
`MoveFocusOrTab "left"` — that's why it's `Alt Shift h`, not `Alt h`.

---

## See also

- [[Command Surface]] — module list, pipe commands, keybinds
- [[notes/Bridge Client & Polling Engine]] — how KDL config keys become poll intervals
- [[notes/Plugin in Habitat Context (Factory Map)]] — which tabs use which layouts
