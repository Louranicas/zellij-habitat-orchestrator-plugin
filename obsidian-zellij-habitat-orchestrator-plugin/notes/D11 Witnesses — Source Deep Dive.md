# D11 Witnesses — Source Deep Dive

> Back to: [[MOC]] · [[Dashboard Modules]] · [[notes/CPU Saturation RCA Summary]]
> Source: `crates/habitat-modules/src/{fiber_cockpit,campaign_attention,sphere_warden}.rs`

The three D11 witnesses share a common principle: **they are sensors, not
actors**. Each reads a shared medium and renders observations. None writes to
any substrate. Approved S1007594.

---

## `fiber_cockpit` — the coordination WITNESS

**Role:** Full browsable view of the agentic-factory coordination medium:
hopf campaign fibers, kv-lease table, factory arming-key states.

**Data path:**

```
bin/fiber-cockpit-snapshot   ← runs every 30s via CommandSource
  → stdout JSON (FiberSnapshot)
  → BridgeData { tag: "fiber_snapshot" }
  → FiberCockpit::handle_event
```

Fallback: the `fiber-data` Zellij pipe still works for manual injection.

**Wire types:**

```rust
struct FiberSnapshot {
    v: u32, ts: u64,
    campaigns: Vec<CampaignDoc>,
    leases: Vec<LeaseRow>,
    arming: Vec<ArmRow>,
    errors: Vec<String>,
}
struct CampaignDoc  { name, root, nodes: Vec<FiberNode>, truncated }
struct FiberNode    { r#loop, scale, anchor, parent, status }
struct LeaseRow     { resource, owner, ttl_remaining: i64, note, expired: bool }
struct ArmRow       { key, value }
```

All fields `#[serde(default)]` — a malformed snapshot is ignored (last retained).

**Constants:**
```rust
STALE_THRESHOLD_SECS = 150.0   // 5 missed 30s polls
SNAPSHOT_HELPER = "/home/louranicas/claude-code-workspace/bin/fiber-cockpit-snapshot"
SNAPSHOT_POLL_SECS = 30.0      // was 5s — raised to 30s after S1008517
SNAPSHOT_TAG = "fiber_snapshot"
```

**Keybinds:** `j`/`k` select campaign, `l`/`Enter` expand fiber tree,
`h` back to list, `g` jump to top.

**Golden fixture** (`fiber_snapshot_golden.json`):
Three campaigns: `factory-pulse-s1007594` (4 nodes), `plugin-plans-s1007594`
(3 nodes), `shipwright-pulse-s1007644` (3 nodes). One lease:
`arena.loop-testing` owned by `tenterframe-commission-s1007594` with
`ttl_remaining: 784`. No arming keys, no errors.

---

## `campaign_attention` — the AWARENESS layer

**Role:** Ambient alert view — surfaces only what *changed*: a campaign fiber
grew, a lease is near expiry, an arming key flipped. Quiet by default; loud
when a signal needs human attention.

**Data path:** Shares the SAME `fiber-data` / `fiber_snapshot` BridgeData feed
as `fiber_cockpit`. One feeder, two witnesses — the stigmergy ideal.

```rust
// SHARED — importing FiberSnapshot from fiber_cockpit (DRY within crate)
use crate::fiber_cockpit::FiberSnapshot;
```

**Change detection:** per campaign, a digest `(node_count | status-multiset | armed)` 
is compared against an acked baseline:
```rust
fn campaign_digest(&self, name: &str) -> String {
    // format!("{nodes}|{}|{armed}", sorted_statuses.join(","))
}
```
A change raises `NEW`; cleared by `a` key or `attention-ack` pipe.
Lease warnings are **stateless** — auto-clear on renew/release/expiry.

**Constants:**
```rust
TTL_CRITICAL_SECS = 30    // lease TTL below this → RED
TTL_WARN_SECS = 120       // lease TTL below this → YELLOW
MAX_WATCHED = 4           // cap on campaigns shown
STALE_THRESHOLD_SECS = 90.0  // 3× 30s cadence
SNAPSHOT_TAG = "fiber_snapshot"  // SAME as fiber_cockpit
```

**Pipe commands:**
- `attention-ack` — acknowledge all NEW flags
- `attention-watch <campaign>` — filter to specific campaign
- `attention-unwatch` — clear filter
- `attention-mine <prefix>` — highlight owner-prefixed leases

**Boundary enforcement:** no import of any KV write or lease set/release API.
Zero substrate writes — grep-gated.

---

## `sphere_warden` — the SENSE organ

**Role:** Diagnoses the coverage gap between live Zellij panes and registered
PV2 Kuramoto spheres (the D7 field-under-population symptom).

**Data path:**

```
bin/zj-sphere-warden   ← runs every 30s via CommandSource
  → stdout JSON (WardenStatus)
  → BridgeData { tag: "sphere_warden" }
  → SphereWarden::handle_event
```

**Wire type:**

```rust
struct WardenStatus {
    v: u32, ts: u64, armed: bool, pv2_up: bool,
    spheres: u32, panes: u32, gap: u32,
    actuation: String,    // always "observe-only" in current build
    session: String,      // Zellij session name
    closure: String,      // operator guidance — never executed
    errors: Vec<String>,
}
```

**Constants:**
```rust
WARDEN_HELPER = "/home/louranicas/claude-code-workspace/bin/zj-sphere-warden"
WARDEN_POLL_SECS = 30.0
WARDEN_TAG = "sphere_warden"
STALE_THRESHOLD_SECS = 90.0
```

**Why observe-only (deliberate design constraint):**
The closure path (`pane-vortex-ctl register`) is gated pending:
1. Luke ratifying the sphere-id convention (`domain:session:pane` vs Zellij's
   generic `terminal_N` IDs)
2. Anti-burst discipline (pswarm SIGABRT scar from registration burst)

The `closure` field in `WardenStatus` surfaces the registration command as
**operator guidance** — it is rendered, never executed.

**Golden fixture** (`sphere_warden_golden.json`):
```json
{ "v": 1, "ts": 1781420434, "armed": false, "pv2_up": true,
  "spheres": 7, "panes": 1, "gap": 0, "actuation": "observe-only", "errors": [] }
```
This is a gap-0 state (spheres ≥ panes — no under-population). The live
S1008584 probe showed gap=11 (16 panes / 5 spheres).

---

## Shared architecture across all three

| Property | `fiber_cockpit` | `campaign_attention` | `sphere_warden` |
|---|---|---|---|
| Data source | `CommandSource` own | Shared `fiber_snapshot` BridgeData | `CommandSource` own |
| Poll cadence | 30s (was 5s) | Driven by `fiber_cockpit` | 30s |
| Stale threshold | 150s (5 missed) | 90s (3 missed) | 90s (3 missed) |
| Tag | `fiber_snapshot` | `fiber_snapshot` | `sphere_warden` |
| Write verbs | None | None | None |
| Grep gate | ✅ | ✅ | ✅ |
| serialize_state | yes (scroll, selected) | yes (acked digests) | yes (last status) |

---

## See also

- [[Dashboard Modules]] — all 12 modules overview
- [[notes/CPU Saturation RCA Summary]] — why 30s cadence is mandatory
- [[notes/Event System & Module Trait]] — `CommandSource` definition
- [[notes/Bridge Client & Polling Engine]] — stagger logic
