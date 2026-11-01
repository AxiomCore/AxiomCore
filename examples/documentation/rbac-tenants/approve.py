"""Writes auth/policy.approvals.json approving every widening item of an axiom-auth-impact/v1 report."""
import json
import sys

report = json.load(open(sys.argv[1]))
items = [i for i in report.get("items", []) if i.get("impact") == "widening"]
approvals = {"format": "axiom-auth-policy-approvals/v1", "policy_after_sha256": report["policy_after_sha256"],
             "plan_after_sha256": report.get("plan_after_sha256"), "approved": [{"id": i["id"], "reason": "E5 example: members may write orders"} for i in items]}
json.dump(approvals, open(sys.argv[2], "w"), sort_keys=True, indent=2)
print(f"approved {len(items)} widening items")
