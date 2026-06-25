#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$(cd "${ROOT}/.." && pwd)"
STATE_DIR="${ORCH_KERNEL_STATE_DIR:-${WORKSPACE}/Orchestrator/operator-kernel/state}"
DB_PATH="${ORCH_KERNEL_DB_PATH:-${STATE_DIR}/orchestrator-kernel.sqlite}"
WASM_EXPECTED_NAME="habitat-plugin-v0.1.2.wasm"

json_escape() {
  sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e ':a;N;$!ba;s/\n/\\n/g'
}

json_string() {
  local value="${1-}"
  if [[ -z "${value}" ]]; then
    printf 'null'
  else
    printf '"%s"' "$(printf '%s' "${value}" | json_escape)"
  fi
}

json_number_or_null() {
  local value="${1-}"
  if [[ "${value}" =~ ^[0-9]+$ ]]; then
    printf '%s' "${value}"
  else
    printf 'null'
  fi
}

sha256_file() {
  local path="$1"
  if [[ -n "${path}" && -f "${path}" ]]; then
    printf 'sha256:%s' "$(sha256sum "${path}" | awk '{print $1}')"
  fi
}

sha256_text() {
  sha256sum | awk '{print "sha256:" $1}'
}

find_orch_kernelctl() {
  if [[ -n "${ORCH_KERNELCTL:-}" && -x "${ORCH_KERNELCTL}" ]]; then
    printf '%s\n' "${ORCH_KERNELCTL}"
    return
  fi
  if command -v orch-kernelctl >/dev/null 2>&1; then
    command -v orch-kernelctl
    return
  fi
  for candidate in \
    "${CARGO_TARGET_DIR:-}/release/orch-kernelctl" \
    "${CARGO_TARGET_DIR:-}/debug/orch-kernelctl" \
    "${ROOT}/target/release/orch-kernelctl" \
    "${ROOT}/target/debug/orch-kernelctl" \
    "/tmp/habitat-zellij-target/debug/orch-kernelctl" \
    "/tmp/habitat-zellij-target/release/orch-kernelctl"
  do
    if [[ -n "${candidate}" && -x "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return
    fi
  done
}

find_wasm() {
  if [[ -n "${HABITAT_PLUGIN_WASM:-}" && -f "${HABITAT_PLUGIN_WASM}" ]]; then
    printf '%s\n' "${HABITAT_PLUGIN_WASM}"
    return
  fi
  for candidate in \
    "${HOME}/.config/zellij/plugins/${WASM_EXPECTED_NAME}" \
    "${WORKSPACE}/${WASM_EXPECTED_NAME}" \
    "${ROOT}/${WASM_EXPECTED_NAME}" \
    "${CARGO_TARGET_DIR:-}/wasm32-wasip1/release/${WASM_EXPECTED_NAME}" \
    "${CARGO_TARGET_DIR:-}/wasm32-wasi/release/${WASM_EXPECTED_NAME}" \
    "/tmp/habitat-zellij-target/wasm32-wasip1/release/${WASM_EXPECTED_NAME}" \
    "/tmp/habitat-zellij-target/wasm32-wasi/release/${WASM_EXPECTED_NAME}" \
    "${CARGO_TARGET_DIR:-}/wasm32-wasip1/release/habitat_plugin.wasm" \
    "${CARGO_TARGET_DIR:-}/wasm32-wasi/release/habitat_plugin.wasm" \
    "${ROOT}/target/wasm32-wasip1/release/habitat_plugin.wasm" \
    "${ROOT}/target/wasm32-wasi/release/habitat_plugin.wasm" \
    "/tmp/habitat-zellij-target/wasm32-wasip1/release/habitat_plugin.wasm" \
    "/tmp/habitat-zellij-target/wasm32-wasi/release/habitat_plugin.wasm" \
    "${ROOT}/target-wasm/release/habitat_plugin.wasm"
  do
    if [[ -f "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return
    fi
  done
}

timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
source_git_rev="$(git -C "${WORKSPACE}" rev-parse HEAD 2>/dev/null || true)"
dirty_count="$(git -C "${WORKSPACE}" status --short 2>/dev/null | wc -l | tr -d ' ' || true)"
if [[ "${dirty_count:-0}" == "0" ]]; then
  dirty_status="clean"
else
  dirty_status="dirty:${dirty_count}"
fi
orch_kernelctl_path="$(find_orch_kernelctl || true)"
orch_kernelctl_hash="$(sha256_file "${orch_kernelctl_path}" || true)"
orch_kernelctl_help_hash=""
if [[ -n "${orch_kernelctl_path}" ]]; then
  orch_kernelctl_help_hash="$("${orch_kernelctl_path}" --help 2>&1 | sha256_text || true)"
fi
sidecar_schema_version=""
if [[ -f "${DB_PATH}" ]] && command -v sqlite3 >/dev/null 2>&1; then
  sidecar_schema_version="$(sqlite3 "${DB_PATH}" "select version from schema_version order by applied_at desc limit 1;" 2>/dev/null || true)"
fi
wasm_path="$(find_wasm || true)"
wasm_hash="$(sha256_file "${wasm_path}" || true)"
zellij_session="${ZELLIJ_SESSION_NAME:-${ZELLIJ:-}}"

cat <<JSON
{
  "schema": "habitat.kernel.identity.bundle.v1",
  "timestamp": "$(printf '%s' "${timestamp}" | json_escape)",
  "workspace": "$(printf '%s' "${WORKSPACE}" | json_escape)",
  "source_git_rev": $(json_string "${source_git_rev}"),
  "dirty_status": "$(printf '%s' "${dirty_status}" | json_escape)",
  "orch_kernelctl_path": $(json_string "${orch_kernelctl_path}"),
  "orch_kernelctl_hash": $(json_string "${orch_kernelctl_hash}"),
  "orch_kernelctl_help_hash": $(json_string "${orch_kernelctl_help_hash}"),
  "sidecar_db_path": "$(printf '%s' "${DB_PATH}" | json_escape)",
  "sidecar_schema_version": $(json_number_or_null "${sidecar_schema_version}"),
  "state_dir": "$(printf '%s' "${STATE_DIR}" | json_escape)",
  "wasm_expected_name": "${WASM_EXPECTED_NAME}",
  "wasm_path": $(json_string "${wasm_path}"),
  "wasm_hash": $(json_string "${wasm_hash}"),
  "zellij_session": $(json_string "${zellij_session}"),
  "plugin_instances": []
}
JSON
