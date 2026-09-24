#!/usr/bin/env python3
"""Small, fail-closed operator interface behind `just release`."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys

import ctl
import flow
import gate
import review
import scan
import train
import versions


HOST_IDS = {"ui-host-web", "ui-host-android", "ui-host-ios"}
HELP = """AxiomCore release commands:
  just release                         Show changed builds and the next safe step
  just release version COMPONENT X.Y.Z Set one candidate version (or ui-host group)
  just release prepare                 Apply reviewed active-wave versions and notes, with SSD backups
  just release review                  Open arrow-key worktree review and commit selected files
  just release COMPONENT               Build and verify a local candidate (cli or ui-host)
  just release publish COMPONENT       Fail closed until a verified publisher is available
  just release test                    Run release-control tests

The active and queued changes are in release/control/intent.json. Candidate
versions are in release/control/versions.json. Nothing is published by prepare
or candidate. A production publisher is not yet installed.
"""


def load() -> tuple[dict, dict, dict]:
    catalog = ctl.read_catalog()
    ledger = versions.read_versions(versions.VERSIONS, catalog)
    intent = flow.read_intent(ctl.CONTROL_DIR / "intent.json", catalog, ledger)
    return catalog, ledger, intent


def status(catalog: dict, ledger: dict, intent: dict) -> None:
    report = scan.inventory(catalog, ctl.WORKSPACE)
    active = {change["component"] for change in intent["changes"]}
    queued = {change["component"] for change in intent.get("queued", [])}
    affected = set(report["affected"])
    print(f"Train {intent['trainId']} · active wave {intent.get('wave', 'default')}")
    print(f"Changed build inputs: {len(affected)} components (relative to local upstream, not published baseline)")
    for component_id in sorted(affected | active | queued):
        state = "active" if component_id in active else "queued" if component_id in queued else "uncovered"
        candidate = ledger["components"][component_id]["candidateVersion"]
        print(f"  {component_id:22} {state:9} candidate={candidate or 'digest/none'}")
    if affected - active - queued:
        print("Action: account for uncovered components in intent.json before preparing.")
    root = Path(os.environ.get("AXIOM_RELEASE_BUILD_ROOT", str(ctl.DEFAULT_BUILD_ROOT))).resolve()
    evidence = train.status(catalog, ctl.WORKSPACE, root)
    print(f"Evidence: {evidence['evidenceRoot']} ({'available' if evidence['storageAvailable'] else 'SSD not mounted'})")
    if active:
        try:
            prepared, edits, fragments = flow.make_preparation(intent, catalog, ctl.WORKSPACE)
            if prepared["blocked"]:
                print("Next: resolve dirty version files listed by `just release prepare`.")
            elif edits or fragments:
                print(f"Next: `just release prepare` will update {len(edits)} version file(s) and create {len(fragments)} note(s).")
            else:
                print("Next: `just release review` to inspect and commit source changes, then run candidate preflight.")
        except (ctl.ReleaseError, OSError, ValueError) as error:
            print(f"Preparation needs attention: {error}")
    print("Production publish: unavailable until publisher adapters verify remote bytes.")


def upstream_state(repository: Path) -> tuple[str | None, int]:
    try:
        upstream = ctl.run("git", "rev-parse", "--abbrev-ref", "--symbolic-full-name",
                           "@{upstream}", cwd=repository).decode().strip()
        ahead = int(ctl.run("git", "rev-list", "--count", f"{upstream}..HEAD",
                            cwd=repository).decode().strip())
        return upstream, ahead
    except (ctl.ReleaseError, ValueError):
        return None, 0


def prepare(catalog: dict, intent: dict) -> None:
    report, edits, fragments = flow.make_preparation(intent, catalog, ctl.WORKSPACE)
    if report["blocked"]:
        raise ctl.ReleaseError("preparation blocked: " + "; ".join(report["blocked"]))
    if not edits and not fragments:
        root = Path(os.environ.get("AXIOM_RELEASE_BUILD_ROOT", str(ctl.DEFAULT_BUILD_ROOT))).resolve()
        evidence = train.train_paths(root, intent["trainId"])["preparation"]
        if evidence.is_file():
            previous = json.loads(evidence.read_text())
            if previous.get("intentSha256") != report["intentSha256"] or not previous.get("applied"):
                raise ctl.ReleaseError(f"existing preparation evidence does not match the active intent: {evidence}; use a new train ID")
            print(f"Active wave is prepared. Evidence: {evidence}")
            review_and_continue(catalog, intent)
        else:
            print(f"Active wave mirrors and notes exist, but preparation evidence is missing: {evidence}; investigate before building.")
        return
    root = ctl.mount_default_build_root()
    paths = train.train_paths(root, intent["trainId"])
    if paths["preparation"].exists():
        raise ctl.ReleaseError(f"preparation evidence already exists: {paths['preparation']}; do not overwrite it")
    backup = flow.apply_preparation(report, edits, fragments, catalog, ctl.WORKSPACE)
    report["applied"] = True
    report["backup"] = str(backup)
    ctl.write_json(paths["preparation"], report)
    print(f"Prepared {len(edits)} version file(s) and {len(fragments)} note(s). Backup: {backup}")
    review_and_continue(catalog, intent)


def review_and_continue(catalog: dict, intent: dict) -> None:
    decision = review.run(catalog, intent["trainId"])
    if decision == "proceed":
        candidate_preflight(catalog, intent)
    elif decision == "exit":
        print("Review exited. No build or publication started.")


def candidate_preflight(catalog: dict, intent: dict) -> None:
    root = Path(os.environ.get("AXIOM_RELEASE_BUILD_ROOT", str(ctl.DEFAULT_BUILD_ROOT))).resolve()
    evidence = train.train_paths(root, intent["trainId"])["preparation"]
    if not evidence.is_file():
        raise ctl.ReleaseError(f"release preflight needs preparation evidence: {evidence}; run `just release prepare`")
    previous = json.loads(evidence.read_text())
    if not previous.get("applied") or previous.get("intentSha256") != ctl.sha256(ctl.canonical(intent)):
        raise ctl.ReleaseError(f"release preflight evidence does not match the active intent: {evidence}")
    active = {change["component"] for change in intent["changes"]}
    plan = ctl.make_plan(catalog, ctl.WORKSPACE, only=active)
    if plan["blocked"] or plan["blockedVersions"]:
        raise ctl.ReleaseError("release preflight is blocked by uncommitted inputs or reused versions: "
                               + ", ".join([*plan["blocked"], *plan["blockedVersions"]]))
    selected = {item["id"] for item in plan["components"] if item["selected"]}
    unscoped = selected - active
    if unscoped:
        raise ctl.ReleaseError("release preflight selected dependencies outside the active wave: "
                               + ", ".join(sorted(unscoped)))
    local = sorted(item["id"] for item in plan["components"]
                   if item["selected"] and item["adapter"] != "ci-only")
    ci_only = sorted(item["id"] for item in plan["components"]
                     if item["selected"] and item["adapter"] == "ci-only")
    print(f"Local candidate preflight passed for train {intent['trainId']}: {len(selected)} selected component(s).")
    print("Local candidate builders: " + (", ".join(local) if local else "none"))
    print("CI builder gap: " + (", ".join(ci_only) if ci_only else "none"))
    ahead_repositories = []
    missing_upstreams = []
    for name in plan.get("repositories", {}):
        repository = ctl.repo_path(catalog, name, ctl.WORKSPACE)
        upstream, ahead = upstream_state(repository)
        if upstream is None:
            missing_upstreams.append(name)
        elif ahead:
            ahead_repositories.append(f"{name} ({ahead} ahead of {upstream})")
    if ahead_repositories:
        print("Remote/CI gap — push reviewed source commits: " + ", ".join(sorted(ahead_repositories)))
    if missing_upstreams:
        print("Remote/CI gap — configure tracked upstreams: " + ", ".join(sorted(missing_upstreams)))
    print("No candidate was built or published. Production publisher adapters are still unavailable.")


def candidate(catalog: dict, intent: dict, component: str) -> None:
    requested = HOST_IDS if component == "ui-host" else {component}
    active = {change["component"] for change in intent["changes"]}
    if not requested <= active:
        raise ctl.ReleaseError("candidate must be in the active wave; queued or unknown: "
                               + ", ".join(sorted(requested - active)))
    plan = ctl.make_plan(catalog, ctl.WORKSPACE, only=requested)
    selected = {item["id"] for item in plan["components"] if item["selected"]}
    if not selected <= active:
        raise ctl.ReleaseError("selected dependencies are not active: " + ", ".join(sorted(selected - active)))
    if plan["blocked"] or plan["blockedVersions"]:
        raise ctl.ReleaseError("commit changed build inputs and use a new published version before candidate build")
    ci_only = sorted(item["id"] for item in plan["components"] if item["adapter"] == "ci-only")
    if ci_only:
        raise ctl.ReleaseError("no verified local build adapter for " + ", ".join(ci_only)
                               + "; selective CI builder and publisher migration is required")
    scoped = {**intent, "changes": [change for change in intent["changes"]
                                     if change["component"] in selected], "queued": []}
    ready = gate.inspect_candidate(scoped, plan, catalog, ctl.WORKSPACE)
    if ready["blockers"]:
        raise ctl.ReleaseError("candidate gate blocked: " + "; ".join(ready["blockers"]))
    root = ctl.mount_default_build_root()
    directory = train.train_paths(root, intent["trainId"])["directory"] / "components" / component
    if directory.exists():
        raise ctl.ReleaseError(f"component candidate already exists: {directory}; evidence is immutable")
    ctl.write_json(directory / "intent.json", scoped)
    ctl.write_json(directory / "plan.json", plan)
    receipts = []
    for entry in plan["components"]:
        ctl.build_component(directory / "plan.json", entry["id"], catalog, ctl.WORKSPACE)
        receipt = root / "artifacts" / entry["id"] / "receipt.json"
        ctl.verify_receipt(receipt)
        receipts.append(receipt)
    stage = ctl.stage_manifest(plan, catalog, ctl.WORKSPACE, intent["trainId"],
                               receipts, directory / "staged.json", scoped)
    result = gate.inspect_candidate(scoped, plan, catalog, ctl.WORKSPACE, stage, receipts)
    ctl.write_json(directory / "gate.json", result)
    notes = ctl.release_notes(plan, catalog, ctl.WORKSPACE, enforce=True, train_id=intent["trainId"])
    ctl.write_new_text(directory / "notes.md", notes)
    if result["blockers"]:
        raise ctl.ReleaseError("staged candidate failed final gate: " + "; ".join(result["blockers"]))
    print(f"Verified candidate for {component}: {directory}")
    print("Nothing published. The production publisher must consume these exact receipts and verify remote bytes.")


def main() -> int:
    action = sys.argv[1] if len(sys.argv) > 1 and sys.argv[1] else "status"
    component = sys.argv[2] if len(sys.argv) > 2 else ""
    version = sys.argv[3] if len(sys.argv) > 3 else ""
    if action in ("help", "--help", "-h"):
        print(HELP)
        return 0
    if action == "test":
        return subprocess.run([sys.executable, "-B", "-m", "unittest", "discover",
                               "-s", str(ctl.CONTROL_DIR), "-p", "test_*.py", "-v"],
                              check=False).returncode
    try:
        catalog, ledger, intent = load()
        if action == "status":
            status(catalog, ledger, intent)
        elif action == "version":
            if not component or not version:
                raise ctl.ReleaseError("usage: just release version COMPONENT X.Y.Z")
            versions.set_candidate(versions.VERSIONS, catalog, component, version, replace=True)
            print(f"Candidate version set for {component}: {version}; review versions.json before preparing.")
        elif action == "prepare":
            prepare(catalog, intent)
        elif action == "review":
            review_and_continue(catalog, intent)
        elif action == "candidate":
            if not component:
                raise ctl.ReleaseError("usage: just release candidate COMPONENT")
            candidate(catalog, intent, component)
        elif action in {item["id"] for item in catalog["components"]} | {"ui-host"}:
            candidate(catalog, intent, action)
        elif action in ("publish", "deploy"):
            if not component:
                raise ctl.ReleaseError("usage: just release publish COMPONENT")
            known = {item["id"] for item in catalog["components"]} | {"ui-host"}
            if component not in known:
                raise ctl.ReleaseError(f"unknown component: {component}")
            root = Path(os.environ.get("AXIOM_RELEASE_BUILD_ROOT", str(ctl.DEFAULT_BUILD_ROOT))).resolve()
            staged = train.train_paths(root, intent["trainId"])["directory"] / "components" / component / "staged.json"
            if not staged.is_file():
                raise ctl.ReleaseError(f"no verified staged candidate for {component} at {staged}; run `just release {component}` after source commits")
            raise ctl.ReleaseError("production publisher is not installed for " + component
                                   + "; this staged candidate is not live and must not be marked published")
        else:
            raise ctl.ReleaseError(f"unknown action {action!r}; run `just release help`")
        return 0
    except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"release: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
