#!/usr/bin/env bash
# Deep trace monitor for Orchestrator Kernel plugin + sidecar under stress.
# Captures multi-level samples, pipe latency, DB tail, zellij log deltas, process census.
set -u

ROOT=/home/louranicas/claude-code-workspace
START=$(date -u +%Y%m%dT%H%M%SZ)
DURATION_SECS=${1:-7200}
INTERVAL_SECS=${2:-5}
OUT_DIR="$ROOT/receipts/orch-kernel-deep-trace-$START"
SAMPLES="$OUT_DIR/samples.tsv"
JSONL="$OUT_DIR/events.jsonl"
ZLOG_DELTA="$OUT_DIR/zellij-delta.log"
PROCESS_TSV="$OUT_DIR/process.tsv"
DB_TSV="$OUT_DIR/db-tail.tsv"
PIPE_TSV="$OUT_DIR/pipe-latency.tsv"
STRESS_TSV="$OUT_DIR/stress-watch.tsv"
SUMMARY="$OUT_DIR/summary.md"
PLUGIN_PATH="${HABITAT_PLUGIN_WASM:-$HOME/.config/zellij/plugins/habitat-plugin.wasm}"
PLUGIN="file:$PLUGIN_PATH"
PLUGIN_CONFIG='modules=orchestrator_kernel,bridge_health,coherence_gauge,role=orchestrator_kernel,sidecar_cli=/home/louranicas/.local/bin/orch-kernelctl,kernel_poll=5'
ZLOG=/tmp/zellij-1000/zellij-log/zellij.log
mkdir -p "$OUT_DIR"
printf 'ts\titer\tlevel\tcheck\tstatus\tdetail\n' > "$SAMPLES"
printf 'ts\titer\tpid\tcomm\tetime\tcpu\tmem\targs\n' > "$PROCESS_TSV"
printf 'ts\titer\tseq\tkind\ttrace_id\tevent_id\tprev_hash\thash\n' > "$DB_TSV"
printf 'ts\titer\tpipe\tstatus\tlatency_ms\tdetail\n' > "$PIPE_TSV"
printf 'ts\titer\tsource\tstatus\tdetail\n' > "$STRESS_TSV"
ZLOG_START_LINES=$(wc -l < "$ZLOG" 2>/dev/null || echo 0)
if [[ -z "${ZELLIJ_SESSION_NAME:-}" ]]; then
  ZELLIJ_SESSION_NAME="$(zellij list-sessions --short --no-formatting 2>/dev/null | head -1 || true)"
fi
: "${ZELLIJ:=0}"
export ZELLIJ_SESSION_NAME ZELLIJ
END=$((SECONDS + DURATION_SECS))
iter=0

emit() {
  local level="$1" check="$2" status="$3" detail="$4" ts
  ts=$(date -u +%FT%TZ)
  detail=$(printf '%s' "$detail" | tr '\t\n' '  ' | cut -c1-1600)
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$ts" "$iter" "$level" "$check" "$status" "$detail" >> "$SAMPLES"
  python3 - <<PY >> "$JSONL"
import json
print(json.dumps({"ts":"$ts","iter":$iter,"level":"$level","check":"$check","status":"$status","detail":r'''$detail'''}, ensure_ascii=False))
PY
}

json_field_summary() {
  python3 - "$1" <<'PY'
import json,sys
try:
    d=json.load(open(sys.argv[1]))
    print("status=%s db=%s events=%s seq=%s chain=%s queue=%s warrants=%s edges=%s generated_at=%s" % (
        d.get('status'), d.get('db_path'), d.get('event_count'), d.get('last_seq'),
        d.get('verify_chain_ok'), d.get('queue_depth'), d.get('warrant_count'),
        len(d.get('edges', [])), d.get('generated_at')))
except Exception as e:
    print(f"HARNESS_PARSE_FAIL {e}")
PY
}

pipe_call() {
  local name="$1" payload="$2" outfile="$3" start_ns end_ns rc latency status detail ts
  ts=$(date -u +%FT%TZ)
  start_ns=$(date +%s%N)
  timeout 8 zellij pipe --name kernel --plugin "$PLUGIN" --plugin-configuration "$PLUGIN_CONFIG" -- "$payload" > "$outfile" 2>"$outfile.err"
  rc=$?
  end_ns=$(date +%s%N)
  latency=$(( (end_ns - start_ns) / 1000000 ))
  detail=$(tr '\t\n' '  ' < "$outfile" | cut -c1-1000)
  if [[ $rc -ne 0 ]]; then
    status="RC_$rc"
    detail="$detail stderr=$(tr '\t\n' ' ' < "$outfile.err" | cut -c1-400)"
  elif [[ -z "$detail" ]]; then
    status="NO_RESPONSE"
  elif [[ "$detail" == *ACK_DURABLE* && "$detail" == *event_hash* ]]; then
    status="ACK_DURABLE"
  elif [[ "$detail" == *SCHEMA_INVALID* ]]; then
    status="NACK_SCHEMA"
  elif [[ "$detail" == *NACK* ]]; then
    status="NACK"
  else
    status="UNKNOWN"
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$ts" "$iter" "$name" "$status" "$latency" "$detail" >> "$PIPE_TSV"
  emit L2 "$name" "$status" "latency_ms=$latency $detail"
}

