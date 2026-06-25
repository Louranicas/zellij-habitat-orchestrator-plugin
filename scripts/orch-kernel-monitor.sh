#!/usr/bin/env bash
# Multi-level monitor for the Zellij Orchestrator Kernel plugin + sidecar.
# Writes TSV + JSONL receipts. Intended to run in a Zellij pane while tests execute.
set -u

ROOT="/home/louranicas/claude-code-workspace"
START="$(date -u +%Y%m%dT%H%M%SZ)"
DURATION_SECS="${1:-7200}"
INTERVAL_SECS="${2:-10}"
CHECK_PIPE="${CHECK_PIPE:-1}"
RECEIPT="$ROOT/receipts/zellij-orchestrator-kernel-monitor-$START.tsv"
JSONL="$ROOT/receipts/zellij-orchestrator-kernel-monitor-$START.jsonl"
SUMMARY="$ROOT/receipts/zellij-orchestrator-kernel-monitor-$START.summary.md"
ARTIFACT_DIR="$ROOT/receipts/zellij-orchestrator-kernel-monitor-$START.artifacts"
PLUGIN_PATH="${HABITAT_PLUGIN_WASM:-$HOME/.config/zellij/plugins/habitat-plugin.wasm}"
PLUGIN="file:$PLUGIN_PATH"
PLUGIN_CONFIG='modules=orchestrator_kernel,bridge_health,coherence_gauge,role=orchestrator_kernel,sidecar_cli=/home/louranicas/.local/bin/orch-kernelctl,kernel_poll=5'
ZLOG="/tmp/zellij-1000/zellij-log/zellij.log"
ZLOG_START_LINES="$(wc -l < "$ZLOG" 2>/dev/null || echo 0)"
if [[ -z "${ZELLIJ_SESSION_NAME:-}" ]]; then
  ZELLIJ_SESSION_NAME="$(zellij list-sessions --short --no-formatting 2>/dev/null | head -1 || true)"
fi
: "${ZELLIJ:=0}"
export ZELLIJ_SESSION_NAME ZELLIJ
END=$((SECONDS + DURATION_SECS))
iter=0
mkdir -p "$ROOT/receipts" "$ARTIFACT_DIR"
printf 'ts\titer\tlevel\tcheck\tstatus\tdetail\n' > "$RECEIPT"

emit() {
  local level="$1" check="$2" status="$3" detail="$4" ts
  ts="$(date -u +%FT%TZ)"
  detail="$(printf '%s' "$detail" | tr '\t\n' '  ' | cut -c1-900)"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$ts" "$iter" "$level" "$check" "$status" "$detail" >> "$RECEIPT"
  TS="$ts" ITER="$iter" LEVEL="$level" CHECK="$check" STATUS="$status" DETAIL="$detail" python3 - <<'PY' >> "$JSONL"
import os
import json
print(json.dumps({
    "ts": os.environ["TS"],
    "iter": int(os.environ["ITER"]),
    "level": os.environ["LEVEL"],
    "check": os.environ["CHECK"],
    "status": os.environ["STATUS"],
    "detail": os.environ["DETAIL"],
}, ensure_ascii=False))
PY
}

parse_snapshot() {
  python3 - "$1" <<'PY'
import json,sys
try:
    if not open(sys.argv[1], "rb").read(1):
        print("HARNESS_EMPTY_OUTPUT bytes=0")
        raise SystemExit(0)
    d=json.load(open(sys.argv[1]))
    print(f"event_count={d.get('event_count')} last_seq={d.get('last_seq')} verify_chain_ok={d.get('verify_chain_ok')} queue_depth={d.get('queue_depth')} warrants={d.get('warrant_count')} edges={len(d.get('edges', []))}")
except Exception as e:
    print(f"HARNESS_JSON_PARSE_FAIL {e}")
PY
}

emit L0 start INFO "receipt=$RECEIPT jsonl=$JSONL artifacts=$ARTIFACT_DIR duration=${DURATION_SECS}s interval=${INTERVAL_SECS}s plugin=$PLUGIN"

