#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DURATION_SECONDS="${DURATION_SECONDS:-3600}"
INTERVAL_SECONDS="${INTERVAL_SECONDS:-30}"
STAMP="${STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUT_PREFIX="${OUT_PREFIX:-plugin-v011-1h}"
OUT_DIR="${ROOT}/receipts/${OUT_PREFIX}-${STAMP}"
DIRECT="${OUT_DIR}/direct-health.tsv"

mkdir -p "${OUT_DIR}"
printf 'iter\tts_utc\tservice\turl\thttp_code\n' >"${DIRECT}"

services=(
  "devops:8082:/health"
  "nerve:8083:/health"
  "toollib:8085:/health"
  "synthex:8092:/health"
  "codesynthor:8111:/health"
  "vms:8120:/health"
  "povm:8125:/health"
  "rm:8130:/health"
  "pv2:8132:/health"
  "orac:8133:/health"
  "orac_health:8134:/health"
  "habitat_memory:8140:/health"
  "workflow_trace:8142:/health"
  "wfe2:8143:/health"
  "me_v2:8180:/api/health"
  "lcm:8200:/health"
  "prometheus_swarm:10002:/health"
)

printf 'plugin-v011-direct-health: start=%s duration=%ss interval=%ss out=%s\n' \
  "${STAMP}" "${DURATION_SECONDS}" "${INTERVAL_SECONDS}" "${OUT_DIR}"

end_epoch="$(($(date +%s) + DURATION_SECONDS))"
iter=0

while [[ "$(date +%s)" -lt "${end_epoch}" ]]; do
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  for spec in "${services[@]}"; do
    name="${spec%%:*}"
    rest="${spec#*:}"
    port="${rest%%:*}"
    path="${rest#*:}"
    url="http://127.0.0.1:${port}${path}"
    code="$(curl -sS -m 2 -o "${OUT_DIR}/direct-${iter}-${name}.json" -w '%{http_code}' "${url}" 2>"${OUT_DIR}/direct-${iter}-${name}.err" || true)"
    printf '%s\t%s\t%s\t%s\t%s\n' "${iter}" "${ts}" "${name}" "${url}" "${code}" >>"${DIRECT}"
  done
  iter="$((iter + 1))"
  sleep "${INTERVAL_SECONDS}"
done

printf 'plugin-v011-direct-health: PASS iterations=%s out=%s\n' "${iter}" "${OUT_DIR}"