sample_db_tail() {
  local db="$1" ts
  ts=$(date -u +%FT%TZ)
  if [[ -n "$db" && -f "$db" ]]; then
    sqlite3 -separator $'\t' "$db" "SELECT seq, kind, trace_id, event_id, COALESCE(prev_hash,''), hash FROM event_log ORDER BY seq DESC LIMIT 5;" 2>/tmp/orch-deep-db.err \
      | awk -v ts="$ts" -v iter="$iter" -F '\t' '{print ts "\t" iter "\t" $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6}' >> "$DB_TSV" || \
      emit L1 db_tail FAIL "$(tr '\n' ' ' </tmp/orch-deep-db.err)"
    edge_count=$(sqlite3 "$db" "SELECT COUNT(*) FROM edge_coherence;" 2>/dev/null || echo err)
    msg_counts=$(sqlite3 -separator ',' "$db" "SELECT integration_state, COUNT(*) FROM messages GROUP BY integration_state;" 2>/dev/null | tr '\n' ';')
    emit L1 db_counts INFO "edge_count=$edge_count message_states=$msg_counts"
  else
    emit L1 db_tail MISSING "db_path=$db"
  fi
}

emit L0 start INFO "out_dir=$OUT_DIR duration=${DURATION_SECS}s interval=${INTERVAL_SECS}s session=${ZELLIJ_SESSION_NAME:-unknown}"

