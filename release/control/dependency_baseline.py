"""Authenticate published dependency receipts for a later consumer candidate.

A source pin that needs a published checksum cannot be prepared in the same
immutable source snapshot as its producer. The later candidate may reuse the
producer's exact receipt, but only after remote publication was recorded and
the current source fingerprint still matches that receipt.
"""

from __future__ import annotations

import json
from pathlib import Path

import ctl
import cycle


def receipt_for(root: Path, entry: dict, source_heads: dict) -> Path:
    saved = entry.get("reusedReceiptPath") if entry.get("reuseCandidate") else None
    if saved:
        path = Path(saved)
        if not path.is_absolute() or root.resolve() not in path.resolve().parents:
            raise ctl.ReleaseError("reused dependency receipt escapes the release volume")
        return path
    return ctl.artifact_receipt_path(root, entry, source_heads)


def published_dependency_baseline(root: Path, catalog: dict, workspace: Path,
                                  requested: set[str], active: set[str]) -> dict:
    """Return only dependencies proven published, never a merely staged build."""
    definitions = {item["id"]: item for item in catalog["components"]}
    needed: set[str] = set()
    pending = list(requested)
    while pending:
        for dependency in definitions[pending.pop()].get("depends_on", []):
            if dependency not in needed:
                needed.add(dependency)
                pending.append(dependency)
    needed -= active
    evidence = cycle.published_evidence(root, catalog)
    baseline = {}
    for component_id in sorted(needed):
        proof = evidence.get(component_id)
        if proof is None:
            continue
        directory = root / "trains" / proof["trainId"] / "components" / component_id
        plan_path = directory / "plan.json"
        stage_path = directory / "staged.json"
        if any(not path.is_file() or path.is_symlink() for path in (plan_path, stage_path)):
            raise ctl.ReleaseError(f"published {component_id} has incomplete candidate evidence")
        plan = json.loads(plan_path.read_text())
        stage = json.loads(stage_path.read_text())
        if plan.get("catalogSha256") != catalog["sha256"] or stage.get("catalogSha256") != catalog["sha256"]:
            raise ctl.ReleaseError(f"published {component_id} uses a different release catalog")
        entry = next((item for item in plan["components"] if item["id"] == component_id), None)
        staged = next((item for item in stage["components"] if item["id"] == component_id), None)
        if not entry or not staged or entry["fingerprint"] != staged["fingerprint"]:
            raise ctl.ReleaseError(f"published {component_id} has inconsistent build evidence")
        path = ctl.artifact_receipt_path(root, entry, plan["repositories"])
        receipt = ctl.verify_receipt(path, entry["fingerprint"])
        actual = {(item["file"], item["sha256"]) for item in receipt["artifacts"]}
        expected = {(item["file"], item["sha256"]) for item in staged["artifacts"]}
        if actual != expected or receipt["component"] != component_id:
            raise ctl.ReleaseError(f"published {component_id} receipt differs from staged bytes")
        baseline[component_id] = {"fingerprint": entry["fingerprint"],
                                  "version": staged.get("version"),
                                  "artifacts": receipt["artifacts"],
                                  "receiptPath": str(path)}
    return baseline
