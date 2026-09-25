#!/usr/bin/env python3
"""Small, fail-closed operator interface behind `just release`."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

import ctl
import ci_builders
import cycle
import flow
import gate
import progress
import publisher
import publish_menu
import publish_targets
import review
import scan
import train
import versions


HOST_IDS = {"ui-host-web", "ui-host-android", "ui-host-ios"}
HELP = """AxiomCore release commands:
  just release web                     Open the local release dashboard (http://127.0.0.1:8716)
  just release                         Show changed builds and the next safe step
  just release new                     Choose components and create the next release cycle interactively
  just release new restart             Archive an unfinished draft and start a different cycle
  just release update                  Add queued components to a safe successor of the current cycle
  just release update restart          Archive an unfinished update draft and restart its selection
  just release capabilities            Show build and publish readiness for every catalog component
  just release version COMPONENT X.Y.Z Set one candidate version (or ui-host group)
  just release prepare                 Apply reviewed active-wave versions and notes, with SSD backups
  just release review                  Open arrow-key worktree review and commit selected files
  just release ui-host                 Build web, Android, and iOS Host candidates with live progress
  just release cli                     Build a CLI candidate when CLI is active
  just release landing                 Build the Astro landing site on the release SSD
  just release docs                    Build the documentation site on the release SSD
  just release COMPONENT               Build one active local candidate
  just release publish                 Choose staged production targets with arrow keys
  just release publish ui-host [TRAIN] Publish the exact signed, gated Host candidate and verify GitHub bytes
  just release publish landing [TRAIN] Deploy the exact static candidate to Cloudflare Pages
  just release publish docs [TRAIN]    Deploy the exact docs candidate to Cloudflare Pages
  just release publish COMPONENT       Publish a staged component through its verified adapter
  just release test                    Run release-control tests

The interactive new-cycle flow manages train IDs, the active component list,
change types, summaries and candidate versions. Selections and answers are
checkpointed on the release SSD; rerun `just release new` after interruption.
`update` preselects unfinished active work and retains its answers. A prepared
train is never rewritten: updating allocates the next train ID automatically.
You do not need to edit intent.json manually. Prepare and candidate do not
publish. Publish requires exact staged receipts and remote verification.
"""


def load() -> tuple[dict, dict, dict]:
    catalog = ctl.read_catalog()
    ledger = versions.read_versions(versions.VERSIONS, catalog)
    intent = flow.read_intent(ctl.CONTROL_DIR / "intent.json", catalog, ledger)
    return catalog, ledger, intent


def web_with_production_environment() -> int:
    """Inject production configuration once, without writing secrets to disk."""
    if os.environ.get("AXIOM_RELEASE_INFISICAL_READY") == "1":
        import web_server
        return web_server.main([])
    if not shutil.which("infisical"):
        raise ctl.ReleaseError("release dashboard needs the Infisical CLI and prod authentication")
    config = ctl.WORKSPACE / "AxiomCore/docs/.infisical.json"
    project_id = json.loads(config.read_text()).get("workspaceId", "")
    if not isinstance(project_id, str) or not re.fullmatch(r"[a-f0-9-]{36}", project_id):
        raise ctl.ReleaseError("release dashboard has no valid Infisical project ID")
    environment = dict(os.environ)
    environment["AXIOM_RELEASE_INFISICAL_READY"] = "1"
    command = ["infisical", "run", "--env=prod", f"--projectId={project_id}", "--",
               sys.executable, "-B", str(ctl.CONTROL_DIR / "release_cli.py"), "web"]
    os.execvpe("infisical", command, environment)
    raise AssertionError("Infisical exec returned unexpectedly")


def prepared_intent_matches(prepared: dict, intent: dict, catalog: dict,
                            ledger: dict | None = None) -> bool:
    if not prepared.get("applied") or prepared.get("trainId") != intent["trainId"]:
        return False
    digest = ctl.sha256(ctl.canonical(intent))
    if prepared.get("intentSha256") == digest:
        return True
    ledger = ledger or versions.read_versions(versions.VERSIONS, catalog)
    # A queued-only addition cannot change a prepared active wave. Authenticate
    # the old full intent through Git history before allowing that narrow case.
    repository = ctl.WORKSPACE / "AxiomCore"
    revisions = ctl.run("git", "log", "--all", "--format=%H", "--",
                        "release/control/intent.json", cwd=repository).decode().splitlines()
    for revision in revisions:
        try:
            raw = ctl.run("git", "show", f"{revision}:release/control/intent.json",
                          cwd=repository)
            historical = flow.validate_intent(json.loads(raw), catalog, ledger)
        except (ctl.ReleaseError, ValueError, KeyError):
            continue
        if ctl.sha256(ctl.canonical(historical)) != prepared.get("intentSha256"):
            continue
        without_queue = lambda value: {key: item for key, item in value.items() if key != "queued"}
        if without_queue(historical) != without_queue(intent):
            return False
        current_queue = {item["component"]: item for item in intent.get("queued", [])}
        return all(current_queue.get(item["component"]) == item
                   for item in historical.get("queued", []))
    return False


def status(catalog: dict, ledger: dict, intent: dict) -> None:
    report = scan.inventory(catalog, ctl.WORKSPACE)
    active = {change["component"] for change in intent["changes"]}
    queued = {change["component"] for change in intent.get("queued", [])}
    affected = set(report["affected"])
    root = Path(os.environ.get("AXIOM_RELEASE_BUILD_ROOT", str(ctl.DEFAULT_BUILD_ROOT))).resolve()
    published = cycle.published_evidence(root, catalog, train_id=intent["trainId"])
    print(f"Train {intent['trainId']} · active wave {intent.get('wave', 'default')}")
    print(f"Changed build inputs: {len(affected)} components (relative to local upstream, not published baseline)")
    for component_id in sorted(affected | active | queued):
        state = ("published" if component_id in active and component_id in published else
                 "active" if component_id in active else
                 "queued" if component_id in queued else "uncovered")
        candidate = ledger["components"][component_id]["candidateVersion"]
        print(f"  {component_id:22} {state:9} candidate={candidate or 'digest/none'}")
    if root.is_dir():
        print(f"Published in this train: {len(active & published.keys())}/{len(active)} active component(s) remotely verified")
    if affected - active - queued:
        print("Action: account for uncovered components in intent.json before preparing.")
    evidence = train.status(catalog, ctl.WORKSPACE, root)
    print(f"Evidence: {evidence['evidenceRoot']} ({'available' if evidence['storageAvailable'] else 'SSD not mounted'})")
    preparation_path = train.train_paths(root, intent["trainId"])["preparation"]
    frozen = False
    if preparation_path.is_file():
        previous = json.loads(preparation_path.read_text())
        if not prepared_intent_matches(previous, intent, catalog, ledger):
            frozen = True
            print("Prepared train is frozen: intent changed after preparation. "
                  "Use a new train ID; existing staged candidates can be published with an explicit TRAIN.")
    if active and not frozen:
        try:
            prepared, edits, fragments = flow.make_preparation(intent, catalog, ctl.WORKSPACE)
            if prepared["blocked"]:
                print("Next: resolve dirty release source files listed by `just release prepare`.")
            elif edits or fragments:
                print(f"Next: `just release prepare` will update {len(edits)} version/changelog file(s) and create {len(fragments)} note(s).")
            else:
                print("Next: `just release review` to inspect and commit source changes, then run candidate preflight.")
        except (ctl.ReleaseError, OSError, ValueError) as error:
            print(f"Preparation needs attention: {error}")
    print("Change active components: `just release update` (preserves prior evidence and chooses the next train ID).")
    print("Start a fresh selection: `just release new`.")
    print("Use `just release capabilities` for destination support and staged candidate readiness.")


def capabilities(catalog: dict) -> None:
    print(f"{'Component':24} {'Build':12} {'Publisher':12} Destination")
    for entry in catalog["components"]:
        build = "local" if entry["adapter"] != "ci-only" else (
            "selective" if entry["id"] in ci_builders.SUPPORTED else "CI gap")
        publisher_ready = (entry["id"].startswith("ui-host-") or entry["id"] in {"landing", "docs"}
                           or entry["id"] in publish_targets.GITHUB or entry["id"] in publish_targets.NPM
                           or entry["id"] in publish_targets.PUB or entry["id"] in publish_targets.GCP
                           or entry["id"] in {"sdk-swift", "dashboard-proxy"})
        publish = "wired" if publisher_ready else "adapter gap"
        print(f"{entry['id']:24} {build:12} {publish:12} {entry['destination']}")
    print("Grouped Host publication: `just release publish ui-host [TRAIN]`.")
    print("Selective = source-bound SSD/Cloud Build candidate builder; destination gates still apply.")


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
            ledger = versions.read_versions(versions.VERSIONS, catalog)
            if not prepared_intent_matches(previous, intent, catalog, ledger):
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
    print(f"Prepared {len(edits)} version/changelog file(s) and {len(fragments)} note(s). Backup: {backup}")
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
    if not prepared_intent_matches(previous, intent, catalog):
        raise ctl.ReleaseError(f"release preflight evidence does not match the active intent: {evidence}")
    active = {change["component"] for change in intent["changes"]}
    from dependency_baseline import published_dependency_baseline
    baseline = published_dependency_baseline(root, catalog, ctl.WORKSPACE, active, active)
    plan = ctl.make_plan(catalog, ctl.WORKSPACE, baseline=baseline, only=active)
    if plan["blocked"] or plan["blockedVersions"]:
        raise ctl.ReleaseError("release preflight is blocked by uncommitted inputs or reused versions: "
                               + ", ".join([*plan["blocked"], *plan["blockedVersions"]]))
    selected = {item["id"] for item in plan["components"] if item["selected"]}
    unscoped = selected - active
    if unscoped:
        raise ctl.ReleaseError("release preflight selected dependencies outside the active wave: "
                               + ", ".join(sorted(unscoped)))
    local = sorted(item["id"] for item in plan["components"]
                   if item["selected"] and (item["adapter"] != "ci-only"
                                            or item["id"] in ci_builders.SUPPORTED))
    ci_only = sorted(item["id"] for item in plan["components"]
                     if item["selected"] and item["adapter"] == "ci-only"
                     and item["id"] not in ci_builders.SUPPORTED)
    print(f"Local candidate preflight passed for train {intent['trainId']}: {len(selected)} selected component(s).")
    print("Available candidate builders: " + (", ".join(local) if local else "none"))
    if HOST_IDS <= set(local):
        print("Next local build: `just release ui-host` (live progress; press l for logs).")
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
        print("Committed source awaiting push (confirmed dashboard release pushes required repos): "
              + ", ".join(sorted(ahead_repositories)))
    if missing_upstreams:
        print("Remote/CI gap — configure tracked upstreams: " + ", ".join(sorted(missing_upstreams)))
    print("No candidate was built or published (preflight only). Build with `just release COMPONENT` "
          "or run the reviewed release in the dashboard.")


def publish_one(component: str, root: Path, train_id: str | None = None) -> dict:
    if component in HOST_IDS:
        component = "ui-host"
    staged = publisher.load_candidate(component, root, train_id=train_id)
    if component == "ui-host":
        return publisher.publish_host(staged)
    if component == "landing":
        return publisher.publish_landing(staged)
    if component == "docs":
        return publisher.publish_docs(staged)
    return publish_targets.publish(staged, component)


def open_candidate_directory(directory: Path, scoped: dict, plan: dict) -> None:
    """Create evidence or resume only an exact, unfinished prior build."""
    if not directory.exists():
        ctl.write_json(directory / "intent.json", scoped)
        ctl.write_json(directory / "plan.json", plan)
        return
    if not directory.is_dir() or directory.is_symlink():
        raise ctl.ReleaseError(f"candidate evidence path is not a directory: {directory}")
    allowed = {"intent.json", "plan.json"}
    unexpected = sorted(path.name for path in directory.iterdir()
                        if path.name not in allowed and not
                        (path.is_file() and not path.is_symlink()
                         and path.name.startswith("build-") and path.suffix == ".log"))
    if unexpected:
        raise ctl.ReleaseError(f"candidate evidence is not an incomplete build; inspect before retrying: {directory} ({', '.join(unexpected)})")
    intent_path = directory / "intent.json"
    plan_path = directory / "plan.json"
    if any(not path.is_file() or path.is_symlink() for path in (intent_path, plan_path)):
        raise ctl.ReleaseError(f"candidate evidence is incomplete or linked; inspect before retrying: {directory}")
    old_intent = json.loads(intent_path.read_text())
    old_plan = json.loads(plan_path.read_text())
    if old_intent != scoped or old_plan != plan:
        raise ctl.ReleaseError(f"candidate source plan changed since the interrupted attempt; use a new train instead of overwriting: {directory}")
    print(f"Resuming matching incomplete candidate: {directory}")


def candidate(catalog: dict, intent: dict, component: str) -> None:
    root = ctl.mount_default_build_root()
    preparation = train.train_paths(root, intent["trainId"])["preparation"]
    if not preparation.is_file():
        raise ctl.ReleaseError(f"candidate requires prepared train evidence: {preparation}")
    prepared = json.loads(preparation.read_text())
    if not prepared_intent_matches(prepared, intent, catalog):
        raise ctl.ReleaseError("active intent changed after preparation; use a new train ID")
    requested = HOST_IDS if component == "ui-host" else {component}
    active = {change["component"] for change in intent["changes"]}
    if not requested <= active:
        raise ctl.ReleaseError("candidate must be in the active wave; queued or unknown: "
                               + ", ".join(sorted(requested - active)))
    # A consumer may be released in a later, automatically managed phase.
    # Reuse only dependency artifacts with recorded remote publication proof.
    from dependency_baseline import published_dependency_baseline, receipt_for
    baseline = published_dependency_baseline(root, catalog, ctl.WORKSPACE,
                                             requested, active)
    plan = ctl.make_plan(catalog, ctl.WORKSPACE, baseline=baseline, only=requested)
    selected = {item["id"] for item in plan["components"] if item["selected"]}
    if not selected <= active:
        raise ctl.ReleaseError("selected dependencies are not active: " + ", ".join(sorted(selected - active)))
    if plan["blocked"] or plan["blockedVersions"]:
        raise ctl.ReleaseError("commit changed build inputs and use a new published version before candidate build")
    dependency_issues = ci_builders.dependency_blockers(selected, versions.read_versions(versions.VERSIONS, catalog), ctl.WORKSPACE)
    if dependency_issues:
        raise ctl.ReleaseError("; ".join(dependency_issues))
    prerequisite_issues = ci_builders.prerequisite_issues(selected)
    if prerequisite_issues:
        raise ctl.ReleaseError("; ".join(prerequisite_issues))
    ci_only = sorted(item["id"] for item in plan["components"]
                     if item["selected"] and item["adapter"] == "ci-only"
                     and item["id"] not in ci_builders.SUPPORTED
                     and not receipt_for(root, item, plan["repositories"]).is_file())
    if ci_only:
        raise ctl.ReleaseError("no verified build receipt for CI-only " + ", ".join(ci_only)
                               + "; run its selective builder and import the exact artifact first")
    scoped = {**intent, "changes": [change for change in intent["changes"]
                                     if change["component"] in selected], "queued": []}
    ready = gate.inspect_candidate(scoped, plan, catalog, ctl.WORKSPACE)
    if ready["blockers"]:
        raise ctl.ReleaseError("candidate gate blocked: " + "; ".join(ready["blockers"]))
    directory = train.train_paths(root, intent["trainId"])["directory"] / "components" / component
    open_candidate_directory(directory, scoped, plan)
    receipts = []
    selected_entries = [entry for entry in plan["components"] if entry["selected"]]
    if any(entry["id"] == "ui-host-android" for entry in selected_entries):
        android = next(item for item in selected_entries if item["id"] == "ui-host-android")
        android_receipt = ctl.artifact_receipt_path(root, android, plan["repositories"])
        if not android_receipt.exists():
            signing_env = ctl.build_environment(root, "ui-host-android")
            ctl.verify_android_signing_access(signing_env, ctl.WORKSPACE / "axiom-ui-host")
            print("Android signing preflight passed; secret values were not displayed or saved.")
    for position, entry in enumerate(selected_entries, 1):
        receipt = receipt_for(root, entry, plan["repositories"])
        if receipt.exists():
            verified = ctl.verify_receipt(receipt, entry["fingerprint"])
            if (verified.get("component") != entry["id"] or
                    (entry["selected"] and verified.get("sourceHeads") != plan["repositories"])):
                raise ctl.ReleaseError(f"existing receipt belongs to a different candidate: {receipt}")
            print(f"Reusing verified {entry['id']} artifact: {receipt}")
        else:
            artifact_directory = receipt.parent
            if artifact_directory.exists() and any(artifact_directory.iterdir()):
                raise ctl.ReleaseError(f"incomplete artifact directory has no receipt; inspect before retrying: {artifact_directory}")
            if entry["adapter"] == "ci-only":
                ci_builders.build_component(directory / "plan.json", entry["id"], catalog, ctl.WORKSPACE)
            else:
                ctl.build_component(directory / "plan.json", entry["id"], catalog, ctl.WORKSPACE,
                                    command_runner=progress.command_runner(directory, entry["id"],
                                                                           position, len(selected_entries)))
            receipt = receipt_for(root, entry, plan["repositories"])
            ctl.verify_receipt(receipt, entry["fingerprint"])
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
    if action == "web":
        try:
            return web_with_production_environment()
        except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
            print(f"release: {error}", file=sys.stderr)
            return 2
    if action == "test":
        return subprocess.run([sys.executable, "-B", "-m", "unittest", "discover",
                               "-s", str(ctl.CONTROL_DIR), "-p", "test_*.py", "-v"],
                              check=False).returncode
    try:
        catalog, ledger, intent = load()
        if action == "status":
            status(catalog, ledger, intent)
        elif action in ("new", "new-cycle", "update"):
            if component not in ("", "restart"):
                raise ctl.ReleaseError(f"usage: just release {action} [restart]")
            root = ctl.mount_default_build_root()
            try:
                cycle.run(catalog, ledger, intent, root, restart=component == "restart",
                          update=action == "update")
            except (KeyboardInterrupt, EOFError):
                command = "update" if action == "update" else "new"
                print(f"Release selection interrupted. Run `just release {command}` to resume its SSD draft: "
                      f"{cycle.draft_path(root, update=action == 'update')}")
                return 130
        elif action == "capabilities":
            capabilities(catalog)
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
            known = {item["id"] for item in catalog["components"]} | {"ui-host"}
            if component and component not in known:
                raise ctl.ReleaseError(f"unknown component: {component}")
            root = ctl.mount_default_build_root()
            selected = [component] if component else publish_menu.choose(catalog, root, intent["trainId"])
            if not selected:
                print("Publish cancelled. Nothing changed.")
            for chosen in selected:
                result = publish_one(chosen, root, version or None)
                remote = result["remote"]
                if isinstance(remote, dict):
                    remote = remote.get("url")
                print(f"Published and remotely verified {chosen}: {remote}")
        else:
            raise ctl.ReleaseError(f"unknown action {action!r}; run `just release help`")
        return 0
    except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"release: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
