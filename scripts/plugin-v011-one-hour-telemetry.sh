#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DURATION_SECONDS="${DURATION_SECONDS:-3600}"
INTERVAL_SECONDS="${INTERVAL_SECONDS:-30}"
STAMP="${STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUT_PREFIX="${OUT_PREFIX:-plugin-v011-1h}"
OUT_DIR="${ROOT}/receipts/${OUT_PREFIX}-${STAMP}"
SUMMARY="${OUT_DIR}/summary.tsv"
PLUGIN="${HABITAT_PLUGIN_WASM:-/home/louranicas/.config/zellij/plugins/habitat-plugin-v0.1.1.wasm}"

mkdir -p "${OUT_DIR}"

printf 'iter\tts_utc\tplugin_sha\tfactory_status_rc\tpanel_rc\tfiber_rc\tkernel_rc\tcc_health_rc\n' >"${SUMMARY}"
printf 'plugin-v011-one-hour-telemetry: start=%s duration=%ss interval=%ss out=%s\n' \
  "${STAMP}" "${DURATION_SECONDS}" "${INTERVAL_SECONDS}" "${OUT_DIR}"

start_epoch="$(date +%s)"
end_epoch="$((start_epoch + DURATION_SECONDS))"
iter=0

while [[ "$(date +%s)" -lt "${end_epoch}" ]]; do
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  plugin_sha="$(sha256sum "${PLUGIN}" | awk '{print $1}')"

  factory_status_rc=0
  panel_rc=0
  fiber_rc=0
  kernel_rc=0
  cc_health_rc=0

  "${ROOT}/bin/factory-status" --mode gate_only --json >"${OUT_DIR}/factory-status-${iter}.json" 2>"${OUT_DIR}/factory-status-${iter}.err" || factory_status_rc=$?
  "${ROOT}/bin/factory-panel-snapshot" --json >"${OUT_DIR}/factory-panel-${iter}.json" 2>"${OUT_DIR}/factory-panel-${iter}.err" || panel_rc=$?
  "${ROOT}/bin/fiber-cockpit-snapshot" >"${OUT_DIR}/fiber-cockpit-${iter}.json" 2>"${OUT_DIR}/fiber-cockpit-${iter}.err" || fiber_rc=$?
  /home/louranicas/.local/bin/orch-kernelctl snapshot --json >"${OUT_DIR}/orch-kernel-${iter}.json" 2>"${OUT_DIR}/orch-kernel-${iter}.err" || kernel_rc=$?
  cc-health >"${OUT_DIR}/cc-health-${iter}.txt" 2>"${OUT_DIR}/cc-health-${iter}.err" || cc_health_rc=$?

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${iter}" "${ts}" "${plugin_sha}" "${factory_status_rc}" "${panel_rc}" "${fiber_rc}" "${kernel_rc}" "${cc_health_rc}" \
    >>"${SUMMARY}"

  iter="$((iter + 1))"
  sleep "${INTERVAL_SECONDS}"
done

printf 'plugin-v011-one-hour-telemetry: PASS iterations=%s out=%s\n' "${iter}" "${OUT_DIR}"
