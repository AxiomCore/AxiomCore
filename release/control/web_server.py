#!/usr/bin/env python3
"""Loopback-only release dashboard over the existing gated control plane.

No production action is performed by a GET request. A release run is created
only from a reviewed preview and an explicit train-ID confirmation.
"""

from __future__ import annotations

import argparse
import base64
import copy
from datetime import datetime, timezone
import hashlib
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
from urllib.parse import parse_qs, urlsplit
import urllib.request

import ctl
import auto_pins
import ci_builders
import cycle
import flow
import publisher
import publish_targets
import release_cli
import versions


ASSETS = Path(__file__).with_name("web")
MAX_REQUEST = 128 * 1024
MAX_DIFF = 128 * 1024
HOST_IDS = ("ui-host-web", "ui-host-android", "ui-host-ios")
TRAIN_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{2,79}")
DEFERRED_PIN_CONSUMERS = frozenset({"sdk-swift", "sdk-flutter", "sdk-atmx-react"})


def now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent,
                                     prefix=".release-web-", delete=False) as output:
        json.dump(value, output, indent=2, sort_keys=True)
        output.write("\n")
        temporary = Path(output.name)
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def git(repository: Path, *args: str) -> str:
    return ctl.run("git", *args, cwd=repository).decode(errors="replace").strip()


def changed_files(repository: Path) -> list[dict]:
    """Use NUL records so spaces and rename paths remain unambiguous."""
    data = ctl.run("git", "status", "--porcelain=v1", "--untracked-files=all", "-z",
                   cwd=repository)
    records = data.split(b"\0")
    result = []
    index = 0
    while index < len(records) and records[index]:
        record = records[index]
        if len(record) < 4 or record[2:3] != b" ":
            raise ctl.ReleaseError(f"unrecognized git status record in {repository}")
        state = record[:2].decode("ascii", "replace")
        path = record[3:].decode("utf-8", "surrogateescape")
        item = {"status": state, "path": path}
        if "R" in state or "C" in state:
            index += 1
            if index >= len(records) or not records[index]:
                raise ctl.ReleaseError(f"incomplete rename record in {repository}")
            item["oldPath"] = records[index].decode("utf-8", "surrogateescape")
        # With --untracked-files=all, Git still reports nested Git checkouts as
        # directory entries. They belong to their own repositories, not this
        # repository's review, commit, or release-push cleanliness check.
        if state != "??" or not path.endswith("/"):
            result.append(item)
        index += 1
    return result


def split_deferred_changes(changes: list[dict], ledger: dict, workspace: Path
                           ) -> tuple[list[dict], list[dict]]:
    """Keep consumers needing unpublished dependency bytes out of this train."""
    ready = []
    deferred = []
    selected = {change["component"] for change in changes}
    for change in changes:
        component = change["component"]
        issues = (ci_builders.dependency_blockers({component}, ledger, workspace)
                  if component in DEFERRED_PIN_CONSUMERS else [])
        if issues or (component == "sdk-flutter" and "runtime-apple" in selected):
            reason = {
                "sdk-atmx-react": "The run will pin the exact published atmx-web version and npm lockfile integrity.",
                "sdk-swift": "The run will pin the exact remotely verified runtime XCFramework checksum.",
                "sdk-flutter": "The run will pin both Flutter podspecs to the remotely verified Apple runtime.",
            }[component]
            deferred.append({"id": component, "reason": reason, "change": change})
        else:
            ready.append(change)
    return ready, deferred


def suggested_version(source: str | None, candidate: str | None,
                      published: str | None) -> str | None:
    """Never suggest a version older than source or already published bytes."""
    options = [value for value in (source, candidate) if value and versions.STABLE_VERSION.fullmatch(value)]
    if published and versions.STABLE_VERSION.fullmatch(published):
        options.append(published)
    if not options:
        return None
    highest = max(options, key=flow.stable_tuple)
    if published and versions.STABLE_VERSION.fullmatch(published) and flow.stable_tuple(highest) <= flow.stable_tuple(published):
        major, minor, patch = flow.stable_tuple(highest)
        return f"{major}.{minor}.{patch + 1}"
    return highest


def release_groups(catalog: dict, selected: set[str]) -> list[str]:
    """Hard dependencies and selected-only rollout predecessors share one order."""
    by_id = {item["id"]: item for item in catalog["components"]}
    ordered: list[str] = []
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(component_id: str) -> None:
        if component_id in visiting:
            raise ctl.ReleaseError(f"release ordering cycle at {component_id}")
        if component_id in visited:
            return
        visiting.add(component_id)
        entry = by_id[component_id]
        for before in [*entry.get("depends_on", []), *entry.get("release_after", [])]:
            if before in selected:
                visit(before)
        visiting.remove(component_id)
        visited.add(component_id)
        group = "ui-host" if component_id in HOST_IDS else component_id
        if group not in ordered:
            ordered.append(group)

    for component in catalog["components"]:
        if component["id"] in selected:
            visit(component["id"])
    return ordered


