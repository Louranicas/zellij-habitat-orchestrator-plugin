#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STAMP="${STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUT_DIR="${ROOT}/receipts/orch-kernel-v012-zero-touch-verify-${STAMP}"
SUMMARY_JSON="${OUT_DIR}/summary.json"
SUMMARY_MD="${OUT_DIR}/summary.md"

mkdir -p "${OUT_DIR}"

run_probe() {
  local name="$1"
  shift
  local stdout_path="${OUT_DIR}/${name}.stdout"
  local stderr_path="${OUT_DIR}/${name}.stderr"
  set +e
  "$@" >"${stdout_path}" 2>"${stderr_path}"
  local rc=$?
  set -e
  printf '%s\n' "${rc}" >"${OUT_DIR}/${name}.rc"
  return 0
}

latest_random_summary() {
  find "${ROOT}/Orchestrator/handshake" -path '*/randomized-ultrastress-*/summary.json' -type f -print0 2>/dev/null \
    | xargs -0 -r ls -1t 2>/dev/null \
    | head -1 || true
}

latest_live_pipe_summary() {
  find "${ROOT}/receipts" -path '*/orch-kernel-v012-live-pipe-proof-*/summary.json' -type f -print0 2>/dev/null \
    | xargs -0 -r ls -1t 2>/dev/null \
    | head -1 || true
}

latest_monitor_jsonl() {
  find "${ROOT}/receipts" -maxdepth 1 -name 'zellij-orchestrator-kernel-monitor-*.jsonl' -type f -print0 2>/dev/null \
    | xargs -0 -r ls -1t 2>/dev/null \
    | head -1 || true
}

latest_deep_trace_pipe_tsv() {
  find "${ROOT}/receipts" -path '*/orch-kernel-deep-trace-*/pipe-latency.tsv' -type f -print0 2>/dev/null \
    | xargs -0 -r ls -1t 2>/dev/null \
    | head -1 || true
}

run_probe "factory-status-json" just factory-status-json gate_only
run_probe "policy-hash" "${ROOT}/habitat-zellij/scripts/orch-kernel-policy-hash.sh"
run_probe "orch-kernel-score" "${ROOT}/habitat-zellij/scripts/orch-kernel-score.sh"
run_probe "sidecar-snapshot" /home/louranicas/.local/bin/orch-kernelctl snapshot --json
run_probe "sidecar-verify-chain" /home/louranicas/.local/bin/orch-kernelctl verify-chain
run_probe "deep-diff-forge-self-test" "${ROOT}/deep-diff-forge/target/release/deep-diff-forge" --self-test
run_probe "factory-substrate-gate-json" just factory-substrate-gate-json
run_probe "loom-policy-check" just loom-policy-check

LIVE_PIPE_SUMMARY="$(latest_live_pipe_summary)"
RANDOM_SUMMARY="$(latest_random_summary)"
MONITOR_JSONL="$(latest_monitor_jsonl)"
DEEP_TRACE_PIPE_TSV="$(latest_deep_trace_pipe_tsv)"

ROOT="${ROOT}" \
OUT_DIR="${OUT_DIR}" \
STAMP="${STAMP}" \
SUMMARY_JSON="${SUMMARY_JSON}" \
SUMMARY_MD="${SUMMARY_MD}" \
LIVE_PIPE_SUMMARY="${LIVE_PIPE_SUMMARY}" \
RANDOM_SUMMARY="${RANDOM_SUMMARY}" \
MONITOR_JSONL="${MONITOR_JSONL}" \
DEEP_TRACE_PIPE_TSV="${DEEP_TRACE_PIPE_TSV}" \
python3 - <<'PY'
import json
import os
from pathlib import Path
from datetime import datetime, timezone

root = Path(os.environ["ROOT"])
out_dir = Path(os.environ["OUT_DIR"])

def read_text(path):
    try:
        return Path(path).read_text()
    except FileNotFoundError:
        return ""

def read_rc(name):
    try:
        return int((out_dir / f"{name}.rc").read_text().strip())
    except Exception:
        return None

def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except Exception as exc:
        return {"_parse_error": str(exc)}

def gate(name, status, evidence, detail="", klass=None):
    return {
        "name": name,
        "status": status,
        "class": klass or status,
        "evidence": evidence,
        "detail": detail,
    }

gates = []

policy_stdout = out_dir / "policy-hash.stdout"
policy_rc = read_rc("policy-hash")
policy_hash_doc = read_json(policy_stdout) if policy_stdout.exists() else {"_parse_error": "missing stdout"}
if policy_rc == 0 and policy_hash_doc.get("status") == "PASS":
    gates.append(gate("policy_hash_resolver", "PASS", str(policy_stdout), f"policy_hash={policy_hash_doc.get('stored_policy_hash')}", "PRODUCT_PASS"))
