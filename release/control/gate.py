#!/usr/bin/env python3
"""Fail-closed release readiness checks across intent, source, notes and bytes.

This is a handoff gate, not a publisher. Passing it means a reviewed,
reproducible candidate can be sent to the owning publication adapter; it does
not mean any remote asset or deployment exists.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys

import ctl
import flow
import versions


GATE_FORMAT = "axiom-platform-release-gate/v1"
HOST_IDS = {"ui-host-web", "ui-host-android", "ui-host-ios"}


def inspect_candidate(intent: dict, plan: dict, catalog: dict, workspace: Path,
                      stage: dict | None = None, receipt_paths: list[Path] | None = None) -> dict:
    blockers: list[str] = []
    if plan.get("format") != ctl.PLAN_FORMAT or plan.get("catalogSha256") != catalog["sha256"]:
        raise ctl.ReleaseError("candidate plan is missing, stale, or uses a different catalog")
    planned = {item["id"]: item for item in plan.get("components", [])}
    selected = {component_id for component_id, item in planned.items() if item["selected"]}
    changes = {item["component"]: item for item in intent["changes"]}
    for component_id in sorted(selected - changes.keys()):
        blockers.append(f"selected component has no release intent: {component_id}")
    for component_id in sorted(changes.keys() - selected):
        blockers.append(f"release intent is not selected by the plan: {component_id}")
    for component_id in sorted(selected & changes.keys()):
        planned_version = planned[component_id].get("version")
        if planned_version is not None and changes[component_id].get("version") != planned_version:
            blockers.append(f"intent version differs from planned source version: {component_id}")
    if selected & HOST_IDS and not HOST_IDS <= planned.keys():
        blockers.append("a UI Host publication requires web, Android, and iOS in the plan; unchanged targets may reuse verified bytes")
    try:
        ctl.require_current_plan(plan, catalog, workspace)
    except ctl.ReleaseError as error:
        blockers.append(f"pinned source gate: {error}")
    try:
        ctl.release_notes(plan, catalog, workspace, enforce=True)
    except (ctl.ReleaseError, KeyError, ValueError) as error:
        blockers.append(f"release notes gate: {error}")
    for component_id in sorted(selected & changes.keys()):
        owner_name = planned[component_id]["owner"]
        owner = ctl.repo_path(catalog, owner_name, workspace)
        fragment = owner / "release-notes" / "unreleased" / f"{intent['trainId']}-{component_id}.json"
        if not fragment.is_file():
            blockers.append(f"release intent has no matching owned fragment: {component_id}")
            continue
        try:
            noted = json.loads(fragment.read_text())
        except (OSError, ValueError):
            blockers.append(f"release intent fragment is unreadable: {component_id}")
            continue
        expected = {key: changes[component_id][key] for key in ("component", "type", "summary", "migration")
                    if key in changes[component_id]}
        if noted != expected:
            blockers.append(f"committed release-note fragment differs from intent: {component_id}")
    if "sdk-flutter" in selected and "runtime-apple" in selected:
        framework = changes.get("runtime-apple", {}).get("version")
        pinned = changes.get("sdk-flutter", {}).get("runtimeVersion")
        if framework and pinned != framework:
            blockers.append("Flutter must explicitly pin the selected Apple framework release via runtimeVersion")
    if "sdk-flutter" in selected:
        owner = ctl.repo_path(catalog, "axiom-sdk", workspace)
        pins = set()
        for platform_name in ("ios", "macos"):
            podspec = owner / f"flutter/axiom_flutter/{platform_name}/axiom_flutter.podspec"
            match = re.search(r"^\s*runtime_version\s*=\s*'([^']+)'", podspec.read_text(), re.MULTILINE)
            if not match:
                blockers.append(f"Flutter {platform_name} podspec lacks an independent runtime pin")
            else:
                pins.add(match.group(1))
        if len(pins) > 1:
            blockers.append("Flutter iOS and macOS podspecs pin different Apple runtime versions")
        requested = changes.get("sdk-flutter", {}).get("runtimeVersion")
        if requested and pins != {requested}:
            blockers.append("Flutter podspec runtime pin differs from release intent")
    if "sdk-atmx-react" in selected and "sdk-atmx-web" in selected:
        web_version = changes.get("sdk-atmx-web", {}).get("version")
        react_path = ctl.repo_path(catalog, "atmx-react", workspace) / "package.json"
        if web_version and react_path.is_file():
            pinned = json.loads(react_path.read_text()).get("dependencies", {}).get("atmx-web")
            if pinned != f"^{web_version}":
                blockers.append(f"React must pin the selected atmx-web release ^{web_version} and regenerate its lockfile")

    receipts = {}
    for path in receipt_paths or []:
        try:
            receipt = ctl.verify_receipt(path)
        except (ctl.ReleaseError, OSError, ValueError, KeyError) as error:
            blockers.append(f"receipt {path}: {error}")
            continue
        component_id = receipt.get("component")
        if component_id in receipts:
            blockers.append(f"duplicate receipt for {component_id}")
        else:
            receipts[component_id] = receipt
    if stage is not None:
        if (stage.get("format") != "axiom-platform-release-manifest/v1"
                or stage.get("status") != "staged-not-published"
                or stage.get("trainId") != intent["trainId"]
                or stage.get("catalogSha256") != catalog["sha256"]):
            blockers.append("stage format, status, train, or catalog differs from this release intent")
        if stage.get("intentSha256") != ctl.sha256(ctl.canonical(intent)):
            blockers.append("stage was not bound to this release intent")
        if stage.get("repositories") != plan.get("repositories"):
            blockers.append("staged source commits differ from the plan")
        staged = {item.get("id"): item for item in stage.get("components", [])}
        if len(staged) != len(stage.get("components", [])) or set(staged) != set(planned):
            blockers.append("staged component set differs from the plan")
        for component_id, item in planned.items():
            result = staged.get(component_id)
            if result is None:
                continue
            if result.get("fingerprint") != item["fingerprint"]:
                blockers.append(f"staged fingerprint differs from plan: {component_id}")
            if result.get("version") != item.get("version"):
                blockers.append(f"staged version differs from plan: {component_id}")
            intended_version = changes.get(component_id, {}).get("version")
            if component_id in HOST_IDS and selected & HOST_IDS:
                intended_version = next((changes[host]["version"] for host in HOST_IDS if host in changes), None)
            if intended_version and result.get("releaseVersion") != intended_version:
                blockers.append(f"staged release version differs from intent: {component_id}")
            receipt = receipts.get(component_id)
            if not receipt:
                blockers.append(f"verified artifact receipt missing: {component_id}")
                continue
            if receipt.get("fingerprint") != item["fingerprint"]:
                blockers.append(f"receipt fingerprint differs from plan: {component_id}")
            staged_artifacts = {(asset.get("file"), asset.get("sha256"))
                                for asset in result.get("artifacts", [])}
            receipt_artifacts = {(asset.get("file"), asset.get("sha256"))
                                 for asset in receipt.get("artifacts", [])}
            if staged_artifacts != receipt_artifacts:
                blockers.append(f"staged artifact differs from verified receipt: {component_id}")
        for component_id in sorted(receipts.keys() - planned.keys()):
            blockers.append(f"receipt component is outside the plan: {component_id}")
    elif receipts:
        blockers.append("receipts were supplied without a staged manifest")

    components = [{"id": item["id"], "selected": item["selected"],
                   "adapter": item["adapter"], "destination": item["destination"],
                   "version": changes.get(item["id"], {}).get("version", item.get("version")),
                   "build": "local-adapter" if item["adapter"] != "ci-only" else "owning-CI-adapter",
                   "publication": "not-run", "publisherAdapter": "not-migrated"}
                  for item in plan["components"]]
    return {"format": GATE_FORMAT, "trainId": intent["trainId"],
            "intentSha256": ctl.sha256(ctl.canonical(intent)),
            "planSha256": ctl.sha256(ctl.canonical(plan)),
            "stageSha256": ctl.sha256(ctl.canonical(stage)) if stage is not None else None,
            "phase": "candidate-ready-for-publisher" if stage is not None and not blockers else
                     "ready-to-build" if not blockers else "blocked",
            "components": components, "blockers": blockers,
            "publicationStatus": "not-published", "productionReady": False,
            "remainingForProduction": ["migrate selected publisher adapters to consume verified receipts",
                                       "verify remote immutable bytes and deployment health",
                                       "issue a trusted signed published baseline"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=ctl.CATALOG)
    parser.add_argument("--workspace", type=Path, default=ctl.WORKSPACE)
    parser.add_argument("--intent", type=Path, default=ctl.CONTROL_DIR / "intent.json")
    parser.add_argument("--versions", type=Path, default=versions.VERSIONS)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--stage", type=Path)
    parser.add_argument("--receipt", type=Path, action="append", default=[])
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    try:
        catalog = ctl.read_catalog(args.catalog)
        ledger = versions.read_versions(args.versions, catalog)
        intent = flow.read_intent(args.intent, catalog, ledger)
        plan = json.loads(args.plan.read_text())
        stage = json.loads(args.stage.read_text()) if args.stage else None
        report = inspect_candidate(intent, plan, catalog, args.workspace.resolve(), stage, args.receipt)
        if args.out:
            ctl.write_json(args.out, report)
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0 if not report["blockers"] else 2
    except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"release-gate: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
