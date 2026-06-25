#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <receipt-dir>" >&2
  exit 2
fi

RECEIPT_DIR="$1"
LEGACY_WASM_DEST="${HOME}/.config/zellij/plugins/habitat-plugin.wasm"
VERSIONED_WASM_DEST="${HOME}/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm"
LAYOUT_DEST="${HOME}/.config/zellij/layouts/synth-orchestrator.kdl"
CONFIG_DEST="${HOME}/.config/zellij/config.kdl"

json_string_value() {
  local value="${1-}"
  printf '"%s"' "$(printf '%s' "${value}" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')"
}

restore_if_present() {
  local src="$1"
  local dest="$2"
  local mode="$3"
  if [[ -f "${src}" ]]; then
    install -m "${mode}" "${src}" "${dest}"
    echo "restored ${dest}"
  else
    echo "skip missing ${src}"
  fi
}

restore_if_present "${RECEIPT_DIR}/habitat-plugin.wasm.before" "${LEGACY_WASM_DEST}" "0755"
restore_if_present "${RECEIPT_DIR}/habitat-plugin-v0.1.2.wasm.before" "${VERSIONED_WASM_DEST}" "0755"
restore_if_present "${RECEIPT_DIR}/synth-orchestrator.kdl.before" "${LAYOUT_DEST}" "0644"
restore_if_present "${RECEIPT_DIR}/config.kdl.before" "${CONFIG_DEST}" "0644"

cat > "${RECEIPT_DIR}/rollback.json" <<JSON
{
  "schema": "habitat.kernel.persistent.rollback.v1",
  "framework": "ai_docs/ZELLIJ_ORCHESTRATOR_KERNEL_DEPLOYMENT_FRAMEWORK_S1008736.md",
  "created_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "receipt_dir": $(json_string_value "${RECEIPT_DIR}"),
  "verdict": "ROLLBACK_APPLIED"
}
JSON