while (( SECONDS < END )); do
  iter=$((iter + 1))
  tick_start=$SECONDS
  ts=$(date -u +%FT%TZ)

  load=$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || true)
  mem=$(free -m 2>/dev/null | awk '/Mem:/ {printf "mem=%s/%sMB ",$3,$2} /Swap:/ {printf "swap=%s/%sMB",$3,$2}')
  zrss=$(ps -C zellij -o rss= 2>/dev/null | awk '{s+=$1} END{print s+0}')
  ok_count=$(ps -eo args 2>/dev/null | /usr/bin/grep -F 'orch-kernelctl' | /usr/bin/grep -v grep | wc -l | tr -d ' ')
  stress_count=$(ps -eo args 2>/dev/null | /usr/bin/grep -E 'stress|telemetry|orch-kernel-monitor|orch-kernel-deep-trace' | /usr/bin/grep -v grep | wc -l | tr -d ' ')
  emit L0 host INFO "load=$load $mem zellij_rss_kb=$zrss orch_kernelctl_processes=$ok_count stress_like_processes=$stress_count"

  ps -eo pid,comm,etime,%cpu,%mem,args 2>/dev/null \
    | /usr/bin/grep -E 'orch-kernel|habitat-plugin|zellij|stress|telemetry|cargo test|codex|bwrap' \
    | /usr/bin/grep -v grep \
    | awk -v ts="$ts" -v iter="$iter" '{pid=$1; comm=$2; et=$3; cpu=$4; mem=$5; $1=$2=$3=$4=$5=""; sub(/^ +/,"",$0); print ts "\t" iter "\t" pid "\t" comm "\t" et "\t" cpu "\t" mem "\t" $0}' >> "$PROCESS_TSV"

  db_path=""
  if /home/louranicas/.local/bin/orch-kernelctl snapshot --json > "$OUT_DIR/snapshot-$iter.json" 2>"$OUT_DIR/snapshot-$iter.err"; then
    summary=$(json_field_summary "$OUT_DIR/snapshot-$iter.json")
    db_path=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("db_path", ""))' "$OUT_DIR/snapshot-$iter.json" 2>/dev/null || true)
    if [[ "$summary" == *'chain=True'* || "$summary" == *'chain=true'* ]]; then
      emit L1 sidecar_snapshot PASS "$summary"
    else
      emit L1 sidecar_snapshot DEGRADED "$summary"
    fi
  else
    emit L1 sidecar_snapshot FAIL "$(tr '\n' ' ' < "$OUT_DIR/snapshot-$iter.err")"
  fi

  if /home/louranicas/.local/bin/orch-kernelctl verify-chain > "$OUT_DIR/verify-$iter.json" 2>"$OUT_DIR/verify-$iter.err"; then
    emit L1 verify_chain PASS "$(tr '\n' ' ' < "$OUT_DIR/verify-$iter.json")"
  else
    emit L1 verify_chain FAIL "$(tr '\n' ' ' < "$OUT_DIR/verify-$iter.err")"
  fi
  sample_db_tail "$db_path"

  pipe_call invalid '{bad json' "$OUT_DIR/pipe-invalid-$iter.out"
  valid_payload="{\"schema\":\"habitat.kernel.submit.request.v1\",\"trace_id\":\"deep-$START-$iter\",\"idempotency_key\":\"deep-$START-$iter\",\"kind\":\"TASK\",\"operator\":\"zen-deep-trace\",\"payload\":{\"source\":\"orch-kernel-deep-trace\",\"iter\":$iter}}"
  pipe_call valid "$valid_payload" "$OUT_DIR/pipe-valid-$iter.out"

  if [[ -f "$ZLOG" ]]; then
    tail -n +$((ZLOG_START_LINES + 1)) "$ZLOG" 2>/dev/null > "$OUT_DIR/zellij-since-start.log"
    crit_count=$(/usr/bin/grep -Ec 'PANIC IN PLUGIN|Action CliPipe did not complete|wasm `unreachable`|thread.*panicked|database is locked|chain violation' "$OUT_DIR/zellij-since-start.log" || true)
    recent=$(/usr/bin/grep -E 'PANIC IN PLUGIN|Action CliPipe did not complete|wasm `unreachable`|thread.*panicked|database is locked|chain violation|orchestrator|kernel' "$OUT_DIR/zellij-since-start.log" | tail -12 | tr '\n' ' ' | cut -c1-1500)
    printf '\n===== iter %s %s =====\n%s\n' "$iter" "$ts" "$recent" >> "$ZLOG_DELTA"
    emit L3 zellij_log INFO "critical_count_since_start=$crit_count recent=$recent"
  else
    emit L3 zellij_log MISSING "$ZLOG missing"
  fi

  for f in \
    "$ROOT/Orchestrator/handshake/randomized-ultrastress-20260625TULTRA_PLUGIN_CMD3/summary.tsv" \
    "$ROOT/receipts/plugin-v011-1h-20260625TULTRA_PLUGIN_CMD3/summary.tsv" \
    "$ROOT/receipts/plugin-v011-1h-20260625TULTRA_PLUGIN_CMD3/direct-health.tsv"; do
    if [[ -f "$f" ]]; then
      line=$(tail -1 "$f" | tr '\t' '|' | cut -c1-1200)
      printf '%s\t%s\t%s\t%s\t%s\n' "$ts" "$iter" "$f" "TAIL" "$line" >> "$STRESS_TSV"
    fi
  done
  recent_stress=$(/usr/bin/grep -RInE 'chain violation|database is locked|NO_RESPONSE|panic|timeout|FAIL|DEGRADED|503' \
    "$ROOT/Orchestrator/handshake/randomized-ultrastress-20260625TULTRA_PLUGIN_CMD3" \
    "$ROOT/receipts/plugin-v011-1h-20260625TULTRA_PLUGIN_CMD3" 2>/dev/null | tail -8 | tr '\n' ' ' | cut -c1-1500)
  [[ -n "$recent_stress" ]] && emit L5 stress_anomalies INFO "$recent_stress" || emit L5 stress_anomalies CLEAR "no recent grep hits"

  pv2_raw=$(curl -s -m 2 localhost:8132/health 2>/dev/null || true)
  pv2=$(printf '%s' "$pv2_raw" | python3 -c 'import json,sys; d=json.load(sys.stdin); print("pv2_status=%s r=%s spheres=%s" % (d.get("status"), d.get("r"), d.get("spheres")))' 2>/dev/null || echo pv2_down_or_unparsed)
  orac_raw=$(curl -s -m 2 localhost:8133/health 2>/dev/null || true)
  orac=$(printf '%s' "$orac_raw" | python3 -c 'import json,sys; d=json.load(sys.stdin); print("orac_status=%s field_r=%s ralph_gen=%s" % (d.get("status"), d.get("field_r"), d.get("ralph_gen")))' 2>/dev/null || echo orac_down_or_unparsed)
  emit L4 habitat INFO "$pv2 $orac"

  elapsed=$((SECONDS - tick_start))
  sleep_for=$((INTERVAL_SECS - elapsed))
  (( sleep_for > 0 )) && sleep "$sleep_for"
done

emit L0 done INFO "out_dir=$OUT_DIR"
{
  echo "# Orchestrator Kernel Deep Trace Summary"
  echo
  echo "- Started: $START"
  echo "- Duration: ${DURATION_SECS}s"
  echo "- Interval: ${INTERVAL_SECS}s"
  echo "- Directory: $OUT_DIR"
  echo
  echo "## Status counts"
  awk 'NR>1{c[$3":"$4":"$5]++} END{for(k in c) print "- " k " = " c[k]}' "$SAMPLES" | sort
  echo
  echo "## Pipe latency tail"
  tail -20 "$PIPE_TSV"
  echo
  echo "## Recent anomalies"
  /usr/bin/grep -nE '\t(FAIL|NO_RESPONSE|UNKNOWN|DEGRADED)\t|chain violation|database is locked|panic|unreachable' "$SAMPLES" | tail -50 || true
} > "$SUMMARY"
printf 'DEEP_TRACE_DIR=%s\nSAMPLES=%s\nSUMMARY=%s\n' "$OUT_DIR" "$SAMPLES" "$SUMMARY"
