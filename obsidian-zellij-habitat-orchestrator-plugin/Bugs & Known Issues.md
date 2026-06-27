# Bugs & Known Issues

> Back to: [[MOC]] · [[Problem Solving]] · in-repo [docs/TASK_STATUS](../docs/TASK_STATUS.md)

## Open issues (from project CLAUDE.md + P0/P1 audit)

| ID | Severity | Issue | Status / route |
|---|---|---|---|
| KI-1 | MEDIUM | **`cmd_pipe` ANSI-escape injection** — command-injection NOT present (firewalled by architecture); ANSI injection into the pipe render path flagged MEDIUM | → Phase 4 sanitizer (NOT STARTED) |
| KI-2 | LOW | **Silent schema drift** — all response structs use `#[serde(default)]`; upstream field renames are absorbed silently | → P3 `LiveDataCheck` trait + `has_live_data()` (NOT STARTED) |
| KI-3 | LOW | **Polling overhead** — ~20 curl/cycle across modules | → P3 URL dedup + `[STALE Xs]` indicator (NOT STARTED) |
| KI-4 | OBS | **Orphan floating instances** — up to 7 floating plugin instances observed; not reliably reproducible (downgraded P1) | observation only |

## Linked RCAs (canonical lives in workspace ai_docs / main vault)

### CPU Saturation — fiber-cockpit Subprocess Storm (S1008517)
- **Canonical:** `ai_docs/CPU_SATURATION_RCA_S1008517.md` · vault
  `[[CPU Saturation — fiber-cockpit Subprocess Storm (S1008517)]]`
- **What:** host load ~3500 on 16 cores from ~1,400 overlapping
  `fiber-cockpit-snapshot` polls — the [[Dashboard Modules]] D11 witness at 5s
  cadence × 11+ Zellij servers × O(KV) subprocess fan-out. MemPalace scheduled
  mine = secondary RAM amplifier.
- **Fixes shipped:** flock · lease cap · poll cadence 5s→30s · emergency reset.
- **Relevance here:** the `fiber_cockpit` / `sphere_warden` self-poll cadences in
  this repo are the direct trigger; respect the 30s cadence + flock when changing
  `command_sources()` intervals.

### Zellij Memory-Exhaustion / PTY Double-Panic Crash (S1008630)
- **Canonical:** `ai_docs/ZELLIJ_0443_SERVER_PTY_DOUBLE_PANIC_RCA_S1008630.md` ·
  vault `[[Zellij Habitat Memory-Exhaustion Crash — RCA (S1008630)]]`
- **What:** Zellij 0.44.3 server PTY double-panic; cured by hand-patched binary
  `a4c68619` (PTY `.fatal()→.non_fatal()`).
- **Relevance here:** the plugin runs inside Zellij; the witness panes are loaded
  by the patched binary. **Never `cargo install zellij`** (D7) — would revert the
  patch. Rollback: `~/.cargo/bin/zellij.4dfa6d57-classC-pre.rollback`.

## Resolved / non-issues

- **Command-injection in `cmd_pipe`** — confirmed NOT present (P0 audit): the
  architecture firewalls it; pipe args never reach a shell.
- **WASM boundary** — PASS (host `cargo check` clean); the host crates do not
  import `zellij_tile`.

## See also

- [[Security & Admission Boundary]] — the boundary that firewalls cmd-injection
- [[Task Status & Roadmap]] — which phase closes each open issue
