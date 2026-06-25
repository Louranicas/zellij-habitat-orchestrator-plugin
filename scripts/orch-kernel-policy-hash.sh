#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
POLICY="${1:-${ROOT}/config/zellij-orchestrator-kernel-warrants.v2.json}"

python3 - "${POLICY}" <<'PY'
import hashlib
import json
import sys

path = sys.argv[1]
with open(path) as f:
    policy = json.load(f)

stored = policy.get("policy_hash")
canonical_policy = dict(policy)
canonical_policy["policy_hash"] = None
canonical = json.dumps(canonical_policy, sort_keys=True, separators=(",", ":")).encode()
expected = "sha256:" + hashlib.sha256(canonical).hexdigest()
status = "PASS" if stored == expected else "FAIL"

print(json.dumps({
    "schema": "habitat.kernel.policy_hash.v1",
    "policy_path": path,
    "stored_policy_hash": stored,
    "expected_policy_hash": expected,
    "hash_rule": "sha256(canonical_json(policy with policy_hash=null))",
    "status": status,
}, indent=2, sort_keys=True))

raise SystemExit(0 if status == "PASS" else 1)
PY