class ReleaseDashboard:
    def __init__(self, workspace: Path = ctl.WORKSPACE, root: Path = ctl.DEFAULT_BUILD_ROOT):
        self.workspace = workspace.resolve()
        self.root = root.resolve()
        self.token = secrets.token_urlsafe(32)
        self.lock = threading.RLock()
        self.previews: dict[str, dict] = {}
        self.worker: threading.Thread | None = None
        self.npm_versions: dict[str, tuple[float, str | None]] = {}
        self.pub_versions: dict[str, tuple[float, str | None]] = {}

    def _npm_latest_version(self, package: str) -> str | None:
        cached = self.npm_versions.get(package)
        if cached and time.monotonic() - cached[0] < 120:
            return cached[1]
        environment = self._npm_public_environment()
        result = subprocess.run(["npm", "view", package, "versions", "--json", "--prefer-online",
                                 "--registry=https://registry.npmjs.org"],
                                cwd=self.workspace / "AxiomCore", env=environment,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=20, check=False)
        if result.returncode:
            if "E404" in result.stderr.decode(errors="replace"):
                self.npm_versions[package] = (time.monotonic(), None)
                return None
            raise ctl.ReleaseError(f"could not verify npm versions for {package}: " +
                                   result.stderr.decode(errors="replace")[-300:])
        values = json.loads(result.stdout)
        if isinstance(values, str):
            values = [values]
        if not isinstance(values, list):
            raise ctl.ReleaseError(f"npm returned an invalid version list for {package}")
        stable = [value for value in values if isinstance(value, str) and
                  versions.STABLE_VERSION.fullmatch(value)]
        latest = max(stable, key=flow.stable_tuple) if stable else None
        self.npm_versions[package] = (time.monotonic(), latest)
        return latest

    def _pub_latest_version(self, package: str) -> str | None:
        cached = self.pub_versions.get(package)
        if cached and time.monotonic() - cached[0] < 120:
            return cached[1]
        with urllib.request.urlopen(f"https://pub.dev/api/packages/{package}", timeout=20) as response:
            payload = json.load(response)
        values = payload.get("versions") if isinstance(payload, dict) else None
        if not isinstance(values, list):
            raise ctl.ReleaseError(f"pub.dev returned an invalid version list for {package}")
        stable = [item["version"] for item in values if isinstance(item, dict) and
                  isinstance(item.get("version"), str) and
                  versions.STABLE_VERSION.fullmatch(item["version"])]
        latest = max(stable, key=flow.stable_tuple) if stable else None
        self.pub_versions[package] = (time.monotonic(), latest)
        return latest

    @staticmethod
    def _npm_public_environment() -> dict[str, str]:
        environment = {key: value for key, value in os.environ.items()
                       if key not in ("NPM_TOKEN", "NODE_AUTH_TOKEN", "NPM_CONFIG_USERCONFIG")}
        environment["NPM_CONFIG_USERCONFIG"] = os.devnull
        return environment

    def _load(self) -> tuple[dict, dict, dict]:
        catalog = ctl.read_catalog()
        ledger = versions.read_versions(versions.VERSIONS, catalog)
        intent = flow.read_intent(ctl.CONTROL_DIR / "intent.json", catalog, ledger)
        return catalog, ledger, intent

    def _repo(self, catalog: dict, name: str) -> Path:
        if name not in catalog["repositories"]:
            raise ctl.ReleaseError(f"unknown repository: {name}")
        return ctl.repo_path(catalog, name, self.workspace)

    def _published(self, catalog: dict) -> dict:
        if not self.root.is_dir():
            return {}
        return cycle.published_evidence(self.root, catalog)

    def state(self) -> dict:
        catalog, ledger, intent = self._load()
        published = self._published(catalog)
        current = (cycle.published_evidence(self.root, catalog, intent["trainId"])
                   if self.root.is_dir() else {})
        latest_job = self.job()
        if (latest_job and latest_job.get("followup", {}).get("trainId") == intent["trainId"]
                and self.root.is_dir()):
            current.update(cycle.published_evidence(self.root, catalog, latest_job["trainId"]))
        active = {item["component"] for item in intent["changes"]}
        queued = {item["component"] for item in intent.get("queued", [])}
        components = []
        for entry in catalog["components"]:
            component_id = entry["id"]
            try:
                source_version = ctl.component_version(entry, catalog, self.workspace)
            except (ctl.ReleaseError, OSError):
                source_version = None
            published_version = published.get(component_id, {}).get("version")
            registry_error = None
            if component_id in publish_targets.NPM:
                try:
                    remote_version = self._npm_latest_version(publish_targets.NPM[component_id])
                    if remote_version and (not published_version or
                                           flow.stable_tuple(remote_version) > flow.stable_tuple(published_version)):
                        published_version = remote_version
                except (ctl.ReleaseError, OSError, subprocess.TimeoutExpired) as error:
                    registry_error = str(error)
            elif component_id in publish_targets.PUB:
                try:
                    remote_version = self._pub_latest_version(publish_targets.PUB[component_id])
                    if remote_version and (not published_version or
                                           flow.stable_tuple(remote_version) > flow.stable_tuple(published_version)):
                        published_version = remote_version
                except (ctl.ReleaseError, OSError, ValueError) as error:
                    registry_error = str(error)
            state = ("published" if component_id in current else "active" if component_id in active
                     else "queued" if component_id in queued else "unselected")
            candidate_error = None
            staged_version = None
            if state == "active" and self.root.is_dir():
                candidate_name = "ui-host" if component_id in HOST_IDS else component_id
                candidate_dir = self.root / "trains" / intent["trainId"] / "components" / candidate_name
                if (candidate_dir / "staged.json").is_file():
                    try:
                        staged = json.loads((candidate_dir / "staged.json").read_text())
                        staged_version = next((item.get("releaseVersion") for item in staged.get("components", [])
                                               if item.get("id") == component_id), None)
                    except (OSError, ValueError):
                        pass
                    try:
                        publisher.load_candidate(candidate_name, self.root, self.workspace, intent["trainId"])
                        state = "staged"
                    except (ctl.ReleaseError, OSError, ValueError) as error:
                        state = "stale"
                        candidate_error = str(error)
                elif candidate_dir.is_dir() and any(candidate_dir.iterdir()):
                    state = "incomplete"
            components.append({
                "id": component_id, "kind": entry["kind"], "owner": entry["owner"],
                "destination": entry["destination"], "dependsOn": entry.get("depends_on", []),
                "localBuild": (entry["adapter"] != "ci-only" or component_id in ci_builders.SUPPORTED),
                "builder": ("Cloud Build" if component_id in ci_builders.IMAGES else
                            "Selective SSD" if component_id in ci_builders.SUPPORTED else
                            "Local SSD" if entry["adapter"] != "ci-only" else "Unavailable"),
                "versioned": versions.versioned(entry),
                "sourceVersion": source_version,
                "candidateVersion": ledger["components"][component_id]["candidateVersion"],
                "lastReleased": (published_version or published.get(component_id, {}).get("trainId")),
                "suggestedVersion": (suggested_version(source_version,
                                     ledger["components"][component_id]["candidateVersion"],
                                     published_version)
                                     if versions.versioned(entry) else None),
                "stagedVersion": staged_version,
                "state": state, "candidateError": candidate_error,
                "registryError": registry_error,
            })
        repositories = []
        for name in catalog["repositories"]:
            try:
                repository = self._repo(catalog, name)
                files = changed_files(repository)
                upstream, ahead = release_cli.upstream_state(repository)
                if files or ahead:
                    repositories.append({"name": name, "files": files, "ahead": ahead,
                                         "upstream": upstream,
                                         "head": git(repository, "rev-parse", "HEAD")})
            except (ctl.ReleaseError, OSError) as error:
                repositories.append({"name": name, "error": str(error), "files": []})
        return {"trainId": intent["trainId"], "storageAvailable": self.root.is_dir(),
                "storageRoot": str(self.root), "components": components,
                "repositories": repositories, "job": latest_job,
                "publishedInCurrentRun": len(current)}

    def diff(self, name: str, path: str) -> dict:
        catalog, _, _ = self._load()
        repository = self._repo(catalog, name)
        matches = [item for item in changed_files(repository) if item["path"] == path]
        if len(matches) != 1:
            raise ctl.ReleaseError("file is not in the current repository change list")
        output = subprocess.run(["git", "diff", "--no-ext-diff", "HEAD", "--", path],
                                cwd=repository, capture_output=True, check=False)
        if output.returncode:
            raise ctl.ReleaseError("could not inspect the selected file")
        content = output.stdout[:MAX_DIFF].decode("utf-8", "replace")
        if matches[0]["status"] == "??":
            content = "Untracked file; inspect it in your editor before committing."
        elif len(output.stdout) > MAX_DIFF:
            content += "\n… diff truncated; inspect the rest in your editor."
        return {"repository": name, "path": path, "diff": content}

    def commit_reviewed(self, name: str, paths: list[str], message: str) -> dict:
        with self.lock:
            if self.worker and self.worker.is_alive():
                raise ctl.ReleaseError("source review is locked while a release run is active")
            return self._commit_reviewed(name, paths, message)

    def _commit_reviewed(self, name: str, paths: list[str], message: str) -> dict:
        if not isinstance(message, str) or not message.strip() or len(message) > 200 or "\n" in message:
            raise ctl.ReleaseError("enter a one-line commit message (up to 200 characters)")
        if not isinstance(paths, list) or not paths or any(not isinstance(item, str) for item in paths):
            raise ctl.ReleaseError("select at least one changed file")
        catalog, _, _ = self._load()
        repository = self._repo(catalog, name)
        current = {item["path"]: item for item in changed_files(repository)}
        if len(set(paths)) != len(paths) or any(path not in current for path in paths):
            raise ctl.ReleaseError("selected files changed; refresh the review list")
        exact = []
        for path in paths:
            exact.append(path)
            if current[path].get("oldPath"):
                exact.append(current[path]["oldPath"])
        staged_before = {item.decode("utf-8", "surrogateescape") for item in
                         ctl.run("git", "diff", "--cached", "--name-only", "-z", cwd=repository).split(b"\0") if item}
        if staged_before - set(exact):
            raise ctl.ReleaseError("unrelated files are already staged; review them before this commit")
        git(repository, "add", "-A", "--", *exact)
        staged_after = {item.decode("utf-8", "surrogateescape") for item in
                        ctl.run("git", "diff", "--cached", "--name-only", "-z", cwd=repository).split(b"\0") if item}
        if not staged_after or staged_after - set(exact):
            raise ctl.ReleaseError("staged paths differ from the reviewed selection; no commit was made")
        git(repository, "commit", "-m", message.strip())
        return {"repository": name, "commit": git(repository, "rev-parse", "HEAD"),
                "remaining": changed_files(repository)}

    def push_committed(self, requested: list[dict]) -> dict:
        """Push only the exact committed HEADs the operator saw in the review panel."""
        with self.lock:
            if self.worker and self.worker.is_alive():
                raise ctl.ReleaseError("source push is locked while a release run is active")
            if (not isinstance(requested, list) or not requested or
                    any(not isinstance(item, dict) or not isinstance(item.get("name"), str)
                        or not isinstance(item.get("head"), str)
                        or not isinstance(item.get("upstream"), str) for item in requested)):
                raise ctl.ReleaseError("select committed repositories to push")
            names = [item["name"] for item in requested]
            if len(set(names)) != len(names):
                raise ctl.ReleaseError("duplicate repository in push request")
            catalog, _, _ = self._load()
            ready = []
            # Check every repository before the first network mutation. Git's
            # normal fast-forward push remains the final race protection.
            for item in requested:
                name = item["name"]
                repository = self._repo(catalog, name)
                head = git(repository, "rev-parse", "HEAD")
                if head != item["head"]:
                    raise ctl.ReleaseError(f"{name} changed since the push list was loaded; refresh it")
                upstream, ahead = release_cli.upstream_state(repository)
                if upstream != item["upstream"]:
                    raise ctl.ReleaseError(f"{name}: tracked branch changed since the push list was loaded")
                if not upstream or "/" not in upstream or ahead < 1:
                    raise ctl.ReleaseError(f"{name} has no committed changes ahead of a tracked upstream")
                remote, branch = upstream.split("/", 1)
                remote_line = git(repository, "ls-remote", remote, f"refs/heads/{branch}")
                remote_head = remote_line.split()[0] if remote_line else ""
                if not remote_head:
                    raise ctl.ReleaseError(f"{name}: tracked remote branch is missing")
                ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", remote_head, head],
                                          cwd=repository, check=False)
                if ancestor.returncode:
                    raise ctl.ReleaseError(f"{name}: remote branch diverged; no push was attempted")
                ready.append((name, repository, remote, branch, head))
            pushed = []
            for name, repository, remote, branch, head in ready:
                git(repository, "push", remote, f"HEAD:refs/heads/{branch}")
                verified = git(repository, "ls-remote", remote, f"refs/heads/{branch}").split()[0]
                if verified != head:
                    raise ctl.ReleaseError(f"{name}: remote tip changed after push; already pushed: {', '.join(pushed)}")
                pushed.append(name)
            return {"pushed": pushed}

    def preview(self, form: dict) -> dict:
        catalog, ledger, old_intent = self._load()
        requested = form.get("components")
        summary = form.get("summary")
        if not isinstance(requested, list) or not requested or len(requested) > len(catalog["components"]):
            raise ctl.ReleaseError("select one or more components")
        if not isinstance(summary, str) or not summary.strip() or len(summary) > 300 or "\n" in summary:
            raise ctl.ReleaseError("enter a one-line release summary (up to 300 characters)")
        by_id = {entry["id"]: entry for entry in catalog["components"]}
        ids = [item.get("id") for item in requested if isinstance(item, dict)]
        if len(ids) != len(requested) or len(set(ids)) != len(ids) or any(item not in by_id for item in ids):
            raise ctl.ReleaseError("release selection contains an unknown or duplicate component")
        selected = set(ids)
        blockers = []
        if selected & set(HOST_IDS) and not set(HOST_IDS) <= selected:
            blockers.append("Select all three UI Host targets together; they share one signed release.")
        updated = copy.deepcopy(ledger)
        changes = []
        published = self._published(catalog)
        for raw in requested:
            component_id = raw["id"]
            entry = by_id[component_id]
            if entry["adapter"] == "ci-only" and component_id not in ci_builders.SUPPORTED:
                blockers.append(f"{component_id}: selective CI builder/receipt producer is not available yet")
            change_type = raw.get("type", "")
            note = raw.get("summary", "")
            migration = raw.get("migration", "")
            if change_type not in flow.CHANGE_TYPES or not isinstance(note, str) or not note.strip() or len(note) > 500 or "\n" in note:
                raise ctl.ReleaseError(f"{component_id}: choose a change type and enter a one-line note")
            change = {"component": component_id, "type": change_type, "summary": note.strip()}
            if change_type == "breaking":
                if not isinstance(migration, str) or not migration.strip():
                    raise ctl.ReleaseError(f"{component_id}: breaking changes need migration guidance")
                change["migration"] = migration.strip()
            previous = next((item for item in [*old_intent["changes"], *old_intent.get("queued", [])]
                             if item["component"] == component_id), {})
            if "runtimeVersion" in previous:
                change["runtimeVersion"] = previous["runtimeVersion"]
            if versions.versioned(entry):
                version = raw.get("version")
                if not isinstance(version, str) or not versions.STABLE_VERSION.fullmatch(version):
                    raise ctl.ReleaseError(f"{component_id}: enter a stable X.Y.Z candidate version")
                source = ctl.component_version(entry, catalog, self.workspace)
                if source and flow.stable_tuple(version) < flow.stable_tuple(source):
                    raise ctl.ReleaseError(f"{component_id}: version {version} is older than source {source}")
                released = published.get(component_id, {}).get("version")
                if released and flow.stable_tuple(version) <= flow.stable_tuple(released):
                    raise ctl.ReleaseError(f"{component_id}: version must be newer than published {released}")
                if component_id in publish_targets.NPM:
                    try:
                        remote = self._npm_latest_version(publish_targets.NPM[component_id])
                    except (ctl.ReleaseError, OSError, subprocess.TimeoutExpired) as error:
                        blockers.append(f"{component_id}: cannot verify npm version availability: {error}")
                    else:
                        if remote and flow.stable_tuple(version) <= flow.stable_tuple(remote):
                            blockers.append(f"{component_id}: npm already has {remote}; choose a version newer than it")
                        else:
                            try:
                                occupied = publish_targets._npm_metadata(
                                    publish_targets.NPM[component_id], version,
                                    self.workspace / "AxiomCore",
                                    self._npm_public_environment())
                            except ctl.ReleaseError as error:
                                blockers.append(f"{component_id}: cannot verify npm candidate availability: {error}")
                            else:
                                if occupied:
                                    blockers.append(f"{component_id}: npm {version} is already occupied; choose a new version")
                elif component_id in publish_targets.PUB:
                    try:
                        remote = self._pub_latest_version(publish_targets.PUB[component_id])
                        occupied = publish_targets._pub_metadata(publish_targets.PUB[component_id], version)
                    except (ctl.ReleaseError, OSError, ValueError) as error:
                        blockers.append(f"{component_id}: cannot verify pub.dev version availability: {error}")
                    else:
                        if occupied or (remote and flow.stable_tuple(version) <= flow.stable_tuple(remote)):
                            blockers.append(f"{component_id}: pub.dev already has {version if occupied else remote}; choose a newer free version")
                updated["components"][component_id]["candidateVersion"] = version
            changes.append(change)
        if "runtime-apple" in selected and "sdk-flutter" in selected:
            runtime_version = updated["components"]["runtime-apple"]["candidateVersion"]
            next(change for change in changes if change["component"] == "sdk-flutter")["runtimeVersion"] = runtime_version
        versions.validate_versions(updated, catalog)
        changes, deferred = split_deferred_changes(changes, updated, self.workspace)
        ids = [change["component"] for change in changes]
        selected = set(ids)
        if not changes:
            raise ctl.ReleaseError("No component is release-ready; publish upstream dependencies and update their source pins first")
        blockers.extend(ci_builders.dependency_blockers(selected, updated, self.workspace))
        blockers.extend(ci_builders.prerequisite_issues(selected))
        followup_ids = {item["id"] for item in deferred}
        blockers.extend(ci_builders.prerequisite_issues(followup_ids))
        for item in deferred:
            upstream = by_id[item["id"]].get("depends_on", [])
            for dependency in upstream:
                if dependency not in selected:
                    blockers.append(f"{item['id']}: select {dependency} so its published bytes can be pinned automatically")
        for component_id in selected:
            missing = set(by_id[component_id].get("depends_on", [])) - selected
            if missing:
                blockers.append(f"{component_id}: select dependency {', '.join(sorted(missing))}")
        train_id = cycle.next_train_id(old_intent["trainId"], self.root)
        effective_summary = summary.strip()
        intent = cycle.compose_intent(old_intent, train_id, changes, effective_summary, published)
        if deferred:
            deferred_ids = {item["id"] for item in deferred}
            intent["queued"] = [change for change in intent["queued"]
                                if change["component"] not in deferred_ids]
            intent["queued"].extend({key: value for key, value in item["change"].items()
                                     if key != "version"} for item in deferred)
        normalized = flow.validate_intent(copy.deepcopy(intent), catalog, updated)
        report, edits, fragments = flow.make_preparation(normalized, catalog, self.workspace)
        blockers.extend(report["blocked"])
        required_repos = {"AxiomCore"}
        for component_id in selected | followup_ids:
            entry = by_id[component_id]
            required_repos.add(entry["owner"])
            required_repos.update(source["repo"] for source in entry["sources"])
        for path in [*edits, *fragments]:
            owner = flow.containing_repo(catalog, self.workspace, path)
            required_repos.add(next(name for name in catalog["repositories"]
                                    if self._repo(catalog, name) == owner))
        repositories = []
        for name in sorted(required_repos):
            repository = self._repo(catalog, name)
            files = changed_files(repository)
            upstream, ahead = release_cli.upstream_state(repository)
            repo_info = {"name": name, "changed": files, "ahead": ahead, "upstream": upstream,
                         "head": git(repository, "rev-parse", "HEAD")}
            if upstream and ahead:
                repo_info["commitsToPush"] = git(repository, "log", "--format=%h %s", "-20",
                                                   f"{upstream}..HEAD").splitlines()
            repositories.append(repo_info)
            if files:
                blockers.append(f"{name}: review and commit {len(files)} existing changed file(s) first")
            if upstream is None:
                blockers.append(f"{name}: configure a tracked upstream before automatic push")
            else:
                try:
                    remote, branch = upstream.split("/", 1)
                    remote_line = git(repository, "ls-remote", remote, f"refs/heads/{branch}")
                    remote_head = remote_line.split()[0] if remote_line else ""
                    local_head = repo_info["head"]
                    repo_info["remoteHead"] = remote_head
                    if not remote_head:
                        blockers.append(f"{name}: tracked remote branch is missing")
                    elif remote_head != local_head and subprocess.run(
                            ["git", "merge-base", "--is-ancestor", remote_head, local_head],
                            cwd=repository, check=False).returncode:
                        blockers.append(f"{name}: remote branch advanced or diverged; synchronize it before release")
                except (ctl.ReleaseError, ValueError, OSError) as error:
                    blockers.append(f"{name}: could not verify remote branch: {error}")
        if not self.root.is_dir() and self.root == ctl.DEFAULT_BUILD_ROOT.resolve():
            # The start action mounts the existing external image; no volume is created.
            storage = "The existing external release image will be mounted before work starts."
        elif not self.root.is_dir():
            blockers.append("external release storage is unavailable")
            storage = "External storage unavailable"
        else:
            storage = str(self.root)
        if "dashboard-proxy" in selected and not os.environ.get("AXIOM_DASHBOARD_ORIGIN", "").startswith("https://"):
            blockers.append("dashboard-proxy needs AXIOM_DASHBOARD_ORIGIN set to its Cloud Run HTTPS origin")
        inputs = {"intent": ctl.sha256((ctl.CONTROL_DIR / "intent.json").read_bytes()),
                  "versions": ctl.sha256(versions.VERSIONS.read_bytes()),
                  "repos": {item["name"]: {"head": item["head"],
                                              "remoteHead": item.get("remoteHead"),
                                              "changed": item["changed"]} for item in repositories}}
        digest = ctl.sha256(ctl.canonical({"form": form, "trainId": train_id, "inputs": inputs,
                                           "catalog": catalog["sha256"]}))
        followup_train = cycle.next_train_id(train_id, self.root,
                                             today=train_id.rsplit(".", 1)[0]) if deferred else None
        public = {"previewId": digest, "trainId": train_id, "components": ids,
                  "requestedCount": len(requested),
                  "deferred": [{"id": item["id"], "reason": item["reason"]} for item in deferred],
                  "followupTrainId": followup_train,
                  "followupOrder": release_groups(catalog, followup_ids) if deferred else [],
                  "summary": effective_summary,
                  "order": release_groups(catalog, selected), "repositories": repositories,
                  "versionFiles": [str(path) for path in edits],
                  "notes": [str(path) for path in fragments],
                  "notesPlanned": len(requested), "storage": storage,
                  "blockers": blockers, "ready": not blockers,
                  "willCommit": True, "willPush": True, "willPublish": True}
        with self.lock:
            self.previews[digest] = {"public": public, "form": copy.deepcopy(form),
                                     "intent": intent, "ledger": updated, "oldIntent": old_intent,
                                     "requiredRepos": sorted(required_repos), "inputs": inputs,
                                     "followupChanges": [item["change"] for item in deferred],
                                     "managedPaths": [str(ctl.CONTROL_DIR / "intent.json"),
                                                      str(versions.VERSIONS),
                                                      *(str(path) for path in edits),
                                                      *(str(path) for path in fragments)]}
        return public

    def _job_path(self, train_id: str) -> Path:
        if not TRAIN_NAME.fullmatch(train_id):
            raise ctl.ReleaseError("invalid release train ID")
        return self.root / "ui-runs" / f"{train_id}.json"

    def _log_path(self, train_id: str) -> Path:
        return self.root / "ui-runs" / f"{train_id}.log"

    def job(self, train_id: str | None = None) -> dict | None:
        directory = self.root / "ui-runs"
        if not directory.is_dir():
            return None
        if train_id:
            path = self._job_path(train_id)
        else:
            files = [item for item in directory.glob("*.json") if item.is_file() and not item.is_symlink()]
            if not files:
                return None
            path = max(files, key=lambda item: item.stat().st_mtime)
        if not path.is_file() or path.is_symlink():
            return None
        document = json.loads(path.read_text())
        if document.get("status") == "running" and not (self.worker and self.worker.is_alive()):
            document["status"] = "interrupted"
        log = self._log_path(document["trainId"])
        if log.is_file() and not log.is_symlink():
            with log.open("rb") as source:
                source.seek(max(0, log.stat().st_size - 96 * 1024))
                document["logTail"] = source.read().decode("utf-8", "replace")
        else:
            document["logTail"] = ""
        document.pop("form", None)
        document.pop("intent", None)
        document.pop("ledger", None)
        document.pop("oldIntent", None)
        document.pop("oldLedger", None)
        document.pop("managedPaths", None)
        if "followup" in document:
            followup = document["followup"]
            document["followup"] = {"trainId": followup["trainId"],
                                    "order": followup["order"],
                                    "components": [item["component"] for item in followup["changes"]]}
        return document

    def _save_job(self, job: dict) -> None:
        job["updatedAt"] = now()
        atomic_json(self._job_path(job["trainId"]), job)

    def _log(self, job: dict, message: str) -> None:
        with self._log_path(job["trainId"]).open("a", encoding="utf-8") as output:
            output.write(f"[{now()}] {message.rstrip()}\n")

    @staticmethod
    def _phase(job: dict) -> dict:
        return job["followup"] if job.get("phase") == "followup" else job

    def start(self, preview_id: str, confirmation: str) -> dict:
        with self.lock:
            if self.worker and self.worker.is_alive():
                raise ctl.ReleaseError("a release is already running; watch its progress below")
            planned = self.previews.get(preview_id)
            if not planned:
                raise ctl.ReleaseError("preview expired; review this release again")
            public = planned["public"]
            if not public["ready"]:
                raise ctl.ReleaseError("resolve all preview blockers before starting")
            if confirmation != f"PUBLISH {public['trainId']}":
                raise ctl.ReleaseError(f"type PUBLISH {public['trainId']} to authorize commits, pushes and publication")
            if self.root == ctl.DEFAULT_BUILD_ROOT.resolve():
                ctl.mount_default_build_root()
            refreshed = self.preview(planned["form"])
            if refreshed["previewId"] != preview_id or not refreshed["ready"]:
                raise ctl.ReleaseError("source or release inputs changed; review a fresh preview")
            if not self.root.is_dir():
                raise ctl.ReleaseError("external release storage is unavailable")
            train_id = public["trainId"]
            if self._job_path(train_id).exists():
                raise ctl.ReleaseError("a run for this train already exists; inspect or resume it")
            job = {"format": "axiom-release-web-run/v1", "trainId": train_id,
                   "status": "running", "startedAt": now(), "updatedAt": now(),
                   "currentStep": None, "completed": [], "error": None,
                   "order": public["order"], "components": public["components"],
                   "requiredRepos": planned["requiredRepos"],
                   "managedPaths": planned["managedPaths"],
                   "form": planned["form"], "intent": planned["intent"],
                   "ledger": planned["ledger"],
                   "oldIntent": (ctl.CONTROL_DIR / "intent.json").read_text(),
                   "oldLedger": versions.VERSIONS.read_text()}
            if planned["followupChanges"]:
                job["followup"] = {"trainId": public["followupTrainId"],
                                   "changes": planned["followupChanges"],
                                   "order": public["followupOrder"],
                                   "requiredRepos": sorted({"AxiomCore", *[
                                       next(item["owner"] for item in ctl.read_catalog()["components"]
                                            if item["id"] == change["component"])
                                       for change in planned["followupChanges"]]})}
            ctl.write_json(self._job_path(train_id), job)
            self._log(job, "Release authorized. Source will be committed and pushed before any build; all builds finish before publication starts.")
            self.worker = threading.Thread(target=self._run, args=(job,), daemon=True)
            self.worker.start()
            return {"trainId": train_id, "status": "running"}

    def resume(self, train_id: str, confirmation: str) -> dict:
        with self.lock:
            if self.worker and self.worker.is_alive():
                raise ctl.ReleaseError("a release is already running")
            path = self._job_path(train_id)
            if not path.is_file() or path.is_symlink():
                raise ctl.ReleaseError("release run does not exist")
            job = json.loads(path.read_text())
            if job.get("format") != "axiom-release-web-run/v1" or job.get("status") == "complete":
                raise ctl.ReleaseError("this release run cannot be resumed")
            if confirmation != f"RESUME {train_id}":
                raise ctl.ReleaseError(f"type RESUME {train_id} to continue the saved run")
            step = job.get("currentStep", "")
            if (job.get("status") == "blocked" and step.startswith("publish:")
                    and step not in job.get("completed", [])):
                component = step.removeprefix("publish:")
                try:
                    if component in publish_targets.NPM:
                        recovery = self.npm_collision_recovery(train_id)
                    elif component in publish_targets.PUB:
                        recovery = self.pub_collision_recovery(train_id)
                    else:
                        recovery = None
                except ctl.ReleaseError as error:
                    # A missing npm version or an exact registry match is not
                    # an immutable collision; ordinary resume remains valid.
                    if not str(error).startswith(("npm has not published this version;",
                                                   "npm now serves the exact staged archive;",
                                                   "pub.dev has not published this version;",
                                                   "pub.dev now serves the exact staged archive;")):
                        raise
                else:
                    if recovery:
                        raise ctl.ReleaseError(
                            f"{recovery['package']}@{recovery['occupiedVersion']} cannot be retried: "
                            "confirm 'Move package candidate to successor train' first, then resume")
            child_pid = job.get("childPid")
            if isinstance(child_pid, int):
                try:
                    os.kill(child_pid, 0)
                except ProcessLookupError:
                    pass
                else:
                    raise ctl.ReleaseError(f"release child process {child_pid} may still be running; inspect it before resuming")
            current = json.loads((ctl.CONTROL_DIR / "intent.json").read_text())
            followup = job.get("followup", {})
            followup_started = ("followup:cycle" in job["completed"] or
                                (followup.get("intent") is not None and
                                 (self.root / "trains" / followup["trainId"] / "cycle-backup").is_dir()
                                 and current == followup["intent"]))
            expected_intent = followup["intent"] if followup_started else job["intent"]
            if "cycle" in job["completed"] and current != expected_intent:
                raise ctl.ReleaseError("active release intent changed; inspect the saved run before resuming")
            job["status"] = "running"
            job["error"] = None
            self._save_job(job)
            self._log(job, "Operator requested resume. Completed steps will not be repeated.")
            self.worker = threading.Thread(target=self._run, args=(job,), daemon=True)
            self.worker.start()
            return {"trainId": train_id, "status": "running"}

    def npm_collision_recovery(self, train_id: str) -> dict:
        """Read-only proof that a blocked npm candidate needs a successor version."""
        path = self._job_path(train_id)
        if not path.is_file() or path.is_symlink():
            raise ctl.ReleaseError("release run does not exist")
        job = json.loads(path.read_text())
        step = job.get("currentStep", "")
        if (job.get("status") != "blocked" or job.get("phase") == "followup"
                or not step.startswith("publish:") or step in job.get("completed", [])):
            raise ctl.ReleaseError("this run is not blocked at a primary npm publication")
        component = step.removeprefix("publish:")
        if component not in publish_targets.NPM or component in {
                item["component"] for item in job.get("deferredPublications", [])}:
            raise ctl.ReleaseError("the blocked component has no npm version-collision recovery")
        candidate = publisher.load_candidate(component, self.root, self.workspace, train_id)
        archive, _ = publish_targets._npm_archive(candidate, component)
        package = publish_targets.NPM[component]
        version = publish_targets._version(candidate, component)
        npm_env = self._npm_public_environment()
        metadata = publish_targets._npm_metadata(package, version, candidate["directory"], npm_env)
        if metadata is None:
            raise ctl.ReleaseError("npm has not published this version; resume the saved run normally")
        expected = "sha512-" + base64.b64encode(hashlib.sha512(archive.read_bytes()).digest()).decode()
        if metadata.get("dist", {}).get("integrity") == expected:
            raise ctl.ReleaseError("npm now serves the exact staged archive; resume the saved run normally")
        if (candidate["directory"] / "publication/oidc-dispatch.json").exists():
            raise ctl.ReleaseError("an OIDC handoff exists for this version; inspect its workflow before replanning")
        # Do not treat stale metadata alone as an immutable conflict. The
        # registry tarball must independently disagree with the staged bytes.
        try:
            publish_targets._verify_npm_tarball(package, version, metadata, archive)
        except ctl.ReleaseError as error:
            if "differs from the exact staged bytes" not in str(error):
                raise
        else:
            raise ctl.ReleaseError("npm metadata disagrees but its tarball matches; retry publication")
        if (candidate["directory"] / "publication/published.json").exists():
            raise ctl.ReleaseError("publication evidence already exists; inspect before replanning")
        entry = next(item for item in candidate["catalog"]["components"] if item["id"] == component)
        source_version = ctl.component_version(entry, candidate["catalog"], self.workspace)
        remote_latest = self._npm_latest_version(package)
        base_version = max((item for item in (version, source_version, remote_latest) if item),
                           key=flow.stable_tuple)
        major, minor, patch = flow.stable_tuple(base_version)
        suggested = None
        for offset in range(1, 101):
            proposal = f"{major}.{minor}.{patch + offset}"
            if publish_targets._npm_metadata(package, proposal, candidate["directory"], npm_env) is None:
                suggested = proposal
                break
        if suggested is None:
            raise ctl.ReleaseError("the next 100 patch versions are occupied; choose a new release manually")
        original_note = next(item["summary"] for item in job["intent"]["changes"]
                             if item["component"] == component)
        return {"trainId": train_id, "component": component, "package": package,
                "occupiedVersion": version, "nextVersion": suggested,
                "suggestedSummary": original_note.replace(version, suggested),
                "followupTrainId": job.get("followup", {}).get("trainId") or
                                   cycle.next_train_id(train_id, self.root)}

    def pub_collision_recovery(self, train_id: str) -> dict:
        """Read-only proof of an immutable pub.dev version collision."""
        path = self._job_path(train_id)
        if not path.is_file() or path.is_symlink():
            raise ctl.ReleaseError("release run does not exist")
        job = json.loads(path.read_text())
        step = job.get("currentStep", "")
        if (job.get("status") != "blocked" or job.get("phase") == "followup"
                or not step.startswith("publish:") or step in job.get("completed", [])):
            raise ctl.ReleaseError("this run is not blocked at a primary pub.dev publication")
        component = step.removeprefix("publish:")
        if component not in publish_targets.PUB or component in {
                item["component"] for item in job.get("deferredPublications", [])}:
            raise ctl.ReleaseError("the blocked component has no pub.dev collision recovery")
        candidate = publisher.load_candidate(component, self.root, self.workspace, train_id)
        package = publish_targets.PUB[component]
        version = publish_targets._version(candidate, component)
        archive = publish_targets._single(candidate, component, f"{package}-{version}.tar.gz")
        data = publish_targets._pub_metadata(package, version)
        if data is None:
            raise ctl.ReleaseError("pub.dev has not published this version; resume the saved run normally")
        remote = publish_targets._pub_remote_archive(data)
        if publish_targets._pub_manifest(archive) == publish_targets._pub_manifest(remote):
            raise ctl.ReleaseError("pub.dev now serves the exact staged archive; resume the saved run normally")
        if (candidate["directory"] / "publication/published.json").exists():
            raise ctl.ReleaseError("publication evidence already exists; inspect before replanning")
        entry = next(item for item in candidate["catalog"]["components"] if item["id"] == component)
        source_version = ctl.component_version(entry, candidate["catalog"], self.workspace)
        base = max((item for item in (version, source_version) if item), key=flow.stable_tuple)
        major, minor, patch = flow.stable_tuple(base)
        suggested = None
        for offset in range(1, 101):
            proposal = f"{major}.{minor}.{patch + offset}"
            if publish_targets._pub_metadata(package, proposal) is None:
                suggested = proposal
                break
        if suggested is None:
            raise ctl.ReleaseError("the next 100 pub.dev patch versions are occupied")
        original_note = next(item["summary"] for item in job["intent"]["changes"]
                             if item["component"] == component)
        # The old Flutter SDK receipt uses the same over-inclusive archiver.
        # Move it to the successor too, before an irreversible upload occurs.
        rebuild = []
        if component == "sdk-flutter-generator" and "sdk-flutter" in {
                item["component"] for item in job["intent"]["changes"]}:
            flutter_step = "publish:sdk-flutter"
            if flutter_step not in job.get("completed", []):
                flutter = publisher.load_candidate("sdk-flutter", self.root, self.workspace, train_id)
                flutter_version = publish_targets._version(flutter, "sdk-flutter")
                flutter_archive = publish_targets._single(
                    flutter, "sdk-flutter", f"axiom_flutter-{flutter_version}.tar.gz")
                with tarfile.open(flutter_archive, "r:gz") as opened:
                    archive_files = {member.name.removeprefix("./") for member in opened if member.isfile()}
                if any(part.startswith(".") for name in archive_files for part in Path(name).parts):
                    rebuild.append("sdk-flutter")
        return {"trainId": train_id, "component": component, "package": package,
                "occupiedVersion": version, "nextVersion": suggested,
                "suggestedSummary": original_note.replace(version, suggested),
                "rebuildWithSuccessor": rebuild,
                "followupTrainId": job.get("followup", {}).get("trainId") or
                                   cycle.next_train_id(train_id, self.root)}

    def replan_npm_collision(self, train_id: str, version: str, summary: str,
                             confirmation: str) -> dict:
        """Explicitly defer one immutable npm collision to a new built train."""
        with self.lock:
            if self.worker and self.worker.is_alive():
                raise ctl.ReleaseError("a release is already running")
            recovery = self.npm_collision_recovery(train_id)
            component = recovery["component"]
            if not isinstance(version, str) or not versions.STABLE_VERSION.fullmatch(version):
                raise ctl.ReleaseError("enter a stable X.Y.Z successor version")
            if flow.stable_tuple(version) <= flow.stable_tuple(recovery["occupiedVersion"]):
                raise ctl.ReleaseError(f"successor version must be newer than {recovery['occupiedVersion']}")
            if not isinstance(summary, str) or not summary.strip() or len(summary) > 500 or "\n" in summary:
                raise ctl.ReleaseError("enter a one-line successor changelog note (up to 500 characters)")
            expected = f"REPLAN {train_id} {component} {version}"
            if confirmation != expected:
                raise ctl.ReleaseError(f"type {expected} to authorize this successor version")
            path = self._job_path(train_id)
            job = json.loads(path.read_text())
            if job.get("currentStep") != f"publish:{component}":
                raise ctl.ReleaseError("the blocked release step changed; review it again")
            catalog = ctl.read_catalog()
            entry = next(item for item in catalog["components"] if item["id"] == component)
            source_version = ctl.component_version(entry, catalog, self.workspace)
            if source_version and flow.stable_tuple(version) < flow.stable_tuple(source_version):
                raise ctl.ReleaseError(f"successor version must not be older than source {source_version}")
            remote_latest = self._npm_latest_version(publish_targets.NPM[component])
            if remote_latest and flow.stable_tuple(version) <= flow.stable_tuple(remote_latest):
                raise ctl.ReleaseError(f"successor version must be newer than npm's latest {remote_latest}")
            candidate_dir = self.root / "trains" / train_id / "components" / component
            npm_env = self._npm_public_environment()
            if publish_targets._npm_metadata(publish_targets.NPM[component], version,
                                             candidate_dir, npm_env) is not None:
                raise ctl.ReleaseError(f"npm {component}@{version} is already occupied; choose another version")
            change = next(item for item in job["intent"]["changes"] if item["component"] == component)
            followup = job.setdefault("followup", {"trainId": recovery["followupTrainId"],
                                                "changes": [], "order": [], "requiredRepos": ["AxiomCore"]})
            if "intent" in followup or any(item["component"] == component for item in followup["changes"]):
                raise ctl.ReleaseError("follow-up train was already planned; inspect before changing it")
            backup = self.root / "ui-runs/recovery-backups" / f"{train_id}-{component}.json"
            if backup.exists() or backup.is_symlink():
                raise ctl.ReleaseError(f"a recovery backup already exists; inspect {backup}")
            atomic_json(backup, job)
            revised_change = copy.deepcopy(change)
            revised_change["summary"] = summary.strip()
            followup["changes"].append(revised_change)
            followup["versionOverrides"] = {**followup.get("versionOverrides", {}), component: version}
            followup["order"] = release_groups(catalog, {item["component"] for item in followup["changes"]})
            required = set(followup["requiredRepos"])
            required.add(entry["owner"])
            required.update(source["repo"] for source in entry["sources"])
            followup["requiredRepos"] = sorted(required)
            job.setdefault("deferredPublications", []).append({
                "component": component, "occupiedVersion": recovery["occupiedVersion"],
                "nextVersion": version, "trainId": followup["trainId"]})
            job["completed"].append(f"publish:{component}")
            job["error"] = (f"{component}@{recovery['occupiedVersion']} remains unpublished in {train_id}. "
                            f"It will be rebuilt and published as {version} in {followup['trainId']}. "
                            "Resume to continue the other selected publications.")
            self._log(job, f"Operator deferred occupied npm {component}@{recovery['occupiedVersion']} "
                      f"to successor train {followup['trainId']} as {version}; original candidate remains intact")
            self._save_job(job)
            return {"trainId": train_id, "component": component, "nextVersion": version,
                    "followupTrainId": followup["trainId"], "status": "ready-to-resume"}

    def replan_pub_collision(self, train_id: str, version: str, summary: str,
                             confirmation: str) -> dict:
        """Keep occupied pub.dev receipts immutable and rebuild in the successor."""
        with self.lock:
            if self.worker and self.worker.is_alive():
                raise ctl.ReleaseError("a release is already running")
            recovery = self.pub_collision_recovery(train_id)
            component = recovery["component"]
            if not isinstance(version, str) or not versions.STABLE_VERSION.fullmatch(version):
                raise ctl.ReleaseError("enter a stable X.Y.Z successor version")
            if flow.stable_tuple(version) <= flow.stable_tuple(recovery["occupiedVersion"]):
                raise ctl.ReleaseError(f"successor version must be newer than {recovery['occupiedVersion']}")
            if not isinstance(summary, str) or not summary.strip() or len(summary) > 500 or "\n" in summary:
                raise ctl.ReleaseError("enter a one-line successor changelog note (up to 500 characters)")
            expected = f"REPLAN {train_id} {component} {version}"
            if confirmation != expected:
                raise ctl.ReleaseError(f"type {expected} to authorize this successor version")
            job = json.loads(self._job_path(train_id).read_text())
            if job.get("currentStep") != f"publish:{component}":
                raise ctl.ReleaseError("the blocked release step changed; review it again")
            catalog = ctl.read_catalog()
            entry = next(item for item in catalog["components"] if item["id"] == component)
            source_version = ctl.component_version(entry, catalog, self.workspace)
            if source_version and flow.stable_tuple(version) < flow.stable_tuple(source_version):
                raise ctl.ReleaseError(f"successor version must not be older than source {source_version}")
            package = publish_targets.PUB[component]
            if publish_targets._pub_metadata(package, version) is not None:
                raise ctl.ReleaseError(f"pub.dev {package}@{version} is occupied; choose another version")
            additions = [component, *recovery.get("rebuildWithSuccessor", [])]
            existing_followup = job.get("followup", {})
            if "intent" in existing_followup or any(
                    item["component"] in additions for item in existing_followup.get("changes", [])):
                raise ctl.ReleaseError("follow-up train was already planned; inspect before changing it")
            backup = self.root / "ui-runs/recovery-backups" / f"{train_id}-{component}.json"
            if backup.exists() or backup.is_symlink():
                raise ctl.ReleaseError(f"a recovery backup already exists; inspect {backup}")
            atomic_json(backup, job)
            followup = job.setdefault("followup", {"trainId": recovery["followupTrainId"],
                                                "changes": [], "order": [], "requiredRepos": ["AxiomCore"]})
            for target in additions:
                change = copy.deepcopy(next(item for item in job["intent"]["changes"]
                                            if item["component"] == target))
                if target == component:
                    change["summary"] = summary.strip()
                    followup["versionOverrides"] = {**followup.get("versionOverrides", {}), target: version}
                followup["changes"].append(change)
                owner = next(item for item in catalog["components"] if item["id"] == target)
                required = set(followup["requiredRepos"])
                required.add(owner["owner"])
                required.update(source["repo"] for source in owner["sources"])
                followup["requiredRepos"] = sorted(required)
                job.setdefault("deferredPublications", []).append({
                    "component": target, "occupiedVersion": recovery["occupiedVersion"] if target == component else None,
                    "nextVersion": version if target == component else change.get("version"),
                    "trainId": followup["trainId"]})
                job["completed"].append(f"publish:{target}")
            followup["order"] = release_groups(catalog, {item["component"] for item in followup["changes"]})
            job["error"] = (f"{component}@{recovery['occupiedVersion']} remains unpublished in {train_id}. "
                            f"It will be rebuilt as {version} in {followup['trainId']}. "
                            "Resume to continue the other selected publications.")
            self._log(job, f"Operator deferred occupied pub.dev {package}@{recovery['occupiedVersion']} "
                      f"to successor train {followup['trainId']} as {version}; "
                      f"also rebuilding {', '.join(recovery.get('rebuildWithSuccessor', [])) or 'no other packages'}")
            self._save_job(job)
            return {"trainId": train_id, "component": component, "nextVersion": version,
                    "followupTrainId": followup["trainId"], "status": "ready-to-resume"}

    def _step(self, job: dict, name: str, action) -> None:
        if name in job["completed"]:
            return
        job["currentStep"] = name
        self._save_job(job)
        self._log(job, f"Starting {name}")
        action()
        job["completed"].append(name)
        self._log(job, f"Completed {name}")
        self._save_job(job)

    def _command(self, job: dict, *args: str, cwd: Path | None = None) -> None:
        self._log(job, "Running " + " ".join(args))
        environment = dict(os.environ)
        environment["TERM"] = "dumb"
        process = subprocess.Popen(args, cwd=cwd or self.workspace / "AxiomCore", env=environment,
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, bufsize=0)
        job["childPid"] = process.pid
        self._save_job(job)
        assert process.stdout is not None
        with self._log_path(job["trainId"]).open("ab") as output:
            for chunk in iter(lambda: process.stdout.read(65536), b""):
                output.write(chunk)
                output.flush()
        code = process.wait()
        job.pop("childPid", None)
        self._save_job(job)
        if code:
            raise ctl.ReleaseError(f"command exited {code}: {' '.join(args)}; see the saved log")

    def _create_cycle(self, job: dict, catalog: dict) -> None:
        phase = self._phase(job)
        train_id = phase["trainId"]
        intent_path = ctl.CONTROL_DIR / "intent.json"
        old_intent = phase["oldIntent"].encode()
        old_ledger = phase["oldLedger"].encode()
        if (self.root / "trains" / train_id / "cycle-backup").is_dir():
            if json.loads(intent_path.read_text()) == phase["intent"] and json.loads(versions.VERSIONS.read_text()) == phase["ledger"]:
                return
            raise ctl.ReleaseError("cycle backup exists but source files differ; inspect before retrying")
        cycle.save_cycle(self.root, train_id, intent_path, versions.VERSIONS,
                         old_intent, old_ledger, phase["intent"], phase["ledger"])

    def _prepare(self, job: dict, catalog: dict) -> None:
        phase = self._phase(job)
        normalized = flow.validate_intent(copy.deepcopy(phase["intent"]), catalog, phase["ledger"])
        evidence = self.root / "trains" / phase["trainId"] / "preparation.json"
        if evidence.is_file():
            previous = json.loads(evidence.read_text())
            if release_cli.prepared_intent_matches(previous, normalized, catalog, phase["ledger"]):
                return
            raise ctl.ReleaseError("preparation evidence differs from this run; inspect before retrying")
        report, edits, fragments = flow.make_preparation(normalized, catalog, self.workspace)
        if report["blocked"]:
            raise ctl.ReleaseError("preparation blocked: " + "; ".join(report["blocked"]))
        backup = flow.apply_preparation(report, edits, fragments, catalog, self.workspace)
        report["applied"] = True
        report["backup"] = str(backup)
        ctl.write_json(evidence, report)

    def _commit_managed(self, job: dict, catalog: dict) -> None:
        phase = self._phase(job)
        paths_by_repo: dict[str, set[str]] = {}
        for absolute in phase["managedPaths"]:
            path = Path(absolute)
            owner = flow.containing_repo(catalog, self.workspace, path)
            name = next(name for name in catalog["repositories"] if self._repo(catalog, name) == owner)
            paths_by_repo.setdefault(name, set()).add(str(path.relative_to(owner)))
        for name, paths in sorted(paths_by_repo.items()):
            repository = self._repo(catalog, name)
            changed = changed_files(repository)
            unexpected = {item["path"] for item in changed} - paths
            if unexpected:
                raise ctl.ReleaseError(f"{name} gained unrelated changes during preparation: {', '.join(sorted(unexpected))}")
            dirty = sorted({item["path"] for item in changed} & paths)
            if not dirty:
                continue
            staged = git(repository, "diff", "--cached", "--name-only")
            if staged and set(staged.splitlines()) - paths:
                raise ctl.ReleaseError(f"{name} has unrelated staged files; inspect before committing")
            git(repository, "add", "-A", "--", *dirty)
            git(repository, "commit", "-m", f"Prepare release train {phase['trainId']}")
            self._log(job, f"Committed {name}: {git(repository, 'rev-parse', 'HEAD')}")

    def _push_sources(self, job: dict, catalog: dict) -> None:
        for name in self._phase(job)["requiredRepos"]:
            repository = self._repo(catalog, name)
            if changed_files(repository):
                raise ctl.ReleaseError(f"{name} has uncommitted files; review them before pushing")
            upstream = git(repository, "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}")
            if "/" not in upstream:
                raise ctl.ReleaseError(f"{name} has no tracked remote branch")
            remote, branch = upstream.split("/", 1)
            head = git(repository, "rev-parse", "HEAD")
            remote_line = git(repository, "ls-remote", remote, f"refs/heads/{branch}")
            remote_head = remote_line.split()[0] if remote_line else ""
            if remote_head == head:
                self._log(job, f"{name}: remote already matches {head[:12]}")
                continue
            if not remote_head:
                raise ctl.ReleaseError(f"{name}: tracked remote branch is missing")
            ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", remote_head, head],
                                      cwd=repository, check=False)
            if ancestor.returncode:
                raise ctl.ReleaseError(f"{name}: remote branch diverged; no automatic push was attempted")
            self._command(job, "git", "push", remote, f"HEAD:refs/heads/{branch}", cwd=repository)
            verified = git(repository, "ls-remote", remote, f"refs/heads/{branch}").split()[0]
            if verified != head:
                raise ctl.ReleaseError(f"{name}: remote tip changed after push")

    def _build(self, job: dict, group: str) -> None:
        train_id = self._phase(job)["trainId"]
        directory = self.root / "trains" / train_id / "components" / group
        if (directory / "staged.json").is_file():
            publisher.load_candidate(group, self.root, self.workspace, train_id)
            self._log(job, f"Reusing verified staged candidate for {group}")
            return
        self._command(job, sys.executable, "-u", "-B", str(ctl.CONTROL_DIR / "release_cli.py"),
                      group, cwd=self.workspace / "AxiomCore")
        publisher.load_candidate(group, self.root, self.workspace, train_id)

    def _publish(self, job: dict, group: str) -> None:
        self._command(job, sys.executable, "-u", "-B", str(ctl.CONTROL_DIR / "release_cli.py"),
                      "publish", group, self._phase(job)["trainId"], cwd=self.workspace / "AxiomCore")

    def _pin_followup(self, job: dict, catalog: dict) -> None:
        consumers = {item["component"] for item in job["followup"]["changes"]}
        allowed = {"axiom-sdk": set(),
                   "atmx-react": {"package.json", "package-lock.json"}}
        if "sdk-swift" in consumers:
            allowed["axiom-sdk"].add("swift/Package.swift")
        if "sdk-flutter" in consumers:
            allowed["axiom-sdk"].update({
                "flutter/axiom_flutter/ios/axiom_flutter.podspec",
                "flutter/axiom_flutter/macos/axiom_flutter.podspec",
            })
        for name in ("axiom-sdk", "atmx-react"):
            if not ({"sdk-swift", "sdk-flutter"} if name == "axiom-sdk" else {"sdk-atmx-react"}) & consumers:
                continue
            repository = self._repo(catalog, name)
            dirty = {item["path"] for item in changed_files(repository)}
            if dirty:
                backup = self.root / "trains" / job["trainId"] / "auto-pins" / name
                if dirty - allowed[name] or not backup.is_dir():
                    raise ctl.ReleaseError(f"{name} changed outside the saved automatic pin step; review source before resuming")
                for relative in dirty:
                    saved = backup / relative
                    committed = ctl.run("git", "show", f"HEAD:{relative}", cwd=repository)
                    if not saved.is_file() or saved.is_symlink() or saved.read_bytes() != committed:
                        raise ctl.ReleaseError(f"{name}/{relative} changed after the automatic pin backup; review it before resuming")
                for consumer in ({"sdk-swift", "sdk-flutter"} if name == "axiom-sdk" else {"sdk-atmx-react"}) & consumers:
                    if consumer == "sdk-flutter":
                        version = job["ledger"]["components"]["runtime-apple"]["candidateVersion"]
                        for relative in dirty & {
                            "flutter/axiom_flutter/ios/axiom_flutter.podspec",
                            "flutter/axiom_flutter/macos/axiom_flutter.podspec",
                        }:
                            saved = backup / relative
                            expected = auto_pins.pin_flutter_podspec(saved.read_text(), version)
                            if (repository / relative).read_text() != expected:
                                raise ctl.ReleaseError(f"{name}/{relative} changed outside the automatic Flutter pin")
                    elif ((consumer == "sdk-swift" and "swift/Package.swift" in dirty)
                          or consumer == "sdk-atmx-react") and ci_builders.dependency_blockers(
                              {consumer}, job["ledger"], self.workspace):
                        raise ctl.ReleaseError(f"{name} has a partial or manually changed dependency pin; inspect the saved backup")
        changed = auto_pins.pin_verified_consumers(self.root, self.workspace, catalog,
                                                   job["trainId"], job["ledger"], consumers)
        issues = ci_builders.dependency_blockers(consumers, job["ledger"], self.workspace)
        if issues:
            raise ctl.ReleaseError("automatic dependency pins are incomplete: " + "; ".join(issues))
        for name, paths in sorted(allowed.items()):
            if not ({"sdk-swift", "sdk-flutter"} if name == "axiom-sdk" else {"sdk-atmx-react"}) & consumers:
                continue
            repository = self._repo(catalog, name)
            dirty = {item["path"] for item in changed_files(repository)}
            if dirty - paths:
                raise ctl.ReleaseError(f"{name} gained unrelated source changes while pinning")
            if dirty:
                git(repository, "add", "-A", "--", *sorted(dirty))
                git(repository, "commit", "--only", "-m",
                    f"Pin verified dependencies for release {job['followup']['trainId']}",
                    "--", *sorted(dirty))
                self._log(job, f"Pinned and committed {name}: {', '.join(sorted(dirty))}")
            elif name in changed:
                raise ctl.ReleaseError(f"{name} pin changed but no reviewable source change remains")

    def _plan_followup(self, job: dict, catalog: dict) -> None:
        followup = job["followup"]
        if "intent" in followup:
            return
        old_intent_text = (ctl.CONTROL_DIR / "intent.json").read_text()
        old_ledger_text = versions.VERSIONS.read_text()
        if json.loads(old_intent_text) != job["intent"] or json.loads(old_ledger_text) != job["ledger"]:
            raise ctl.ReleaseError("active release metadata changed before the automatic SDK phase")
        published = cycle.published_evidence(self.root, catalog, job["trainId"])
        summary = f"Complete verified SDK dependency pins from release {job['trainId']}"
        updated_ledger = copy.deepcopy(job["ledger"])
        for component, version in followup.get("versionOverrides", {}).items():
            updated_ledger["components"][component]["candidateVersion"] = version
        versions.validate_versions(updated_ledger, catalog)
        intent = cycle.compose_intent(job["intent"], followup["trainId"],
                                      followup["changes"], summary, published)
        normalized = flow.validate_intent(copy.deepcopy(intent), catalog, updated_ledger)
        report, edits, fragments = flow.make_preparation(normalized, catalog, self.workspace)
        if report["blocked"]:
            raise ctl.ReleaseError("automatic SDK preparation blocked: " + "; ".join(report["blocked"]))
        required = {"AxiomCore"}
        for change in followup["changes"]:
            entry = next(item for item in catalog["components"] if item["id"] == change["component"])
            required.add(entry["owner"])
            required.update(source["repo"] for source in entry["sources"])
            for dependency in entry.get("depends_on", []):
                predecessor = next(item for item in catalog["components"] if item["id"] == dependency)
                required.update(source["repo"] for source in predecessor["sources"])
        followup.update({"intent": intent, "ledger": updated_ledger,
                         "oldIntent": old_intent_text, "oldLedger": old_ledger_text,
                         "requiredRepos": sorted(required),
                         "managedPaths": [str(ctl.CONTROL_DIR / "intent.json"),
                                          str(versions.VERSIONS),
                                          *(str(path) for path in edits),
                                          *(str(path) for path in fragments)]})
        self._save_job(job)
        self._log(job, f"Automatic SDK phase prepared as internal train {followup['trainId']}; source pins were committed first")

    def _run(self, job: dict) -> None:
        try:
            catalog = ctl.read_catalog()
            self._step(job, "cycle", lambda: self._create_cycle(job, catalog))
            self._step(job, "prepare", lambda: self._prepare(job, catalog))
            self._step(job, "commit", lambda: self._commit_managed(job, catalog))
            self._step(job, "push", lambda: self._push_sources(job, catalog))
            for group in job["order"]:
                self._step(job, f"build:{group}", lambda target=group: self._build(job, target))
            for group in job["order"]:
                self._step(job, f"publish:{group}", lambda target=group: self._publish(job, target))
            if job.get("followup"):
                job["phase"] = "followup"
                self._save_job(job)
                self._step(job, "followup:pins", lambda: self._pin_followup(job, catalog))
                self._step(job, "followup:plan", lambda: self._plan_followup(job, catalog))
                self._step(job, "followup:cycle", lambda: self._create_cycle(job, catalog))
                self._step(job, "followup:prepare", lambda: self._prepare(job, catalog))
                self._step(job, "followup:commit", lambda: self._commit_managed(job, catalog))
                self._step(job, "followup:push", lambda: self._push_sources(job, catalog))
                for group in job["followup"]["order"]:
                    self._step(job, f"followup:build:{group}", lambda target=group: self._build(job, target))
                for group in job["followup"]["order"]:
                    self._step(job, f"followup:publish:{group}", lambda target=group: self._publish(job, target))
            job["status"] = "complete"
            job["currentStep"] = None
            self._log(job, "Release complete. Every selected component has remotely verified publication evidence.")
        except Exception as error:
            job["status"] = "blocked"
            job["error"] = str(error)
            self._log(job, f"Stopped safely: {error}")
        finally:
            self._save_job(job)


