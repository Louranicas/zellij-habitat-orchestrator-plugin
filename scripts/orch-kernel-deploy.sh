#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/habitat-zellij-target}"
MODE="dry-run"

case "${1:---dry-run}" in
  --dry-run) MODE="dry-run" ;;
  --apply) MODE="apply" ;;
  *)
    echo "usage: $0 [--dry-run|--apply]" >&2
    exit 2
    ;;
esac

cd "${ROOT}"
CARGO_TARGET_DIR="${TARGET_DIR}" cargo build -p orchestrator-kernel-sidecar --bin orch-kernelctl

BIN="${TARGET_DIR}/debug/orch-kernelctl"
DEST="${HOME}/.local/bin/orch-kernelctl"

if [[ "${MODE}" == "dry-run" ]]; then
  echo "orch-kernel-deploy: dry-run binary=${BIN} dest=${DEST}"
  exit 0
fi

ARMED="$(atuin kv get factory.authorize.zellij-orchestrator-kernel 2>/dev/null || true)"
if [[ "${ARMED}" != "armed" ]]; then
  echo "orch-kernel-deploy: refused; factory.authorize.zellij-orchestrator-kernel is not armed" >&2
  exit 3
fi

mkdir -p "$(dirname "${DEST}")"
if [[ -e "${DEST}" ]]; then
  install -m 0755 "${DEST}" "${DEST}.bak"
fi
install -m 0755 "${BIN}" "${DEST}"
echo "orch-kernel-deploy: installed ${DEST}"
