#!/usr/bin/env bash
set -euo pipefail

DEST="${HOME}/.local/bin/orch-kernelctl"
MODE="dry-run"

case "${1:---dry-run}" in
  --dry-run) MODE="dry-run" ;;
  --apply) MODE="apply" ;;
  *)
    echo "usage: $0 [--dry-run|--apply]" >&2
    exit 2
    ;;
esac

if [[ "${MODE}" == "dry-run" ]]; then
  echo "orch-kernel-rollback: dry-run target=${DEST}"
  exit 0
fi

if [[ -e "${DEST}.bak" ]]; then
  install -m 0755 "${DEST}.bak" "${DEST}"
  echo "orch-kernel-rollback: restored ${DEST}.bak to ${DEST}"
else
  echo "orch-kernel-rollback: no backup at ${DEST}.bak" >&2
  exit 4
fi
