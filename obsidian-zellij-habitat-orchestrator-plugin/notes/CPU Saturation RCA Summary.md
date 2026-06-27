# CPU Saturation RCA Summary

> Back to: [[MOC]] · [[Bugs & Known Issues]] · [[Dashboard Modules]]
> **Canonical:** `ai_docs/CPU_SATURATION_RCA_S1008517.md` · main vault `[[CPU Saturation — fiber-cockpit Subprocess Storm (S1008517)]]`

This is a **summary** note for the vault — the canonical RCA lives in the
workspace `ai_docs/`. Read that file for the full Mermaid schematics and
operator runbook.

---

## What happened

Host load reached ~3500 on 16 cores. Root cause: ~1,400 overlapping
`fiber-cockpit-snapshot` subprocesses from the `fiber_cockpit` D11 witness.

**Mechanism:**
- `fiber_cockpit` self-polls `bin/fiber-cockpit-snapshot` at 5s cadence.
- With 11+ Zellij servers active (session multiplied across Fleet tabs),
  each with its own plugin instance, the effective poll rate was
  11 × (1/5s) = ~2.2 calls/sec.
- Each call spawned a subprocess with O(KV) fan-out — multiple `atuin kv get`
  operations per invocation.
- **Secondary amplifier:** MemPalace scheduled mine ran concurrently and
  consumed additional RAM.

---

## Fixes shipped

| Fix | Description |
|---|---|
| **flock guard** | `bin/fiber-cockpit-snapshot` wraps itself in `flock` to prevent overlapping runs of the same script |
| **Lease cap** | Maximum concurrent subprocess leases enforced |
| **Cadence 5s → 30s** | `fiber_cockpit` and `sphere_warden` poll interval lengthened to 30s |
| **Emergency reset** | Documented in canonical RCA runbook |

---

## Standing constraints for this repo

1. **D11 witness cadence must be ≥ 30s.** If you change `command_sources()`
   intervals, verify the new concurrent subprocess count across all active
   sessions first.
2. **Flock guard must be active.** Validate before deploying any change to
   `bin/fiber-cockpit-snapshot` or `bin/zj-sphere-warden`.
3. **Never run both D11 witnesses + MemPalace mine concurrently on a
   resource-constrained host** without timing analysis.

---

## Relevance to the stagger pattern

The `BridgeClient` stagger (L4 in [[notes/Durable Lessons & Design Decisions]])
was reinforced by this RCA. Even with stagger, 20 concurrent endpoints × many
instances can saturate the host. The stagger prevents the T=0 spike; the 30s
cadence prevents the steady-state accumulation.
