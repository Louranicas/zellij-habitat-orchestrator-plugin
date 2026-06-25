#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$(cd "${ROOT}/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/habitat-zellij-target}"
MODE="dry-run"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RECEIPT_DIR="${WORKSPACE}/receipts/orch-kernel-persistent-promotion-${STAMP}"
WASM_SRC="${HABITAT_PLUGIN_WASM:-${TARGET_DIR}/wasm32-wasip1/release/habitat_plugin.wasm}"
WASM_DEST="${HOME}/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm"
LEGACY_WASM_DEST="${HOME}/.config/zellij/plugins/habitat-plugin.wasm"
LAYOUT_DEST="${HOME}/.config/zellij/layouts/synth-orchestrator.kdl"
CONFIG_DEST="${HOME}/.config/zellij/config.kdl"
ARM_KEY="factory.authorize.zellij-orchestrator-kernel"

case "${1:---dry-run}" in
  --dry-run) MODE="dry-run" ;;
  --apply) MODE="apply" ;;
  *)
    echo "usage: $0 [--dry-run|--apply]" >&2
    exit 2
    ;;
esac

sha256_or_null() {
  local path="$1"
  if [[ -f "${path}" ]]; then
    sha256sum "${path}" | awk '{print $1}'
  else
    printf 'null'
  fi
}

json_string() {
  local value="${1-}"
  if [[ "${value}" == "null" || -z "${value}" ]]; then
    printf 'null'
  else
    printf '"%s"' "$(printf '%s' "${value}" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')"
  fi
}

json_string_value() {
  local value="${1-}"
  printf '"%s"' "$(printf '%s' "${value}" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')"
}

if [[ ! -f "${WASM_SRC}" ]]; then
  echo "orch-kernel-promote-persistent: missing wasm ${WASM_SRC}" >&2
  exit 2
fi

WASM_SRC_HASH="$(sha256_or_null "${WASM_SRC}")"
LEGACY_BEFORE_HASH="$(sha256_or_null "${LEGACY_WASM_DEST}")"
VERSIONED_BEFORE_HASH="$(sha256_or_null "${WASM_DEST}")"
LAYOUT_BEFORE_HASH="$(sha256_or_null "${LAYOUT_DEST}")"
CONFIG_BEFORE_HASH="$(sha256_or_null "${CONFIG_DEST}")"

if [[ "${MODE}" == "dry-run" ]]; then
  cat <<EOF
orch-kernel-promote-persistent: dry-run
  wasm_src=${WASM_SRC}
  wasm_src_sha256=${WASM_SRC_HASH}
  wasm_dest=${WASM_DEST}
  layout=${LAYOUT_DEST}
  config=${CONFIG_DEST}
  receipt_dir=${RECEIPT_DIR}
EOF
  exit 0
fi

ARMED="$(atuin kv get "${ARM_KEY}" 2>/dev/null || true)"
if [[ "${ARMED}" != "armed" ]]; then
  echo "orch-kernel-promote-persistent: refused; ${ARM_KEY} is not armed" >&2
  exit 3
fi

mkdir -p "$(dirname "${WASM_DEST}")" "$(dirname "${LAYOUT_DEST}")" "${RECEIPT_DIR}"

backup_file() {
  local path="$1"
  local label="$2"
  if [[ -f "${path}" ]]; then
    install -m 0644 "${path}" "${RECEIPT_DIR}/${label}.before"
    install -m 0644 "${path}" "${path}.orch-kernel-${STAMP}.bak"
    install -m 0644 "${path}" "${path}.bak"
  fi
}

backup_file "${LEGACY_WASM_DEST}" "habitat-plugin.wasm"
backup_file "${WASM_DEST}" "habitat-plugin-v0.1.2.wasm"
backup_file "${LAYOUT_DEST}" "synth-orchestrator.kdl"
backup_file "${CONFIG_DEST}" "config.kdl"

install -m 0755 "${WASM_SRC}" "${WASM_DEST}"

if [[ -f "${LAYOUT_DEST}" ]]; then
  perl -0pi -e 's#file:~/.config/zellij/plugins/habitat-plugin\.wasm#file:~/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm#g' "${LAYOUT_DEST}"
fi

if [[ -f "${CONFIG_DEST}" ]]; then
  perl -0pi -e 's#file:~/.config/zellij/plugins/habitat-plugin\.wasm#file:~/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm#g' "${CONFIG_DEST}"
fi

WASM_AFTER_HASH="$(sha256_or_null "${WASM_DEST}")"
LAYOUT_AFTER_HASH="$(sha256_or_null "${LAYOUT_DEST}")"
CONFIG_AFTER_HASH="$(sha256_or_null "${CONFIG_DEST}")"

cat > "${RECEIPT_DIR}/promotion.json" <<JSON
{
  "schema": "habitat.kernel.persistent.promotion.v1",
  "framework": "ai_docs/ZELLIJ_ORCHESTRATOR_KERNEL_DEPLOYMENT_FRAMEWORK_S1008736.md",
  "created_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "mode": "persistent_promotion",
  "arm_key": "${ARM_KEY}",
  "target_wasm": "habitat-plugin-v0.1.2.wasm",
  "wasm_src": $(json_string "${WASM_SRC}"),
  "wasm_src_sha256": $(json_string "${WASM_SRC_HASH}"),
  "wasm_dest": $(json_string "${WASM_DEST}"),
  "wasm_dest_sha256_before": $(json_string "${VERSIONED_BEFORE_HASH}"),
  "wasm_dest_sha256_after": $(json_string "${WASM_AFTER_HASH}"),
  "legacy_wasm_dest": $(json_string "${LEGACY_WASM_DEST}"),
  "legacy_wasm_sha256_before": $(json_string "${LEGACY_BEFORE_HASH}"),
  "layout_dest": $(json_string "${LAYOUT_DEST}"),
  "layout_sha256_before": $(json_string "${LAYOUT_BEFORE_HASH}"),
  "layout_sha256_after": $(json_string "${LAYOUT_AFTER_HASH}"),
  "config_dest": $(json_string "${CONFIG_DEST}"),
  "config_sha256_before": $(json_string "${CONFIG_BEFORE_HASH}"),
  "config_sha256_after": $(json_string "${CONFIG_AFTER_HASH}"),
  "rollback_command": $(json_string_value "habitat-zellij/scripts/orch-kernel-rollback-persistent.sh ${RECEIPT_DIR}"),
  "persistent_references": [
    $(json_string_value "${LAYOUT_DEST}"),
    $(json_string_value "${CONFIG_DEST}")
  ],
  "verdict": "PROMOTED_PENDING_REPRODUCTION"
}
JSON

printf '%s\n' "${RECEIPT_DIR}"