else:
    gates.append(gate("policy_hash_resolver", "GOVERNANCE_BLOCKED", str(policy_stdout), json.dumps(policy_hash_doc, sort_keys=True), "GOVERNANCE_BLOCKED"))

factory_stdout = out_dir / "factory-status-json.stdout"
factory_rc = read_rc("factory-status-json")
factory = read_json(factory_stdout) if factory_stdout.exists() else {"_parse_error": "missing stdout"}
required_down = factory.get("services", {}).get("required_down", [])
factory_verdict = factory.get("verdict")
factory_detail = factory.get("reason") or "; ".join(required_down)
if factory_rc == 0 and factory_verdict in ("pass", "ready", "ok", "healthy", "factory-ready"):
    gates.append(gate("factory_readiness", "PASS", str(factory_stdout), factory_detail, "PRODUCT_PASS"))
elif required_down:
    gates.append(gate("factory_readiness", "ENV_BLOCKED", str(factory_stdout), factory_detail, "ENV_BLOCKED"))
elif factory_rc not in (0, None):
    gates.append(gate("factory_readiness", "ENV_BLOCKED", str(factory_stdout), f"rc={factory_rc} {factory_detail}", "ENV_BLOCKED"))
elif factory_rc == 0 and factory_verdict == "degraded":
    gates.append(gate("factory_readiness", "DEGRADED", str(factory_stdout), factory_detail, "PRODUCT_DEGRADED"))
else:
    gates.append(gate("factory_readiness", "UNKNOWN", str(factory_stdout), factory_detail, "COVERAGE_GAP"))

score_stdout = out_dir / "orch-kernel-score.stdout"
score = read_json(score_stdout) if score_stdout.exists() else {"_parse_error": "missing stdout"}
if score.get("verdict") == "READY_FOR_INDEPENDENT_VERIFY" and score.get("score", 0) >= 90:
    gates.append(gate("score_framework", "PASS", str(score_stdout), f"score={score.get('score')} cap={score.get('hard_score_cap')}", "PRODUCT_PASS"))
elif score.get("verdict") == "REQUEST_CHANGES" and score.get("score", 0) >= 80:
    gates.append(gate("score_framework", "DEGRADED", str(score_stdout), json.dumps(score, sort_keys=True), "SEMANTIC_DEGRADED"))
else:
    gates.append(gate("score_framework", "FAIL", str(score_stdout), json.dumps(score, sort_keys=True), "PRODUCT_FAIL"))

snapshot_stdout = out_dir / "sidecar-snapshot.stdout"
snapshot = read_json(snapshot_stdout) if snapshot_stdout.exists() else {"_parse_error": "missing stdout"}
if snapshot.get("verify_chain_ok") is True:
    gates.append(gate("sidecar_snapshot", "PASS", str(snapshot_stdout), f"events={snapshot.get('event_count')} queue_depth={snapshot.get('queue_depth')}", "PRODUCT_PASS"))
else:
    gates.append(gate("sidecar_snapshot", "FAIL", str(snapshot_stdout), json.dumps(snapshot, sort_keys=True), "PRODUCT_FAIL"))

verify_stdout = out_dir / "sidecar-verify-chain.stdout"
verify = read_json(verify_stdout) if verify_stdout.exists() else {"_parse_error": "missing stdout"}
if verify.get("verify_chain_ok") is True:
    gates.append(gate("sidecar_verify_chain", "PASS", str(verify_stdout), "verify_chain_ok=true", "PRODUCT_PASS"))
else:
    gates.append(gate("sidecar_verify_chain", "FAIL", str(verify_stdout), json.dumps(verify, sort_keys=True), "PRODUCT_FAIL"))

ddf_rc = read_rc("deep-diff-forge-self-test")
ddf_out = read_text(out_dir / "deep-diff-forge-self-test.stdout").strip()
if ddf_rc == 0 and "self-test ok" in ddf_out:
    gates.append(gate("deep_diff_forge_self_test", "PASS", str(out_dir / "deep-diff-forge-self-test.stdout"), ddf_out, "PRODUCT_PASS"))
else:
    gates.append(gate("deep_diff_forge_self_test", "FAIL", str(out_dir / "deep-diff-forge-self-test.stdout"), f"rc={ddf_rc} {ddf_out}", "PRODUCT_FAIL"))

