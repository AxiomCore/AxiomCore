#!/usr/bin/env python3
"""Group one release train's local, unpublished evidence on the release SSD."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys

import ctl
import flow
import versions


def train_paths(root: Path, train_id: str) -> dict[str, Path]:
    directory = root / "trains" / train_id
    if root.resolve() not in directory.resolve().parents:
        raise ctl.ReleaseError("train path escapes release build root")
    return {"directory": directory, "preparation": directory / "preparation.json",
            "plan": directory / "plan.json", "notes": directory / "notes.md",
            "staged": directory / "staged.json", "gate": directory / "gate.json",
            "published": directory / "published.json"}


def current(catalog: dict, workspace: Path, root: Path,
            intent_path: Path = ctl.CONTROL_DIR / "intent.json",
            versions_path: Path = versions.VERSIONS) -> tuple[dict, dict[str, Path]]:
    ledger = versions.read_versions(versions_path, catalog)
    intent = flow.read_intent(intent_path, catalog, ledger)
    return intent, train_paths(root, intent["trainId"])


def make_current_plan(catalog: dict, workspace: Path, root: Path,
                      intent_path: Path = ctl.CONTROL_DIR / "intent.json",
                      versions_path: Path = versions.VERSIONS) -> tuple[dict, Path]:
    intent, paths = current(catalog, workspace, root, intent_path, versions_path)
    plan = ctl.make_plan(catalog, workspace, only={change["component"] for change in intent["changes"]})
    if plan["blocked"] or plan["blockedVersions"]:
        raise ctl.ReleaseError("train plan has dirty or version-blocked inputs; commit reviewed changes first")
    intended = {change["component"] for change in intent["changes"]}
    selected = {item["id"] for item in plan["components"] if item["selected"]}
    if selected - intended:
        raise ctl.ReleaseError("selected dependencies need intent entries or an authenticated reusable baseline: "
                               + ", ".join(sorted(selected - intended)))
    ctl.write_json(paths["plan"], plan)
    return plan, paths["plan"]


def status(catalog: dict, workspace: Path, root: Path,
           intent_path: Path = ctl.CONTROL_DIR / "intent.json",
           versions_path: Path = versions.VERSIONS) -> dict:
    intent, paths = current(catalog, workspace, root, intent_path, versions_path)
    plan = json.loads(paths["plan"].read_text()) if paths["plan"].exists() else None
    entries = []
    if plan:
        entries = [{"component": item["id"], "disposition": "BUILD" if item["selected"] else "REUSE",
                    "version": next((change.get("version") for change in intent["changes"]
                                     if change["component"] == item["id"]), item.get("version"))}
                   for item in plan["components"]]
    return {"trainId": intent["trainId"], "intent": str(intent_path),
            "versionLedger": str(versions_path), "evidenceRoot": str(paths["directory"]),
            "storageAvailable": root.is_dir(),
            "evidence": {name: {"path": str(path), "exists": path.exists()}
                         for name, path in paths.items() if name != "directory"},
            "components": entries, "publicationStatus": "not-proven-published"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=ctl.CATALOG)
    parser.add_argument("--workspace", type=Path, default=ctl.WORKSPACE)
    parser.add_argument("--intent", type=Path, default=ctl.CONTROL_DIR / "intent.json")
    parser.add_argument("--versions", type=Path, default=versions.VERSIONS)
    parser.add_argument("command", choices=("plan", "status"))
    args = parser.parse_args()
    try:
        catalog = ctl.read_catalog(args.catalog)
        if args.command == "plan":
            root = ctl.require_external_build_root()
            plan, path = make_current_plan(catalog, args.workspace.resolve(), root,
                                           args.intent, args.versions)
            print(f"Planned {len(plan['components'])} component(s): {path}; nothing published")
        else:
            root = Path(os.environ.get("AXIOM_RELEASE_BUILD_ROOT", str(ctl.DEFAULT_BUILD_ROOT))).resolve()
            print(json.dumps(status(catalog, args.workspace.resolve(), root,
                                    args.intent, args.versions), indent=2))
        return 0
    except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"release-train: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
