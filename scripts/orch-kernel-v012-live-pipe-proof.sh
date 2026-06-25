#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$(cd "${ROOT}/.." && pwd)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
SESSION="habitat-v012-live-pipe-proof-${STAMP}"
OUT_DIR="${WORKSPACE}/receipts/orch-kernel-v012-live-pipe-proof-${STAMP}"
PLUGIN_PATH="${HABITAT_PLUGIN_WASM:-${HOME}/.config/zellij/plugins/habitat-plugin-v0.1.2.wasm}"
KDL="/tmp/${SESSION}.kdl"
RUNNER="/tmp/${SESSION}.sh"
ZELLIJ_LOG="${ZELLIJ_LOG:-/tmp/zellij-1000/zellij-log/zellij.log}"

mkdir -p "${OUT_DIR}"

write_summary() {
  local verdict="$1" reason="$2"
  python3 - "$OUT_DIR" "$verdict" "$reason" "$SESSION" "$PLUGIN_PATH" <<'PY'
import hashlib
import json
import os
import sys
from datetime import datetime, timezone

out_dir, verdict, reason, session, plugin_path = sys.argv[1:6]

def sha256_file(path):
    if not os.path.exists(path):
        return None
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()

doc = {
    "schema": "habitat.kernel.v012.live_pipe_proof.v1",
    "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "session": session,
    "plugin_path": plugin_path,
    "plugin_sha256": sha256_file(plugin_path),
    "valid_response": os.path.join(out_dir, "valid.json"),
    "valid_response_sha256": sha256_file(os.path.join(out_dir, "valid.json")),
    "invalid_response": os.path.join(out_dir, "invalid.json"),
    "invalid_response_sha256": sha256_file(os.path.join(out_dir, "invalid.json")),
    "verdict": verdict,
    "reason": reason,
}
with open(os.path.join(out_dir, "summary.json"), "w") as f:
    json.dump(doc, f, indent=2, sort_keys=True)
    f.write("\n")
PY
}

env_blocked() {
  local reason="$1"
  echo "orch-kernel-v012-live-pipe-proof: ENV_BLOCKED ${reason}" >&2
  write_summary "ENV_BLOCKED" "$reason"
  printf '%s\n' "${OUT_DIR}"
  exit 3
}

cleanup() {
  if command -v zellij >/dev/null 2>&1; then
    zellij delete-session "${SESSION}" --force >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if [[ ! -f "${PLUGIN_PATH}" ]]; then
  env_blocked "missing plugin ${PLUGIN_PATH}"
fi

if ! command -v zellij >/dev/null 2>&1; then
  env_blocked "zellij not found"
fi

if ! command -v script >/dev/null 2>&1; then
  env_blocked "script(1) not found"
fi

if [[ ! -w "${HOME}" ]]; then
  env_blocked "HOME is not writable: ${HOME}"
fi

if [[ ! -w "$(dirname "${KDL}")" ]]; then
  env_blocked "tmp layout directory is not writable: $(dirname "${KDL}")"
fi

cat >"${KDL}" <<KDL
layout {
    tab name="v012-live-pipe-proof" {
        pane split_direction="vertical" {
            pane command="/usr/bin/bash" {
                args "${RUNNER}"
            }
            pane {
                plugin location="file:${PLUGIN_PATH}" {
                    modules "fleet_view,bridge_health,fiber_cockpit,campaign_attention,sphere_warden"
                }
            }
        }
    }
}
KDL

cat >"${RUNNER}" <<RUNNER
#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="${OUT_DIR}"
mkdir -p "\${OUT_DIR}"
sleep 4

zellij action pipe \\
  --name kernel \\
  -- '{"schema":"habitat.kernel.submit.request.v1","trace_id":"v012-live-pipe-${STAMP}","idempotency_key":"v012-live-pipe-${STAMP}","kind":"TASK","operator":"orch-kernel-v012-live-pipe-proof","payload":{"probe":"v012-live-pipe-proof"}}' \\
  > "\${OUT_DIR}/valid.json"

zellij action pipe \\
  --name kernel \\
  -- '{bad json' \\
  > "\${OUT_DIR}/invalid.json"

zellij action list-panes > "\${OUT_DIR}/panes.txt" || true
zellij action dump-screen --path "\${OUT_DIR}/screen.txt" || true
date -u +%Y-%m-%dT%H:%M:%SZ > "\${OUT_DIR}/completed_at.txt"
sleep 1
RUNNER
chmod +x "${RUNNER}"

set +e
timeout 25s script -q -c "zellij --session ${SESSION} --new-session-with-layout ${KDL}" "${OUT_DIR}/typescript.log" >"${OUT_DIR}/script.stdout" 2>"${OUT_DIR}/script.stderr"
script_rc=$?
set -e

if [[ "${script_rc}" -ne 0 && "${script_rc}" -ne 124 ]]; then
  if /usr/bin/grep -qi 'Read-only file system' "${OUT_DIR}/typescript.log" "${OUT_DIR}/script.stderr" "${OUT_DIR}/script.stdout" 2>/dev/null; then
    env_blocked "zellij launch failed due read-only filesystem rc=${script_rc}"
  fi
  echo "orch-kernel-v012-live-pipe-proof: zellij proof command failed rc=${script_rc}" >&2
  write_summary "FAIL" "zellij proof command failed rc=${script_rc}"
  exit "${script_rc}"
fi

if [[ -f "${ZELLIJ_LOG}" ]]; then
  tail -n 160 "${ZELLIJ_LOG}" >"${OUT_DIR}/zellij-tail.log" || true
fi

if [[ ! -s "${OUT_DIR}/valid.json" ]]; then
  echo "orch-kernel-v012-live-pipe-proof: valid pipe response missing" >&2
  write_summary "FAIL" "valid pipe response missing"
  exit 4
fi

if [[ ! -s "${OUT_DIR}/invalid.json" ]]; then
  echo "orch-kernel-v012-live-pipe-proof: invalid pipe response missing" >&2
  write_summary "FAIL" "invalid pipe response missing"
  exit 4
fi

if ! python3 - "${OUT_DIR}/valid.json" "${OUT_DIR}/invalid.json" <<'PY'
import json
import sys

valid_path, invalid_path = sys.argv[1:3]
with open(valid_path) as f:
    valid = json.load(f)
with open(invalid_path) as f:
    invalid = json.load(f)

errors = []
if valid.get("verdict") != "ACK_DURABLE":
    errors.append(f"valid verdict={valid.get('verdict')!r}")
if valid.get("mode") != "B_SEALED_SYNC":
    errors.append(f"valid mode={valid.get('mode')!r}")
if not valid.get("event_hash"):
    errors.append("valid missing event_hash")
if invalid.get("verdict") != "NACK_SCHEMA_INVALID":
    errors.append(f"invalid verdict={invalid.get('verdict')!r}")
if invalid.get("mode") != "A_FAIL_CLOSED":
    errors.append(f"invalid mode={invalid.get('mode')!r}")
if errors:
    raise SystemExit("; ".join(errors))
PY
then
  echo "orch-kernel-v012-live-pipe-proof: pipe response JSON validation failed" >&2
  write_summary "FAIL" "pipe response JSON validation failed"
  exit 5
fi

write_summary "PASS" "valid ACK_DURABLE and invalid NACK_SCHEMA_INVALID"

printf '%s\n' "${OUT_DIR}"
