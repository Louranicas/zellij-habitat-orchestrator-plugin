#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STAMP="${STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
DURATION_SECONDS="${DURATION_SECONDS:-10800}"
MONITOR_INTERVAL_SECONDS="${MONITOR_INTERVAL_SECONDS:-15}"
DEEP_TRACE_INTERVAL_SECONDS="${DEEP_TRACE_INTERVAL_SECONDS:-10}"
TELEMETRY_INTERVAL_SECONDS="${TELEMETRY_INTERVAL_SECONDS:-30}"
RANDOM_INTERVAL_SECONDS="${RANDOM_INTERVAL_SECONDS:-15}"
PLUGIN="${HABITAT_PLUGIN_WASM:-${HOME}/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm}"
OUT_DIR="${ROOT}/receipts/plugin-v012-3h-ultratest-${STAMP}"
SUMMARY="${OUT_DIR}/summary.md"
EVENTS="${OUT_DIR}/events.tsv"
PIDS="${OUT_DIR}/pids.tsv"

mkdir -p "${OUT_DIR}"
printf 'ts_utc\tevent\tdetail\n' >"${EVENTS}"
printf 'name\tpid\n' >"${PIDS}"

log_event() {
  local event="$1" detail="$2"
  printf '%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${event}" "${detail}" >>"${EVENTS}"
}

spawn() {
  local name="$1"
  shift
  log_event "spawn" "${name}: $*"
  "$@" >"${OUT_DIR}/${name}.stdout" 2>"${OUT_DIR}/${name}.stderr" &
  local pid=$!
  printf '%s\t%s\n' "${name}" "${pid}" >>"${PIDS}"
}

if [[ ! -f "${PLUGIN}" ]]; then
  log_event "fatal" "missing plugin ${PLUGIN}"
  echo "plugin-v012-3h-ultratest-suite: missing plugin ${PLUGIN}" >&2
  exit 2
fi

plugin_sha="$(sha256sum "${PLUGIN}" | awk '{print $1}')"
log_event "start" "stamp=${STAMP} duration=${DURATION_SECONDS}s plugin=${PLUGIN} sha256=${plugin_sha}"

spawn monitor env \
  HABITAT_PLUGIN_WASM="${PLUGIN}" \
  CHECK_PIPE="${CHECK_PIPE:-1}" \
  "${ROOT}/habitat-zellij/scripts/orch-kernel-monitor.sh" \
  "${DURATION_SECONDS}" "${MONITOR_INTERVAL_SECONDS}"

spawn deep_trace env \
  HABITAT_PLUGIN_WASM="${PLUGIN}" \
  "${ROOT}/habitat-zellij/scripts/orch-kernel-deep-trace.sh" \
  "${DURATION_SECONDS}" "${DEEP_TRACE_INTERVAL_SECONDS}"

spawn telemetry env \
  STAMP="${STAMP}" \
  OUT_PREFIX="plugin-v012-3h" \
  HABITAT_PLUGIN_WASM="${PLUGIN}" \
  DURATION_SECONDS="${DURATION_SECONDS}" \
  INTERVAL_SECONDS="${TELEMETRY_INTERVAL_SECONDS}" \
  "${ROOT}/habitat-zellij/scripts/plugin-v011-one-hour-telemetry.sh"

spawn direct_health env \
  STAMP="${STAMP}" \
  OUT_PREFIX="plugin-v012-3h" \
  DURATION_SECONDS="${DURATION_SECONDS}" \
  INTERVAL_SECONDS="${TELEMETRY_INTERVAL_SECONDS}" \
  "${ROOT}/habitat-zellij/scripts/plugin-v011-direct-health.sh"

spawn randomized_ultrastress env \
  RUN_ID="${STAMP}_V012_RANDOMIZED" \
  DURATION_SECONDS="${DURATION_SECONDS}" \
  INTERVAL_SECONDS="${RANDOM_INTERVAL_SECONDS}" \
  "${ROOT}/Orchestrator/scripts/orchestrator-randomized-ultrastress.sh"

log_event "running" "pids=$(tr '\n' ';' <"${PIDS}")"

overall_rc=0
while IFS=$'\t' read -r name pid; do
  [[ "${name}" == "name" ]] && continue
  if wait "${pid}"; then
    log_event "child_done" "${name} pid=${pid} rc=0"
  else
    rc=$?
    log_event "child_done" "${name} pid=${pid} rc=${rc}"
    overall_rc=1
  fi
done <"${PIDS}"

telemetry_dir="${ROOT}/receipts/plugin-v012-3h-${STAMP}"
random_dir="${ROOT}/Orchestrator/handshake/randomized-ultrastress-${STAMP}_V012_RANDOMIZED"
monitor_summary="$(ls -1t "${ROOT}"/receipts/zellij-orchestrator-kernel-monitor-*.summary.md 2>/dev/null | head -1 || true)"
deep_summary="$(ls -1td "${ROOT}"/receipts/orch-kernel-deep-trace-* 2>/dev/null | head -1 || true)"

{
  echo "# Habitat Plugin v0.1.2 Three-Hour Ultratest Summary"
  echo
  echo "- Stamp: \`${STAMP}\`"
  echo "- Duration: \`${DURATION_SECONDS}s\`"
  echo "- Plugin: \`${PLUGIN}\`"
  echo "- Plugin SHA256: \`${plugin_sha}\`"
  echo "- Suite dir: \`${OUT_DIR}\`"
  echo "- Overall child status: \`$([[ "${overall_rc}" -eq 0 ]] && echo PASS || echo DEGRADED)\`"
  echo
  echo "## Child Processes"
  sed 's/^/- /' "${PIDS}"
  echo
  echo "## Primary Receipts"
  echo "- Monitor summary: \`${monitor_summary}\`"
  echo "- Deep trace dir: \`${deep_summary}\`"
  echo "- Telemetry dir: \`${telemetry_dir}\`"
  echo "- Randomized ultrastress dir: \`${random_dir}\`"
  echo
  echo "## Suite Events"
  tail -50 "${EVENTS}"
  echo
  echo "## Telemetry Counts"
  if [[ -f "${telemetry_dir}/summary.tsv" ]]; then
    awk 'NR>1{total++; if($3!="'"${plugin_sha}"'") hash_mismatch++; rc["factory:"$4]++; rc["panel:"$5]++; rc["fiber:"$6]++; rc["kernel:"$7]++; rc["cc:"$8]++} END{print "total=" total " hash_mismatch=" (hash_mismatch+0); for(k in rc) print k "=" rc[k]}' "${telemetry_dir}/summary.tsv" | sort
  else
    echo "missing telemetry summary"
  fi
  echo
  echo "## Direct Health Counts"
  if [[ -f "${telemetry_dir}/direct-health.tsv" ]]; then
    awk 'NR>1{c[$3":"$5]++} END{for(k in c) print k "=" c[k]}' "${telemetry_dir}/direct-health.tsv" | sort
  else
    echo "missing direct health"
  fi
  echo
  echo "## Randomized Counts"
  if [[ -f "${random_dir}/summary.tsv" ]]; then
    awk 'NR>1{c[$3":"$4":"$5]++} END{for(k in c) print k "=" c[k]}' "${random_dir}/summary.tsv" | sort
  else
    echo "missing randomized summary"
  fi
} >"${SUMMARY}"

log_event "done" "summary=${SUMMARY} rc=${overall_rc}"
printf 'PLUGIN_V012_ULTRATEST_SUMMARY=%s\n' "${SUMMARY}"
exit "${overall_rc}"
