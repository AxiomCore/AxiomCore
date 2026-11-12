"""Writes an axiom-auth-policy-approvals/v1 file approving every widening item of an
`axiom auth policy check --json` output ({"report": axiom-auth-impact/v1, "check": {...}})
or of a bare axiom-auth-impact/v1 report (`--report <file>`).
Usage: approve.py <check or report json> <approvals out>"""
import json
import sys

document = json.load(open(sys.argv[1]))
report = document.get("report", document)
if report.get("format") != "axiom-auth-impact/v1":
    raise SystemExit("not an axiom-auth-impact/v1 report")
# Approval ids as the CLI computes them: policy_changes[].id, and access:<endpoint>:<subject>:<direction>.
ids = [i["id"] for i in report.get("policy_changes", []) if i.get("direction") == "widening"]
ids += [f"access:{a['endpoint']}:{a['subject']}:{a['direction']}" for a in report.get("access", []) if a.get("direction") == "widening"]
approvals = {"format": "axiom-auth-policy-approvals/v1", "policy_after_sha256": report["policy_after_sha256"],
             "plan_after_sha256": report.get("plan_after_sha256"),
             "approved": [{"id": i, "reason": "E5 example: members may write orders"} for i in sorted(ids)]}
json.dump(approvals, open(sys.argv[2], "w"), sort_keys=True, indent=2)
print(f"approved {len(ids)} widening items")
