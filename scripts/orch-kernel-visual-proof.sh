#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$(cd "${ROOT}/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/habitat-zellij-target}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${WORKSPACE}/receipts/zellij-orchestrator-kernel-visual-proof-${STAMP}.md"

SNAPSHOT="$(ORCH_KERNEL_STATE_DIR="${ORCH_KERNEL_STATE_DIR:-${WORKSPACE}/Orchestrator/operator-kernel/state}" \
  CARGO_TARGET_DIR="${TARGET_DIR}" \
  cargo run -q -p orchestrator-kernel-sidecar --bin orch-kernelctl -- snapshot --json)"

{
  echo "# Zellij Orchestrator Kernel Visual Proof ${STAMP}"
  echo
  echo "Source: \`orch-kernelctl snapshot --json\`"
  echo
  echo '```json'
  printf '%s\n' "${SNAPSHOT}"
  echo '```'
  echo
  echo "Dashboard expectation: measured edges render from \`snapshot.edges\`; no edge facts render as \`unmeasured\`."
} > "${OUT}"

echo "orch-kernel-visual-proof: wrote ${OUT}"
