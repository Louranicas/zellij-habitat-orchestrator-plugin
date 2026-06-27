# P0 + P1 Security Audit (2026-04-22)

> Back to: [[MOC]] · [[Security & Admission Boundary]] · [[Bugs & Known Issues]] · source `ai_docs/P0_P1_AUDIT_2026-04-22.md`

Canonical audit from Session 109 (2026-04-22). Scope: P0.G4 backup, P0.G9
cmd_pipe security, P1.G1 WASM boundary, P1.G3 floating instances, P1.G5 dead
structs. **Audit complete — no code changes in this session; fixes queued for
P2+ and Phase 4.**

---

## P0.G4 — Backup v0.1.0 (DONE)

```
/home/louranicas/.config/zellij/plugins/habitat-plugin-v0.1.0.wasm
  SHA256: 2be5217c1c5667942ab335966eb757e13beaede0eefe3ccbaa8cd309f19d9138
  Size:   1.2 MB
```

Rollback: `\cp -f habitat-plugin-v0.1.0.wasm habitat-plugin.wasm`

---

## P0.G9 — cmd_pipe Security Audit

### Finding: command-injection vector does NOT exist

**Threat model from plan §Phase 4:** cmd_pipe accepts arbitrary CLI strings; if
payload reaches `run_command` that is command injection in a permissioned WASM
plugin.

### Traced pipe flow

```
zellij pipe -p habitat-plugin.wasm -n <cmd> -- <payload>
  └─ HabitatDashboard::pipe (main.rs:162)
      └─ HabitatEvent::PipeCommand { name, payload }
          └─ dispatch_event → CmdPipe::handle_event (cmd_pipe.rs:121)
              └─ CmdPipe::dispatch (cmd_pipe.rs:61)  ← payload terminates here
                  └─ PipeLogEntry stored in VecDeque (cmd_pipe.rs:55)
                      └─ rendered via format! in CmdPipe::render (cmd_pipe.rs:130)
```

### Why payload cannot reach `run_command`

- `run_command` has exactly **two call sites**, both in `main.rs`:
  - `main.rs:123` — `Event::Timer` via `self.bridge.poll_due(…)`
  - `main.rs:148` — `BareKey::Char('r')` via `self.bridge.poll_due(0.0, …)`
- Both paths consume `BridgeClient::endpoints`, populated **only** by
  `register_sources(m.data_sources())` at plugin init.
- `CmdPipe::data_sources()` returns `Vec::new()` — CmdPipe never contributes
  endpoints.
- `BridgeClient::poll_due` hard-codes argv:
  `["curl", "-s", "--max-time", "2", "--connect-timeout", "1", &ep.url]`
  — the URL is the only user-controlled byte, comes from `DataSource.url` set at
  module construction, never from runtime input.

**Conclusion: payload is architecturally firewalled from the subprocess boundary.**

### Residual vector — ANSI / control-char injection (MEDIUM)

`CmdPipe::dispatch` interpolates payload into strings that later reach the
terminal via `println!`:
- `format!("{}/sphere/{}", self.pv2_url, payload)` → stored as `entry.target`
- `format!("unrecognized command: {command}")` → error detail
- `entry.detail = payload.chars().take(80).collect()` → first 80 chars verbatim

`CmdPipe::render` emits these into the terminal. `truncate` (in `render.rs`)
limits *visible character count* but does NOT strip ANSI escape bytes. A payload
containing `\x1b[2J\x1b[H` clears the screen; `\x1b]8;;http://…\x07` injects
a clickable hyperlink OSC.

| Class | Severity |
|---|---|
| Command injection via `run_command` | **Not Present** |
| ANSI/control-char injection via render | **Medium** |
| Log-flood DoS (VecDeque capped at MAX_LOG=50) | **Low** |
| Unsanitized command name | **Low** |

### Recommended fix (deferred to Phase 4)

In `CmdPipe::dispatch` (cmd_pipe.rs:61):

```rust
fn sanitize(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !matches!(*c, '\u{1b}' | '\u{7f}'))
        .take(max)
        .collect()
}
// let command = sanitize(command, 64);
// let payload = sanitize(payload, 128);
```

Two unit tests required: escape bytes stripped; length capped at 128.

---

## P1.G1 — WASM boundary (PASS)

```bash
CARGO_TARGET_DIR=/tmp/habitat-zellij-target cargo check \
  -p habitat-core -p habitat-modules -p habitat-bridge-client \
  --target x86_64-unknown-linux-gnu
# Finished `dev` profile in 3.47s
```

The three non-plugin crates compile for host target. Only
`crates/habitat-plugin/` imports `zellij_tile` — confirmed by grep. Tests
live in the three host crates. No further action required.

---

## P1.G3 — Floating instances (observation-only)

Inventory: 4 layout-declared instances (compact + fleet + minimal +
synth-orchestrator). Live probe (2026-04-22T20:55Z): 1 active client on
`terminal_20`; no 7-instance reproduction. Finding: **not reproducible** —
downgraded from P1 to observation-only. Re-check if instances reappear
(`zellij action list-clients` + per-tab `dump-screen`).

---

## P1.G5 — Dead response structs

18 top-level structs in `habitat-core/src/responses.rs` (not 17 as plan claimed).
12 used, 6 dead at top level (33%):

| Status | Struct |
|---|---|
| DEAD | `RalphState`, `Pv2Spheres`, `Pv2BridgesHealth`, `BusInfo`, `SynthexThermal` (+ children `SynthexHeatSource`, `ThermalAdjustments`), `PovmHealth` |
| LIVE | `OracHealth`, `OracBridges`, `OracThermal`, `EmergenceState`, `HebbianState`, `CouplingState`, `TokenState`, `SessionStats`, `Pv2Health`, `Pv2Field`, `BusEvents`, `Proposals` |

Action: single-commit deletion of 8 dead structs before P2. Completed in P2.

---

## Summary

| Item | Status | Outcome |
|---|---|---|
| P0.G4 backup | ✅ DONE | `habitat-plugin-v0.1.0.wasm` in place |
| P0.G9 cmd_pipe audit | ✅ DONE | cmd-injection NOT present; ANSI injection MEDIUM → Phase 4 |
| P1.G1 WASM boundary | ✅ DONE | host target builds 3.47s |
| P1.G3 floating instances | ✅ DONE | not reproducible; observation-only |
| P1.G5 dead structs | ✅ DONE (audit) | 6 dead identified; deletion done in P2 |
