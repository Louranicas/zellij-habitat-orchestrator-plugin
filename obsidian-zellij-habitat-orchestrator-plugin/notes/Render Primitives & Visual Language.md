# Render Primitives & Visual Language

> Back to: [[MOC]] · [[Architecture Schematics]] · [[notes/Event System & Module Trait]]
> Source: `crates/habitat-core/src/render.rs`

`render.rs` is the **visual vocabulary** — every module renders exclusively
through these primitives. This note documents the complete palette so that
reading module source is unambiguous.

---

## ANSI color constants

```rust
pub const R:   &str = "\x1b[0m";    // Reset
pub const B:   &str = "\x1b[1m";    // Bold
pub const D:   &str = "\x1b[2m";    // Dim
pub const GRN: &str = "\x1b[32m";   // Green
pub const YEL: &str = "\x1b[33m";   // Yellow
pub const RED: &str = "\x1b[31m";   // Red
pub const CYN: &str = "\x1b[36m";   // Cyan
pub const MAG: &str = "\x1b[35m";   // Magenta
pub const BLU: &str = "\x1b[34m";   // Blue
```

## Icon constants

```rust
pub const ICON_UP:    &str = "●";   // U+25CF — service UP
pub const ICON_DOWN:  &str = "○";   // U+25CB — service DOWN
pub const ICON_CHECK: &str = "✓";   // U+2713
pub const ICON_CROSS: &str = "✗";   // U+2717
pub const HLINE:      &str = "─";   // U+2500 — separator line
```

---

## `RenderLine`

The atomic render unit. Every module's `render(rows, cols)` returns
`Vec<RenderLine>`. Zellij renders these sequentially, one per terminal row.

```rust
RenderLine::new(content: String)   // arbitrary content
RenderLine::blank()                // empty line (padding)
RenderLine::separator(width)       // dim horizontal rule of `width` ─ chars
```

> ⚠️ The **ANSI-injection open issue (KI-1)** lives exactly here: `RenderLine`
> stores the content string verbatim and emits it to the terminal. A malicious
> `zellij pipe` payload that contains `\x1b[2J` lands in a `CmdPipe` entry
> and reaches `RenderLine::new()` unsanitized. See [[notes/P0 P1 Security Audit (2026-04-22)]] §P0.G9.

---

## `truncate(s, max) → &str`

Limits a string to `max` bytes while **preserving UTF-8 char boundaries**.
Critical because `max` is a byte count, not a char count — the function backs
off from any boundary that would split a multi-byte character.

```
"héllo", max=2 → "h"   // 'é' is 2 bytes; can't include it, so back off to 1 byte
"日本語", max=1 → ""    // 3-byte chars; no whole char fits in 1 byte
```

`truncate` is the **only** character-count limiter in the codebase. It limits
*visible count* but does NOT strip ANSI escape bytes — a raw `\x1b[...m`
occupies bytes in the slice but contributes no visible characters. This is
intentional for coloured prefixes but hazardous for user-supplied payloads.

---

## `fmt_num(n: u64) → String`

Compact display for large numbers. Used in `fiber_cockpit` (campaign count,
lease count) and event counters.

| Range | Output |
|---|---|
| `n < 1_000` | `"42"` — bare digits |
| `1_000 ≤ n < 1_000_000` | `"1.5K"` — 1 decimal |
| `n ≥ 1_000_000` | `"2.5M"` — 1 decimal |

`#[allow(clippy::cast_precision_loss)]` — intentional: display-only, 1dp
formatting accepts f64 rounding at large counts.

---

## `thermal_band(temp, target) → (&'static str, &'static str)`

Returns `(color, label)` for thermal/coupling overlay state.

| `|temp - target|` | Color | Label |
|---|---|---|
| `≤ 0.3` | `GRN` | `"NORMAL"` |
| `0.3 < d ≤ 0.5`, temp < target | `YEL` | `"COOL"` |
| `0.3 < d ≤ 0.5`, temp > target | `YEL` | `"HOT"` |
| `> 0.5` | `RED` | `"CRITICAL"` |

Used by `coherence_gauge` and `bridge_health` to colour-code coupling/thermal
deviation. The target is the *desired* Kuramoto coupling factor (typically
`0.5` from ORAC's thermal PID).

---

## `stale_tag(elapsed_since_valid, threshold_secs) → Option<String>`

P3 staleness indicator. Returns `Some("[STALE 45s]")` in yellow when data
has not refreshed within the threshold.

**Threshold policy (P3 spec):** callers compute `max(3 × interval_secs, 45.0)`.
So a 5s-poll module goes stale at 45s (3 missed polls).

```
stale_tag(0.0, 45.0)   → None    (fresh)
stale_tag(45.0, 45.0)  → None    (inclusive boundary = still fresh)
stale_tag(45.1, 45.0)  → Some("[STALE 45s]")
stale_tag(1e12, 45.0)  → Some("[STALE 9999s]")   (capped at 9999)
```

The 9999s cap prevents a system-sleep-resumed pane from emitting
line-breaking stale tags.

> P3 is NOT STARTED — `BridgeClient.last_valid_tick` does not yet exist and
> modules do not yet call `stale_tag`. The function is ready; the wiring is
> the open work. See [[Task Status & Roadmap]].

---

## `cycle_indicator(phase: &str) → String`

Renders the RALPH cognitive cycle phases as a compact initialism:

```
Phases: Recognize · Act · Learn · Predict · Harden
Unknown phase → defaults to "Recognize" (index 0)
```

Active phase is bold+cyan; inactive phases are dim. Example with phase="Learn":

```
R A L P H  →  [dim]R[/] [dim]A[/] [bold+cyan]L[/] [dim]P[/] [dim]H[/]
```

Used by `coherence_gauge` to surface RALPH's current convergence phase from
the ORAC `/health` response.

---

## Test coverage in `render.rs`

`render.rs` has 20 embedded unit tests covering:
- `truncate`: shorter/exact/longer/UTF-8 boundary/multibyte-only/empty
- `fmt_num`: boundaries (999/1000/1_000_000)
- `thermal_band`: all 4 bands including boundary cases
- `cycle_indicator`: known phase + unknown-defaults-to-Recognize
- `RenderLine`: blank empty, separator width, new preserves content
- `stale_tag`: below/at/over threshold, yellow escape, negative elapsed, 9999 cap

---

## See also

- [[notes/Event System & Module Trait]] — how `Vec<RenderLine>` is produced
- [[notes/P0 P1 Security Audit (2026-04-22)]] — ANSI-injection vector via `RenderLine`
- [[Dashboard Modules]] — which modules use which primitives