def handler_for(dashboard: ReleaseDashboard):
    class Handler(BaseHTTPRequestHandler):
        server_version = "AxiomReleaseDashboard/1"

        def _headers(self, code: int, mime: str, length: int) -> None:
            self.send_response(code)
            self.send_header("Content-Type", mime)
            self.send_header("Content-Length", str(length))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("X-Frame-Options", "DENY")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("Content-Security-Policy",
                             "default-src 'none'; script-src 'self'; style-src 'self'; "
                             "connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'")
            self.end_headers()

        def _json(self, code: int, value: dict) -> None:
            raw = json.dumps(value, ensure_ascii=False).encode()
            try:
                self._headers(code, "application/json; charset=utf-8", len(raw))
                self.wfile.write(raw)
            except (BrokenPipeError, ConnectionResetError):
                # A browser refresh can cancel a slow source-status request.
                return

        def _authorized(self, mutation: bool = False) -> bool:
            if not secrets.compare_digest(self.headers.get("X-Axiom-Release-Token", ""), dashboard.token):
                self._json(HTTPStatus.FORBIDDEN, {"error": "invalid dashboard token; reload this page"})
                return False
            if mutation:
                expected = f"http://127.0.0.1:{self.server.server_port}"
                if self.headers.get("Origin") != expected:
                    self._json(HTTPStatus.FORBIDDEN, {"error": "release actions require this loopback page"})
                    return False
            return True

        def do_GET(self) -> None:
            parsed = urlsplit(self.path)
            if parsed.path in ("/", "/index.html"):
                raw = (ASSETS / "index.html").read_text().replace("__RELEASE_TOKEN__", dashboard.token).encode()
                self._headers(HTTPStatus.OK, "text/html; charset=utf-8", len(raw))
                self.wfile.write(raw)
                return
            assets = {"/app.js": ("app.js", "text/javascript; charset=utf-8"),
                      "/style.css": ("style.css", "text/css; charset=utf-8")}
            if parsed.path in assets:
                name, mime = assets[parsed.path]
                raw = (ASSETS / name).read_bytes()
                self._headers(HTTPStatus.OK, mime, len(raw))
                self.wfile.write(raw)
                return
            if not self._authorized():
                return
            try:
                if parsed.path == "/api/state":
                    result = dashboard.state()
                elif parsed.path == "/api/job":
                    query = parse_qs(parsed.query)
                    result = {"job": dashboard.job(query.get("train", [None])[0])}
                elif parsed.path == "/api/npm-recovery":
                    query = parse_qs(parsed.query)
                    result = dashboard.npm_collision_recovery(query.get("train", [""])[0])
                elif parsed.path == "/api/pub-recovery":
                    query = parse_qs(parsed.query)
                    result = dashboard.pub_collision_recovery(query.get("train", [""])[0])
                elif parsed.path == "/api/diff":
                    query = parse_qs(parsed.query)
                    result = dashboard.diff(query.get("repo", [""])[0], query.get("path", [""])[0])
                else:
                    self._json(HTTPStatus.NOT_FOUND, {"error": "not found"})
                    return
                self._json(HTTPStatus.OK, result)
            except (ctl.ReleaseError, OSError, ValueError, KeyError) as error:
                self._json(HTTPStatus.BAD_REQUEST, {"error": str(error)})

        def do_POST(self) -> None:
            if not self._authorized(mutation=True):
                return
            if self.headers.get("Content-Type", "").split(";", 1)[0] != "application/json":
                self._json(HTTPStatus.UNSUPPORTED_MEDIA_TYPE, {"error": "send application/json"})
                return
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if size < 2 or size > MAX_REQUEST:
                    raise ctl.ReleaseError("invalid or oversized release request")
                value = json.loads(self.rfile.read(size))
                if not isinstance(value, dict):
                    raise ctl.ReleaseError("release request must be a JSON object")
                path = urlsplit(self.path).path
                if path == "/api/preview":
                    result = dashboard.preview(value)
                elif path == "/api/start":
                    result = dashboard.start(value.get("previewId", ""), value.get("confirmation", ""))
                elif path == "/api/resume":
                    result = dashboard.resume(value.get("trainId", ""), value.get("confirmation", ""))
                elif path == "/api/npm-recovery":
                    result = dashboard.replan_npm_collision(value.get("trainId", ""),
                                                            value.get("version", ""),
                                                            value.get("summary", ""),
                                                            value.get("confirmation", ""))
                elif path == "/api/pub-recovery":
                    result = dashboard.replan_pub_collision(value.get("trainId", ""),
                                                            value.get("version", ""),
                                                            value.get("summary", ""),
                                                            value.get("confirmation", ""))
                elif path == "/api/commit":
                    result = dashboard.commit_reviewed(value.get("repository", ""),
                                                       value.get("paths", []), value.get("message", ""))
                elif path == "/api/push":
                    result = dashboard.push_committed(value.get("repositories", []))
                else:
                    self._json(HTTPStatus.NOT_FOUND, {"error": "not found"})
                    return
                self._json(HTTPStatus.OK, result)
            except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
                self._json(HTTPStatus.BAD_REQUEST, {"error": str(error)})

        def log_message(self, format: str, *args) -> None:
            # Avoid logging the token or request bodies. Paths are fixed above.
            print(f"release-web: {format % args}", file=sys.stderr)

    return Handler


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Local Axiom release dashboard")
    parser.add_argument("--port", type=int, default=8716)
    args = parser.parse_args(argv)
    if not 0 <= args.port <= 65535:
        parser.error("port must be between 0 and 65535")
    dashboard = ReleaseDashboard()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler_for(dashboard))
    print(f"Release dashboard: http://127.0.0.1:{server.server_port}/", flush=True)
    print("Loopback only. Keep this process running while the release runs; work is checkpointed on the release SSD.", flush=True)
    try:
        server.serve_forever(poll_interval=0.25)
    except KeyboardInterrupt:
        print("Stopping dashboard. Any in-progress step must finish or be inspected before resume.", flush=True)
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