substrate_stdout = out_dir / "factory-substrate-gate-json.stdout"
substrate_rc = read_rc("factory-substrate-gate-json")
substrate = read_json(substrate_stdout) if substrate_stdout.exists() else {"_parse_error": "missing stdout"}
if substrate_rc == 0 and substrate.get("would_block") is False:
    gates.append(gate("factory_substrate_gate", "PASS", str(substrate_stdout), f"regime={substrate.get('regime')} trust_class={substrate.get('trust_class')}", "PRODUCT_PASS"))
elif substrate.get("would_block") is True:
    gates.append(gate("factory_substrate_gate", "ENV_BLOCKED", str(substrate_stdout), f"regime={substrate.get('regime')} trust_class={substrate.get('trust_class')}", "ENV_BLOCKED"))
else:
    gates.append(gate("factory_substrate_gate", "UNKNOWN", str(substrate_stdout), json.dumps(substrate, sort_keys=True), "COVERAGE_GAP"))

loom_rc = read_rc("loom-policy-check")
loom_out = read_text(out_dir / "loom-policy-check.stdout").strip()
if loom_rc == 0 and "LOOM_POLICY_FIXTURES_OK" in loom_out:
    gates.append(gate("loom_policy_check", "PASS", str(out_dir / "loom-policy-check.stdout"), loom_out.replace("\n", "; "), "PRODUCT_PASS"))
else:
    gates.append(gate("loom_policy_check", "FAIL", str(out_dir / "loom-policy-check.stdout"), f"rc={loom_rc} {loom_out}", "PRODUCT_FAIL"))

live_path = os.environ.get("LIVE_PIPE_SUMMARY") or ""
if live_path:
    live = read_json(live_path)
    live_verdict = live.get("verdict")
    if live_verdict == "PASS":
        gates.append(gate("live_pipe_proof_latest", "PASS", live_path, live.get("reason", ""), "PRODUCT_PASS"))
    elif live_verdict == "ENV_BLOCKED":
        gates.append(gate("live_pipe_proof_latest", "ENV_BLOCKED", live_path, live.get("reason", ""), "ENV_BLOCKED"))
    else:
        gates.append(gate("live_pipe_proof_latest", "FAIL", live_path, json.dumps(live, sort_keys=True), "PRODUCT_FAIL"))
else:
    gates.append(gate("live_pipe_proof_latest", "COVERAGE_GAP", "", "no live-pipe proof summary found", "COVERAGE_GAP"))

random_path = os.environ.get("RANDOM_SUMMARY") or ""
if random_path:
    random_doc = read_json(random_path)
    overall = random_doc.get("overall")
    if overall == "PASS":
        gates.append(gate("randomized_ultrastress_latest", "PASS", random_path, "overall=PASS", "PRODUCT_PASS"))
    elif overall == "PASS_WITH_DEGRADED":
        gates.append(gate("randomized_ultrastress_latest", "DEGRADED", random_path, "overall=PASS_WITH_DEGRADED", "PRODUCT_DEGRADED"))
    elif overall == "FAIL":
        gates.append(gate("randomized_ultrastress_latest", "FAIL", random_path, "overall=FAIL", "PRODUCT_FAIL"))
    else:
        gates.append(gate("randomized_ultrastress_latest", "UNKNOWN", random_path, json.dumps(random_doc, sort_keys=True)[:900], "COVERAGE_GAP"))
else:
    gates.append(gate("randomized_ultrastress_latest", "COVERAGE_GAP", "", "no randomized ultrastress summary found", "COVERAGE_GAP"))

monitor_path = os.environ.get("MONITOR_JSONL") or ""
if monitor_path:
    counts = {}
    harness_fail = 0
    sidecar_fail = 0
    try:
        with open(monitor_path) as f:
            for line in f:
                if not line.strip():
                    continue
                row = json.loads(line)
                key = f"{row.get('check')}:{row.get('status')}"
                counts[key] = counts.get(key, 0) + 1
                if row.get("status") == "HARNESS_FAIL":
                    harness_fail += 1
                if row.get("check") in ("sidecar_snapshot", "verify_chain") and row.get("status") == "FAIL":
                    sidecar_fail += 1
        if harness_fail:
            gates.append(gate("passive_monitor_latest", "FAIL", monitor_path, f"harness_fail={harness_fail} counts={counts}", "HARNESS_FAIL"))
        elif sidecar_fail:
            gates.append(gate("passive_monitor_latest", "FAIL", monitor_path, f"sidecar_fail={sidecar_fail} counts={counts}", "PRODUCT_FAIL"))
        else:
            gates.append(gate("passive_monitor_latest", "PASS", monitor_path, f"counts={counts}", "PRODUCT_PASS"))
    except Exception as exc:
        gates.append(gate("passive_monitor_latest", "FAIL", monitor_path, f"jsonl_parse_error={exc}", "HARNESS_FAIL"))