while (( SECONDS < END )); do
  iter=$((iter + 1))
  tick_start=$SECONDS

  # L0 — host/process resource census.
  load="$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || true)"
  mem="$(free -m 2>/dev/null | awk '/Mem:/ {print "mem_used_mb="$3" mem_total_mb="$2} /Swap:/ {print "swap_used_mb="$3" swap_total_mb="$2}' | tr '\n' ' ')"
  okctl_count="$(ps -eo args 2>/dev/null | /usr/bin/grep -F 'orch-kernelctl' | /usr/bin/grep -v grep | wc -l | tr -d ' ')"
  zellij_rss="$(ps -C zellij -o rss= 2>/dev/null | awk '{s+=$1} END{print s+0}')"
  emit L0 host_resources INFO "load=$load $mem orch_kernelctl_processes=$okctl_count zellij_rss_kb=$zellij_rss"

  # L1 — sidecar snapshot and chain verification.
  iter_dir="$ARTIFACT_DIR/iter-$iter"
  mkdir -p "$iter_dir"
  snapshot_json="$iter_dir/snapshot.json"
  snapshot_err="$iter_dir/snapshot.err"
  verify_out="$iter_dir/verify.out"
  verify_err="$iter_dir/verify.err"
  if /home/louranicas/.local/bin/orch-kernelctl snapshot --json >"$snapshot_json" 2>"$snapshot_err"; then
    snapshot_bytes="$(wc -c < "$snapshot_json" 2>/dev/null || echo 0)"
    parsed="$(parse_snapshot "$snapshot_json")"
    if [[ "$parsed" == HARNESS_EMPTY_OUTPUT* ]]; then
      emit L1 sidecar_snapshot HARNESS_FAIL "$parsed artifact=$snapshot_json stderr=$snapshot_err"
    elif [[ "$parsed" == HARNESS_JSON_PARSE_FAIL* ]]; then
      emit L1 sidecar_snapshot HARNESS_FAIL "$parsed bytes=$snapshot_bytes artifact=$snapshot_json stderr=$snapshot_err"
    elif [[ "$parsed" == *'verify_chain_ok=true'* || "$parsed" == *'verify_chain_ok=True'* ]]; then
      emit L1 sidecar_snapshot PASS "$parsed bytes=$snapshot_bytes artifact=$snapshot_json"
    else
      emit L1 sidecar_snapshot DEGRADED "$parsed bytes=$snapshot_bytes artifact=$snapshot_json"
    fi
  else
    emit L1 sidecar_snapshot FAIL "rc=$? stderr=$(tr '\n' ' ' <"$snapshot_err") artifact=$snapshot_json stderr_path=$snapshot_err"
  fi

  if /home/louranicas/.local/bin/orch-kernelctl verify-chain >"$verify_out" 2>"$verify_err"; then
    emit L1 verify_chain PASS "$(tr '\n' ' ' <"$verify_out") artifact=$verify_out"
  else
    emit L1 verify_chain FAIL "$(tr '\n' ' ' <"$verify_err") artifact=$verify_err"
  fi

  # L2 — plugin pipe: invalid must NACK, valid sentinel must answer durably or fail explicitly.
  # In sandboxed Codex shells, zellij pipe can report "There is no active session!" even
  # while list-sessions can see the UI. CHECK_PIPE=0 keeps the monitor useful without
  # generating artificial CliPipe timeout pressure.
  if [[ "$CHECK_PIPE" == "1" ]]; then
    invalid="$(timeout 6 zellij pipe --name kernel --plugin "$PLUGIN" --plugin-configuration "$PLUGIN_CONFIG" -- '{bad json' 2>&1 | tr '\n' ' ' | cut -c1-700)"
    if [[ "$invalid" == *'SCHEMA_INVALID'* ]]; then
      emit L2 plugin_invalid_pipe PASS "$invalid"
    else
      emit L2 plugin_invalid_pipe FAIL "$invalid"
    fi

    sentinel="{\"schema\":\"habitat.kernel.submit.request.v1\",\"trace_id\":\"monitor-$START-$iter\",\"idempotency_key\":\"monitor-$START-$iter\",\"kind\":\"TASK\",\"operator\":\"zen-monitor\",\"payload\":{\"source\":\"orch-kernel-monitor\",\"iter\":$iter}}"
    valid="$(timeout 8 zellij pipe --name kernel --plugin "$PLUGIN" --plugin-configuration "$PLUGIN_CONFIG" -- "$sentinel" 2>&1 | tr '\n' ' ' | cut -c1-900)"
    if [[ -z "$valid" ]]; then
      emit L2 plugin_valid_pipe NO_RESPONSE ""
    elif [[ "$valid" == *'ACK_DURABLE'* && "$valid" == *'event_hash'* ]]; then
      emit L2 plugin_valid_pipe PASS "$valid"
    elif [[ "$valid" == *'NACK'* || "$valid" == *'RECEIVED'* ]]; then
      emit L2 plugin_valid_pipe DEGRADED "$valid"
    else
      emit L2 plugin_valid_pipe UNKNOWN "$valid"
    fi
  else
    emit L2 plugin_pipe SKIP "CHECK_PIPE=0; sandbox cannot reach active zellij pipe context"
  fi

  # L3 — Zellij log deltas relevant to plugin/pipe failures.
  if [[ -f "$ZLOG" ]]; then
    relevant="$(tail -n +$((ZLOG_START_LINES + 1)) "$ZLOG" 2>/dev/null | /usr/bin/grep -E 'PANIC IN PLUGIN|Action CliPipe did not complete|wasm `unreachable`|SIDECAR|orchestrator|kernel' | tail -20 | tr '\n' ' ' | cut -c1-900)"
    count="$(tail -n +$((ZLOG_START_LINES + 1)) "$ZLOG" 2>/dev/null | /usr/bin/grep -Ec 'PANIC IN PLUGIN|Action CliPipe did not complete|wasm `unreachable`' || true)"
    emit L3 zellij_log INFO "critical_count_since_start=$count recent=$relevant"
  else
    emit L3 zellij_log MISSING "$ZLOG missing"
  fi

  # L4 — habitat field/thermal context, sampled cheaply.
  pv2_raw="$(curl -s -m 2 localhost:8132/health 2>/dev/null || true)"
  pv2="$(printf '%s' "$pv2_raw" | python3 -c 'import json,sys; d=json.load(sys.stdin); print("pv2_status=%s r=%s spheres=%s" % (d.get("status"), d.get("r"), d.get("spheres")))' 2>/dev/null || echo pv2_down_or_unparsed)"
  orac_raw="$(curl -s -m 2 localhost:8133/health 2>/dev/null || true)"
  orac="$(printf '%s' "$orac_raw" | python3 -c 'import json,sys; d=json.load(sys.stdin); print("orac_status=%s field_r=%s ralph_gen=%s" % (d.get("status"), d.get("field_r"), d.get("ralph_gen")))' 2>/dev/null || echo orac_down_or_unparsed)"
  emit L4 habitat_context INFO "$pv2 $orac"

  elapsed=$((SECONDS - tick_start))
  sleep_for=$((INTERVAL_SECS - elapsed))
  (( sleep_for > 0 )) && sleep "$sleep_for"
done

emit L0 "done" INFO "receipt=$RECEIPT jsonl=$JSONL artifacts=$ARTIFACT_DIR"
{
  echo "# Orchestrator Kernel Monitor Summary"
  echo
  echo "- Started: $START"
  echo "- Duration: ${DURATION_SECS}s"
  echo "- Interval: ${INTERVAL_SECS}s"
  echo "- TSV: $RECEIPT"
  echo "- JSONL: $JSONL"
  echo "- Artifacts: $ARTIFACT_DIR"
  echo
  echo "## Counts"
  awk 'NR>1 {k=$3":"$4":"$5; c[k]++} END {for (k in c) print "- " k " = " c[k]}' "$RECEIPT" | sort
  echo
  echo "## Tail"
  tail -20 "$RECEIPT"
} > "$SUMMARY"
printf 'MONITOR_RECEIPT=%s\nMONITOR_JSONL=%s\nMONITOR_ARTIFACTS=%s\nMONITOR_SUMMARY=%s\n' "$RECEIPT" "$JSONL" "$ARTIFACT_DIR" "$SUMMARY"
