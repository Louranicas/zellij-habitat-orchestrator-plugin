#!/usr/bin/env bash
# capture-fixtures.sh — Tier-2 live-snapshot capture for habitat-zellij test fixtures.
#
# Hardening Plan v3 §WS-0 P2 — Two-tier fixture strategy.
#   Tier 1 (hand-crafted, versioned): crates/habitat-core/tests/fixtures/*.json
#   Tier 2 (live snapshot, gitignored): crates/habitat-core/tests/fixtures/live/*.json
#
# Run manually when the upstream service shape changes. Review + promote to Tier 1
# by hand. DO NOT auto-commit Tier 2 outputs; they're reproducible and may contain
# session-specific identifiers.
#
# Usage: bash scripts/capture-fixtures.sh
# Prereqs: curl, jq, habitat running (PV2 :8132, ORAC :8133)
# Exit codes: 0 all captured (even if some services unreachable — each probe is
# independent and non-blocking); 64 on arg-level misuse.

# No `set -e` — probes must survive individual service outages per Charter §1
# Shell chapter. Each failure is logged; overall exit is 0 unless setup fails.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$SCRIPT_DIR/../crates/habitat-core/tests/fixtures/live"
mkdir -p "$OUT_DIR"

probe() {
  local name="$1"
  local url="$2"
  local out="$OUT_DIR/${name}.json"
  local code
  code=$(curl -s -o "$out" -w '%{http_code}' -m 3 "$url" 2>/dev/null)
  if [[ "$code" == "200" ]]; then
    # Validate JSON; if not JSON (e.g. Nerve plain-text), wrap for consistency.
    if ! jq -e . "$out" >/dev/null 2>&1; then
      local raw
      raw=$(cat "$out")
      jq -n --arg body "$raw" '{"raw": $body}' > "$out"
    fi
    printf "  [OK]    %-30s -> %s\n" "$name" "$(basename "$out")"
  else
    printf "  [miss]  %-30s (http %s, service down?)\n" "$name" "$code"
    rm -f "$out"
  fi
}

echo "Capturing live snapshots to $OUT_DIR ..."
probe orac_health          http://127.0.0.1:8133/health
probe orac_metrics         http://127.0.0.1:8133/metrics
probe orac_bridges         http://127.0.0.1:8133/bridges
probe orac_thermal         http://127.0.0.1:8133/thermal
probe orac_hebbian         http://127.0.0.1:8133/hebbian
probe orac_coupling        http://127.0.0.1:8133/coupling
probe pv2_health           http://127.0.0.1:8132/health
probe pv2_field            http://127.0.0.1:8132/field
probe pv2_field_proposals  http://127.0.0.1:8132/field/proposals
probe pv2_bus_info         http://127.0.0.1:8132/bus/info
probe pv2_bus_events       http://127.0.0.1:8132/bus/events
probe pv2_bus_tasks        http://127.0.0.1:8132/bus/tasks
probe synthex_api_health   http://127.0.0.1:8090/api/health
probe me_api_health        http://127.0.0.1:8180/api/health
probe nerve_health         http://127.0.0.1:8083/health

echo ""
echo "Capture complete. Review with:"
echo "  ls -la $OUT_DIR"
echo "Promote a live snapshot to Tier 1 (manual review) with:"
echo "  cp '$OUT_DIR/<name>.json' 'crates/habitat-core/tests/fixtures/<name>.json'"