else:
    gates.append(gate("passive_monitor_latest", "COVERAGE_GAP", "", "no passive monitor jsonl found", "COVERAGE_GAP"))

deep_trace_pipe = os.environ.get("DEEP_TRACE_PIPE_TSV") or ""
if deep_trace_pipe:
    counts = {}
    max_latency = 0
    total = 0
    try:
        with open(deep_trace_pipe) as f:
            header = next(f, None)
            for line in f:
                parts = line.rstrip("\n").split("\t")
                if len(parts) < 6:
                    continue
                status = parts[3]
                counts[status] = counts.get(status, 0) + 1
                total += 1
                try:
                    max_latency = max(max_latency, int(parts[4]))
                except ValueError:
                    pass
        invalid = sorted(k for k in counts if k not in ("ACK_DURABLE", "NACK_SCHEMA"))
        if total == 0:
            gates.append(gate("deep_trace_transport_latest", "COVERAGE_GAP", deep_trace_pipe, "no pipe rows", "COVERAGE_GAP"))
        elif invalid:
            gates.append(gate("deep_trace_transport_latest", "DEGRADED", deep_trace_pipe, f"unexpected_statuses={invalid} counts={counts} max_latency_ms={max_latency}", "PRODUCT_DEGRADED"))
        elif max_latency > 1000:
            gates.append(gate("deep_trace_transport_latest", "DEGRADED", deep_trace_pipe, f"counts={counts} max_latency_ms={max_latency}", "PRODUCT_DEGRADED"))
        else:
            gates.append(gate("deep_trace_transport_latest", "PASS", deep_trace_pipe, f"counts={counts} max_latency_ms={max_latency}", "PRODUCT_PASS"))
    except Exception as exc:
        gates.append(gate("deep_trace_transport_latest", "FAIL", deep_trace_pipe, f"parse_error={exc}", "HARNESS_FAIL"))
else:
    gates.append(gate("deep_trace_transport_latest", "COVERAGE_GAP", "", "no deep trace pipe-latency.tsv found", "COVERAGE_GAP"))

status_order = [g["status"] for g in gates]
if any(s == "FAIL" for s in status_order):
    verdict = "FAIL"
elif any(s == "ENV_BLOCKED" for s in status_order):
    verdict = "ENV_BLOCKED"
elif any(s == "GOVERNANCE_BLOCKED" for s in status_order):
    verdict = "GOVERNANCE_BLOCKED"
elif any(s in ("COVERAGE_GAP", "UNKNOWN") for s in status_order):
    verdict = "COVERAGE_GAP"
elif any(s == "DEGRADED" for s in status_order):
    verdict = "PASS_WITH_DEGRADED"
else:
    verdict = "PASS"

doc = {
    "schema": "habitat.kernel.v012.zero_touch_verify.v1",
    "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "stamp": os.environ["STAMP"],
    "mode": "zero_touch_read_only",
    "root": str(root),
    "out_dir": str(out_dir),
    "verdict": verdict,
    "gates": gates,
    "zero_touch_constraints": {
        "no_promotion": True,
        "no_rollback": True,
        "no_service_restart": True,
        "no_arming": True,
        "no_active_zellij_session_created_by_default": True,
    },
}

with open(os.environ["SUMMARY_JSON"], "w") as f:
    json.dump(doc, f, indent=2, sort_keys=True)
    f.write("\n")

with open(os.environ["SUMMARY_MD"], "w") as f:
    f.write("# Orchestrator Kernel v0.1.2 Zero-Touch Verification\n\n")
    f.write(f"- Stamp: `{os.environ['STAMP']}`\n")
    f.write(f"- Verdict: `{verdict}`\n")
    f.write(f"- JSON: `{os.environ['SUMMARY_JSON']}`\n")
    f.write("- Mode: `zero_touch_read_only`\n\n")
    f.write("## Gates\n\n")
    f.write("| Gate | Status | Class | Evidence | Detail |\n")
    f.write("|---|---|---|---|---|\n")
    for g in gates:
        detail = str(g["detail"]).replace("|", "\\|").replace("\n", " ")[:500]
        f.write(f"| `{g['name']}` | `{g['status']}` | `{g['class']}` | `{g['evidence']}` | {detail} |\n")
PY

printf 'ZERO_TOUCH_VERIFY_JSON=%s\nZERO_TOUCH_VERIFY_SUMMARY=%s\n' "${SUMMARY_JSON}" "${SUMMARY_MD}"
