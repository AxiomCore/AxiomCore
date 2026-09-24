#!/usr/bin/env python3
"""Read-only-by-default planner for AxiomCore's platform releases.

This deliberately does not publish, tag, deploy, or edit source repositories.
Only `build` runs a narrowly declared, local build adapter; receipts and
archives from that adapter must live on the configured external build volume.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import tomllib


CONTROL_DIR = Path(__file__).resolve().parent
WORKSPACE = CONTROL_DIR.parents[2]
CATALOG = CONTROL_DIR / "catalog.toml"
PLAN_FORMAT = "axiom-platform-release-plan/v1"
RECEIPT_FORMAT = "axiom-platform-build-receipt/v1"
EXTERNAL_SSD = Path("/Volumes/ExternalSSD")
DEFAULT_BUILD_IMAGE = EXTERNAL_SSD / "AxiomReleaseBuild.sparsebundle"
DEFAULT_BUILD_MOUNT = Path("/Volumes/AxiomReleaseBuild")
DEFAULT_BUILD_ROOT = DEFAULT_BUILD_MOUNT / "axiom-release"


class ReleaseError(Exception):
    pass


def run(*args: str, cwd: Path | None = None, env: dict[str, str] | None = None,
        capture: bool = True) -> bytes:
    completed = subprocess.run(args, cwd=cwd, env=env, check=False,
                               stdout=subprocess.PIPE if capture else None,
                               stderr=subprocess.PIPE if capture else None)
    if completed.returncode:
        detail = (completed.stderr or b"").decode(errors="replace").strip()
        raise ReleaseError(f"{' '.join(args)} failed ({completed.returncode}): {detail}")
    return completed.stdout or b""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def read_catalog(path: Path = CATALOG) -> dict:
    raw = path.read_bytes()
    catalog = tomllib.loads(raw.decode())
    if catalog.get("format") != "axiom-platform-release-catalog/v1":
        raise ReleaseError("unsupported release catalog format")
    repositories = catalog.get("repositories", {})
    components = catalog.get("components", [])
    if not isinstance(repositories, dict) or not isinstance(components, list):
        raise ReleaseError("release catalog has malformed repositories or components")
    ids = [component.get("id") for component in components]
    if len(ids) != len(set(ids)) or any(not isinstance(id_, str) for id_ in ids):
        raise ReleaseError("release component IDs must be unique strings")
    for component in components:
        if component.get("owner") not in repositories:
            raise ReleaseError(f"unknown owner for {component['id']}")
        if not component.get("sources"):
            raise ReleaseError(f"{component['id']} has no declared source inputs")
        for source in component["sources"]:
            if source.get("repo") not in repositories or not source.get("paths"):
                raise ReleaseError(f"{component['id']} has an invalid source selector")
            for prefix in source["paths"]:
                if not isinstance(prefix, str) or prefix.startswith("/") or ".." in Path(prefix).parts:
                    raise ReleaseError(f"unsafe source selector {prefix!r} in {component['id']}")
        for dependency in component.get("depends_on", []):
            if dependency not in ids or dependency == component["id"]:
                raise ReleaseError(f"invalid dependency {dependency!r} in {component['id']}")
        required = component.get("required_artifacts", [])
        if (not isinstance(required, list)
                or any(not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name)
                       for name in required)
                or len(required) != len(set(required))):
            raise ReleaseError(f"invalid required artifact names in {component['id']}")
    topological_components(components)
    catalog["sha256"] = sha256(raw)
    return catalog


def topological_components(components: list[dict]) -> list[dict]:
    by_id = {component["id"]: component for component in components}
    ordered: list[dict] = []
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(component_id: str) -> None:
        if component_id in visiting:
            raise ReleaseError(f"release dependency cycle at {component_id}")
        if component_id in visited:
            return
        visiting.add(component_id)
        for dependency in by_id[component_id].get("depends_on", []):
            visit(dependency)
        visiting.remove(component_id)
        visited.add(component_id)
        ordered.append(by_id[component_id])

    for component in components:
        visit(component["id"])
    return ordered


def repo_path(catalog: dict, repo: str, workspace: Path = WORKSPACE) -> Path:
    candidate = (workspace / catalog["repositories"][repo]).resolve()
    if candidate != workspace.resolve() and workspace.resolve() not in candidate.parents:
        raise ReleaseError(f"repository {repo} escapes the workspace")
    if not (candidate / ".git").exists():
        raise ReleaseError(f"repository {repo} is absent at {candidate}")
    return candidate


def git_snapshot(path: Path) -> dict:
    head = run("git", "rev-parse", "HEAD", cwd=path).decode().strip()
    dirty_paths: set[str] = set()
    status = run("git", "status", "--porcelain=v1", "-z", "--untracked-files=all", cwd=path).split(b"\0")
    index = 0
    while index < len(status):
        record = status[index]
        index += 1
        if not record:
            continue
        flags = record[:2]
        dirty_paths.add(record[3:].decode("utf-8", "surrogateescape"))
        if b"R" in flags or b"C" in flags:
            if index < len(status):
                dirty_paths.add(status[index].decode("utf-8", "surrogateescape"))
                index += 1
    entries: dict[str, str] = {}
    for record in run("git", "ls-tree", "-rz", "--full-tree", "HEAD", cwd=path).split(b"\0"):
        if not record:
            continue
        metadata, filename = record.split(b"\t", 1)
        mode, kind, object_id = metadata.split(b" ", 2)
        if kind == b"blob":
            entries[filename.decode("utf-8", "surrogateescape")] = f"{mode.decode()}:{object_id.decode()}"
    return {"head": head, "dirtyPaths": dirty_paths, "entries": entries}


def matches(filename: str, prefix: str) -> bool:
    if prefix in ("", "."):
        return True
    if prefix.endswith("/"):
        return filename.startswith(prefix)
    return filename == prefix


def source_fingerprint(component: dict, snapshots: dict[str, dict]) -> tuple[str, list[dict]]:
    source_groups = []
    for source in component["sources"]:
        repo = source["repo"]
        prefixes = source["paths"]
        for prefix in prefixes:
            if not any(matches(name, prefix) for name in snapshots[repo]["entries"]) and not any(
                    matches(name, prefix) for name in snapshots[repo]["dirtyPaths"]):
                raise ReleaseError(f"{component['id']} selector {repo}:{prefix} matches no source files")
        entries = [(name, object_id) for name, object_id in snapshots[repo]["entries"].items()
                   if any(matches(name, prefix) for prefix in prefixes)]
        dirty = any(any(matches(path, prefix) for prefix in prefixes)
                    for path in snapshots[repo]["dirtyPaths"])
        digest = sha256(canonical(sorted(entries)))
        source_groups.append({"repo": repo, "paths": prefixes, "sha256": digest,
                              "files": len(entries), "dirty": dirty})
    return sha256(canonical(source_groups)), source_groups


def read_baseline(path: Path | None) -> dict:
    if path is None:
        return {}
    baseline = json.loads(path.read_text())
    if baseline.get("format") != "axiom-platform-release-manifest/v1" or baseline.get("status") != "published":
        raise ReleaseError("baseline must declare a published platform release; authenticate its origin before use")
    components = baseline.get("components")
    if not isinstance(components, list):
        raise ReleaseError("baseline has no component list")
    indexed = {}
    for component in components:
        if not isinstance(component, dict) or not isinstance(component.get("id"), str):
            raise ReleaseError("baseline has a malformed component")
        component_id = component["id"]
        if component_id in indexed:
            raise ReleaseError(f"duplicate component in baseline: {component_id}")
        fingerprint = component.get("fingerprint")
        if not isinstance(fingerprint, str) or not re.fullmatch(r"[0-9a-f]{64}", fingerprint):
            raise ReleaseError(f"invalid fingerprint in baseline: {component_id}")
        artifacts = component.get("artifacts", [])
        if not isinstance(artifacts, list):
            raise ReleaseError(f"invalid artifact list in baseline: {component_id}")
        for artifact in artifacts:
            if (not isinstance(artifact, dict)
                    or not isinstance(artifact.get("file"), str)
                    or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", artifact["file"])
                    or not re.fullmatch(r"[0-9a-f]{64}", str(artifact.get("sha256", "")))):
                raise ReleaseError(f"invalid artifact in baseline: {component_id}")
        indexed[component_id] = component
    return indexed


def component_version(component: dict, catalog: dict, workspace: Path) -> str | None:
    source = component.get("version")
    if not source:
        return None
    path = repo_path(catalog, component["owner"], workspace) / source
    if not path.is_file():
        raise ReleaseError(f"version file is missing for {component['id']}: {path}")
    if path.suffix == ".toml":
        document = tomllib.loads(path.read_text())
        package = document.get("package") or document.get("tool", {}).get("poetry")
        if isinstance(package, dict) and package.get("version"):
            return str(package["version"])
    if path.suffix == ".json":
        return str(json.loads(path.read_text())["version"])
    if path.suffix in (".yaml", ".yml"):
        match = re.search(r"^version:\s*([^\s#]+)", path.read_text(), re.MULTILINE)
        if match:
            return match.group(1)
    raise ReleaseError(f"unsupported version source for {component['id']}: {path}")


def make_plan(catalog: dict, workspace: Path = WORKSPACE, baseline: dict | None = None,
              only: set[str] | None = None, force: set[str] | None = None) -> dict:
    ordered = topological_components(catalog["components"])
    ids = {component["id"] for component in ordered}
    if only and not only <= ids:
        raise ReleaseError(f"unknown components: {', '.join(sorted(only - ids))}")
    forced = force or set()
    if not forced <= ids:
        raise ReleaseError(f"unknown forced components: {', '.join(sorted(forced - ids))}")
    scope = set(ids if not only else only)
    by_id = {component["id"]: component for component in ordered}
    for component_id in tuple(scope):
        stack = [component_id]
        while stack:
            for dependency in by_id[stack.pop()].get("depends_on", []):
                if dependency not in scope:
                    scope.add(dependency)
                    stack.append(dependency)
    ordered = [component for component in ordered if component["id"] in scope]
    if not forced <= scope:
        raise ReleaseError("forced components must be included in the requested scope")
    forced = set(forced)
    for component in ordered:
        if any(dependency in forced for dependency in component.get("depends_on", [])):
            forced.add(component["id"])
    required_repos = {source["repo"] for component in ordered for source in component["sources"]}
    snapshots = {repo: git_snapshot(repo_path(catalog, repo, workspace)) for repo in sorted(required_repos)}
    previous = baseline or {}
    fingerprints: dict[str, str] = {}
    results = []
    for component in ordered:
        component_id = component["id"]
        direct_hash, sources = source_fingerprint(component, snapshots)
        definition = {key: value for key, value in component.items() if key != "destination"}
        fingerprint = sha256(canonical({"definition": definition, "inputs": direct_hash,
                                       "dependencies": {dep: fingerprints[dep] for dep in component.get("depends_on", [])}}))
        fingerprints[component_id] = fingerprint
        old = previous.get(component_id, {})
        same = old.get("fingerprint") == fingerprint
        # A matching input hash is not sufficient to reuse a missing or
        # unverified artifact. An explicit receipt/manifest is required.
        artifacts = old.get("artifacts") or []
        source_dirty = any(group["dirty"] for group in sources)
        reusable = (component_id not in forced and same and not source_dirty and bool(artifacts)
                    and all(item.get("sha256") and item.get("file") for item in artifacts))
        version = component_version(component, catalog, workspace)
        version_blocked = bool(old and not reusable and version is not None and old.get("version") == version)
        if source_dirty:
            reason = "source repository has uncommitted changes"
        elif version_blocked:
            reason = f"version {version} already belongs to the published baseline"
        elif not old:
            reason = "no baseline release"
        elif component_id in forced:
            reason = "operator forced rebuild for external configuration or validation"
        elif not same:
            reason = "declared source or dependency inputs changed"
        elif not reusable:
            reason = "matching inputs but no reusable artifact receipt"
        else:
            reason = "matching inputs and reusable artifact receipt"
        results.append({"id": component_id, "owner": component["owner"], "kind": component["kind"],
                        "adapter": component["adapter"], "destination": component["destination"],
                        "dependsOn": component.get("depends_on", []),
                        "version": version,
                        "fingerprint": fingerprint, "sourceGroups": sources,
                        "forced": component_id in forced,
                        "blocked": source_dirty or version_blocked,
                        "versionBlocked": version_blocked,
                        "selected": not reusable,
                        "reuseCandidate": reusable,
                        "reason": reason, "artifacts": artifacts if reusable else []})
    blocked_repos = sorted({group["repo"] for item in results for group in item["sourceGroups"] if group["dirty"]})
    blocked_versions = sorted(item["id"] for item in results if item["versionBlocked"])
    return {"format": PLAN_FORMAT, "catalogSha256": catalog["sha256"],
            "forced": sorted(forced),
            "repositories": {repo: {"head": value["head"], "dirty": repo in blocked_repos}
                             for repo, value in snapshots.items()},
            "components": results,
            "blocked": blocked_repos, "blockedVersions": blocked_versions}


def write_new_text(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x") as destination:
            destination.write(value)
    except FileExistsError as error:
        raise ReleaseError(f"refusing to overwrite existing release evidence: {path}") from error


def write_json(path: Path, value: dict) -> None:
    write_new_text(path, json.dumps(value, indent=2, sort_keys=True) + "\n")


def default_image_mount_points() -> list[str]:
    images = plistlib.loads(run("hdiutil", "info", "-plist")).get("images", [])
    return [entity["mount-point"] for image in images
            if Path(image.get("image-path", "")).resolve() == DEFAULT_BUILD_IMAGE.resolve()
            for entity in image.get("system-entities", [])
            if entity.get("mount-point")]


def default_image_is_mounted() -> bool:
    return str(DEFAULT_BUILD_MOUNT) in default_image_mount_points()


def mount_default_build_root() -> Path:
    if platform.system() != "Darwin":
        raise ReleaseError("the local release build volume can only be mounted on macOS")
    if not DEFAULT_BUILD_IMAGE.is_dir():
        raise ReleaseError(f"release image is missing; do not create or overwrite another path: {DEFAULT_BUILD_IMAGE}")
    ssd_info = plistlib.loads(run("diskutil", "info", "-plist", str(EXTERNAL_SSD)))
    if ssd_info.get("MountPoint") != str(EXTERNAL_SSD) or ssd_info.get("Internal") is not False:
        raise ReleaseError("ExternalSSD is not the expected mounted external disk")
    if DEFAULT_BUILD_MOUNT.exists():
        if not default_image_is_mounted():
            raise ReleaseError(f"{DEFAULT_BUILD_MOUNT} already exists but is not this release image")
    else:
        other_mounts = default_image_mount_points()
        if other_mounts:
            raise ReleaseError(f"release image is already attached at {', '.join(other_mounts)}; inspect before remounting")
        run("hdiutil", "attach", "-nobrowse", "-mountpoint", str(DEFAULT_BUILD_MOUNT),
            str(DEFAULT_BUILD_IMAGE), capture=False)
        if not default_image_is_mounted():
            raise ReleaseError("release image attached without the expected mount point")
    DEFAULT_BUILD_ROOT.mkdir(exist_ok=True)
    return require_external_build_root()


def require_external_build_root() -> Path:
    github_ci = os.environ.get("CI") == "true" and os.environ.get("GITHUB_ACTIONS") == "true"
    configured = os.environ.get("AXIOM_RELEASE_BUILD_ROOT", "")
    if not configured and github_ci:
        configured = os.environ.get("RUNNER_TEMP", "")
    if not configured:
        configured = str(DEFAULT_BUILD_ROOT)
    root = Path(configured).expanduser().resolve()
    if not root.is_absolute() or not root.exists() or not root.is_dir():
        if not github_ci and root == DEFAULT_BUILD_ROOT:
            raise ReleaseError("default release build volume is not mounted; run 'just release-mount'")
        raise ReleaseError("AXIOM_RELEASE_BUILD_ROOT must be an existing directory")
    if not github_ci:
        if platform.system() != "Darwin":
            raise ReleaseError("local release builds require an explicitly configured external volume")
        containing_mount = root
        while not os.path.ismount(containing_mount) and containing_mount != containing_mount.parent:
            containing_mount = containing_mount.parent
        info = plistlib.loads(run("diskutil", "info", "-plist", str(containing_mount)))
        mount = Path(info.get("MountPoint") or "/")
        fs_name = str(info.get("FilesystemName") or info.get("FilesystemType") or "")
        if mount == Path("/") or not str(mount).startswith("/Volumes/"):
            raise ReleaseError("release build root is not on a mounted external volume")
        if info.get("Internal") is True:
            raise ReleaseError("release build root is on internal media")
        if "APFS" not in fs_name.upper():
            raise ReleaseError(f"release build root uses {fs_name or 'unknown filesystem'}; use an APFS volume or APFS image on the SSD")
        if mount == DEFAULT_BUILD_MOUNT and not default_image_is_mounted():
            raise ReleaseError("AxiomReleaseBuild is not backed by the expected image on ExternalSSD")
    if not os.access(root, os.W_OK):
        raise ReleaseError("release build root is not writable")
    return root


def build_environment(root: Path, component_id: str) -> dict[str, str]:
    root = root.resolve()
    env = os.environ.copy()
    work = root / "work" / component_id
    folders = (work, root / "artifacts" / component_id, root / "cache" / "cargo",
               root / "cache" / "gradle", root / "cache" / "npm",
               root / "cache" / "pnpm", root / "cache" / "pip",
               root / "cache" / "habitat", root / "cache" / "xdg", root / "tmp")
    for folder in folders:
        if not folder.resolve().is_relative_to(root):
            raise ReleaseError(f"build output path escapes the external build root: {folder}")
    for folder in folders:
        folder.mkdir(parents=True, exist_ok=True)
    env.update({
        "CARGO_HOME": str(root / "cache" / "cargo"),
        "CARGO_TARGET_DIR": str(work / "cargo-target"),
        "GRADLE_USER_HOME": str(root / "cache" / "gradle"),
        "npm_config_cache": str(root / "cache" / "npm"),
        "npm_config_store_dir": str(root / "cache" / "pnpm"),
        "PIP_CACHE_DIR": str(root / "cache" / "pip"),
        "HABITAT_CACHE_ROOT": str(root / "cache" / "habitat"),
        "XDG_CACHE_HOME": str(root / "cache" / "xdg"),
        "TMPDIR": str(root / "tmp"),
        "AXIOM_UI_HOST_BUILD_ROOT": str(work / "ui-host"),
        "AXIOM_UI_HOST_DIST_ROOT": str(root / "artifacts" / "ui-host-dist"),
        # Signing material must not follow the build cache onto a portable SSD.
        "AXIOM_UI_HOST_SIGNING_TEMP_ROOT": "/private/tmp" if platform.system() == "Darwin" else "/tmp",
    })
    return env


def package_cli(binary: Path, output: Path) -> None:
    with output.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                info = tarfile.TarInfo("axiom")
                info.size = binary.stat().st_size
                info.mode = 0o755
                info.mtime = info.uid = info.gid = 0
                with binary.open("rb") as source:
                    archive.addfile(info, source)


def verify_locked_renderer(catalog: dict, workspace: Path) -> None:
    host = repo_path(catalog, "axiom-ui-host", workspace)
    lock = json.loads((host / "toolchain/engine-source.lock.json").read_text())
    expected = lock["engine"]["commit"]
    engine = Path(os.environ.get("AXIOM_UI_HOST_ENGINE_SOURCE") or workspace / "research/lynx").resolve()
    if not (engine / ".git").exists():
        raise ReleaseError(f"pinned renderer checkout is missing: {engine}")
    actual = run("git", "rev-parse", "HEAD", cwd=engine).decode().strip()
    if actual != expected:
        raise ReleaseError(f"renderer checkout {actual} does not match pinned commit {expected}")
    if run("git", "status", "--porcelain=v1", "-z", "--untracked-files=all", cwd=engine):
        raise ReleaseError("renderer checkout has uncommitted source or generated files")


def build_component(plan_path: Path, component_id: str, catalog: dict, workspace: Path = WORKSPACE) -> dict:
    planned = json.loads(plan_path.read_text())
    if planned.get("format") != PLAN_FORMAT or planned.get("catalogSha256") != catalog["sha256"]:
        raise ReleaseError("build plan is missing, stale, or uses a different release catalog")
    entry = next((item for item in planned["components"] if item["id"] == component_id), None)
    if not entry or not entry["selected"]:
        raise ReleaseError(f"{component_id} is not selected for a build")
    if entry["adapter"] == "ci-only":
        raise ReleaseError(f"{component_id} has no SSD-safe local build adapter; use its owning CI job")
    require_current_plan(planned, catalog, workspace)
    if entry["adapter"].startswith("host-"):
        verify_locked_renderer(catalog, workspace)
    root = require_external_build_root()
    artifact_dir = root / "artifacts" / component_id
    if artifact_dir.exists() and any(artifact_dir.iterdir()):
        raise ReleaseError(f"refusing to overwrite existing build artifacts: {artifact_dir}")
    env = build_environment(root, component_id)
    adapter = entry["adapter"]
    if adapter == "cli":
        run("cargo", "build", "--locked", "--release", "--manifest-path", str(workspace / "AxiomCore/cli/Cargo.toml"),
            cwd=workspace, env=env, capture=False)
        binary = Path(env["CARGO_TARGET_DIR"]) / "release" / "axiom-cli"
        if not binary.is_file():
            raise ReleaseError("CLI build completed without an axiom-cli binary")
        suffix = "macos" if platform.system() == "Darwin" else "linux"
        arch = "arm64" if platform.machine() in ("arm64", "aarch64") else "amd64"
        artifact = artifact_dir / f"axiom-{suffix}-{arch}.tar.gz"
        package_cli(binary, artifact)
    else:
        scripts = {"host-web": ("build-web.sh", "axiom-ui-host-web-browser.zip"),
                   "host-android": ("build-android.sh", "axiom-ui-host-android-emulator.apk"),
                   "host-ios": ("build-ios.sh", "axiom-ui-host-ios-simulator.app.zip")}
        script, filename = scripts[adapter]
        argument = {"host-web": None, "host-android": "release", "host-ios": "simulator"}[adapter]
        command = [str(workspace / "axiom-ui-host/scripts" / script)]
        if argument:
            command.append(argument)
        run(*command, cwd=workspace / "axiom-ui-host", env=env, capture=False)
        built = Path(env["AXIOM_UI_HOST_BUILD_ROOT"]) / "output" / filename
        if not built.is_file():
            raise ReleaseError(f"host build completed without {filename}")
        artifact = artifact_dir / filename
        with built.open("rb") as source, artifact.open("xb") as destination:
            shutil.copyfileobj(source, destination)
    digest = sha256_file(artifact)
    receipt = {"format": RECEIPT_FORMAT, "component": component_id,
               "fingerprint": entry["fingerprint"], "sourceHeads": planned["repositories"],
               "artifacts": [{"file": artifact.name, "path": str(artifact), "sha256": digest}]}
    write_json(artifact_dir / "receipt.json", receipt)
    return receipt


def verify_receipt(path: Path, expected: str | None = None) -> dict:
    receipt = json.loads(path.read_text())
    if receipt.get("format") != RECEIPT_FORMAT:
        raise ReleaseError(f"invalid build receipt: {path}")
    if expected and receipt.get("fingerprint") != expected:
        raise ReleaseError(f"input fingerprint mismatch for {receipt.get('component')}")
    for item in receipt.get("artifacts", []):
        artifact = Path(item["path"])
        if not artifact.is_file() or artifact.name != item["file"]:
            raise ReleaseError(f"artifact is absent or renamed: {artifact}")
        if sha256_file(artifact) != item["sha256"]:
            raise ReleaseError(f"artifact checksum mismatch: {artifact}")
    if not receipt.get("artifacts"):
        raise ReleaseError(f"build receipt has no artifacts: {path}")
    return receipt


def require_current_plan(plan: dict, catalog: dict, workspace: Path) -> dict:
    if plan.get("format") != PLAN_FORMAT or plan.get("catalogSha256") != catalog["sha256"]:
        raise ReleaseError("plan is stale or uses a different release catalog")
    ids = {item["id"] for item in plan.get("components", [])}
    if not ids:
        raise ReleaseError("plan contains no components")
    fresh = make_plan(catalog, workspace, only=ids)
    if fresh["blocked"]:
        raise ReleaseError("commit or isolate dirty source inputs: " + ", ".join(fresh["blocked"]))
    if plan.get("blockedVersions"):
        raise ReleaseError("versioned components need new versions before building: " +
                           ", ".join(plan["blockedVersions"]))
    current = {item["id"]: item for item in fresh["components"]}
    for item in plan["components"]:
        if current[item["id"]]["fingerprint"] != item["fingerprint"]:
            raise ReleaseError(f"source inputs changed since planning: {item['id']}")
    if fresh["repositories"] != plan.get("repositories"):
        raise ReleaseError("source commits changed since planning")
    return fresh


def record_artifact(plan: dict, catalog: dict, workspace: Path, component_id: str,
                    artifacts: list[Path], output: Path) -> dict:
    require_current_plan(plan, catalog, workspace)
    entry = next((item for item in plan["components"] if item["id"] == component_id), None)
    if not entry:
        raise ReleaseError(f"component is not in the plan: {component_id}")
    root = require_external_build_root()
    records = []
    for artifact in artifacts:
        real = artifact.resolve()
        if not real.is_file() or root not in real.parents:
            raise ReleaseError(f"artifact must be a file under AXIOM_RELEASE_BUILD_ROOT: {artifact}")
        records.append({"file": real.name, "path": str(real), "sha256": sha256_file(real)})
    if not records:
        raise ReleaseError("at least one artifact is required")
    receipt = {"format": RECEIPT_FORMAT, "component": component_id,
               "fingerprint": entry["fingerprint"], "sourceHeads": plan["repositories"],
               "artifacts": records}
    if root not in output.resolve().parents:
        raise ReleaseError("receipt output must be under AXIOM_RELEASE_BUILD_ROOT")
    write_json(output, receipt)
    return receipt


def stage_manifest(plan: dict, catalog: dict, workspace: Path, train_id: str,
                   receipt_paths: list[Path], output: Path, intent: dict | None = None) -> dict:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{2,79}", train_id):
        raise ReleaseError("train ID must be 3-80 safe letters, digits, dots, underscores, or hyphens")
    if intent is not None and intent.get("trainId") != train_id:
        raise ReleaseError("release intent train ID differs from staged train ID")
    require_current_plan(plan, catalog, workspace)
    root = require_external_build_root()
    if root not in output.resolve().parents:
        raise ReleaseError("staged manifest output must be under AXIOM_RELEASE_BUILD_ROOT")
    receipts = {}
    for path in receipt_paths:
        receipt = verify_receipt(path)
        component_id = receipt["component"]
        if component_id in receipts:
            raise ReleaseError(f"duplicate build receipt for {component_id}")
        receipts[component_id] = receipt
    planned_ids = {item["id"] for item in plan["components"]}
    if planned_ids != receipts.keys():
        raise ReleaseError("every planned component needs exactly one verified artifact receipt")
    components = []
    definitions = {component["id"]: component for component in catalog["components"]}
    intended = {change["component"]: change for change in intent["changes"]} if intent else {}
    host_versions = {change["version"] for component_id, change in intended.items()
                     if component_id.startswith("ui-host-")}
    if len(host_versions) > 1:
        raise ReleaseError("UI Host targets in one staged train need the same release version")
    host_version = next(iter(host_versions), None)
    for entry in plan["components"]:
        receipt = receipts[entry["id"]]
        if receipt["fingerprint"] != entry["fingerprint"]:
            raise ReleaseError(f"receipt does not match the pinned inputs for {entry['id']}")
        if entry["selected"] and receipt.get("sourceHeads") != plan["repositories"]:
            raise ReleaseError(f"new artifact receipt does not match the pinned commits for {entry['id']}")
        if entry["reuseCandidate"]:
            expected = {(item["file"], item["sha256"]) for item in entry["artifacts"]}
            observed = {(item["file"], item["sha256"]) for item in receipt["artifacts"]}
            if expected != observed:
                raise ReleaseError(f"reused artifact differs from the published baseline: {entry['id']}")
        for artifact in receipt["artifacts"]:
            if root not in Path(artifact["path"]).resolve().parents:
                raise ReleaseError(f"artifact is outside the configured build root: {artifact['path']}")
        required = set(definitions[entry["id"]].get("required_artifacts", []))
        observed_names = {artifact["file"] for artifact in receipt["artifacts"]}
        if required and observed_names != required:
            raise ReleaseError(f"{entry['id']} receipt must contain exactly: {', '.join(sorted(required))}")
        components.append({"id": entry["id"], "owner": entry["owner"], "kind": entry["kind"],
                           "version": entry["version"], "fingerprint": entry["fingerprint"],
                           "releaseVersion": host_version if entry["id"].startswith("ui-host-") and host_version
                           else intended.get(entry["id"], {}).get("version", entry.get("releaseVersion") or entry["version"]),
                           "destination": entry["destination"],
                           "source": "built" if entry["selected"] else "reused",
                           "artifacts": [{"file": artifact["file"], "sha256": artifact["sha256"]}
                                         for artifact in receipt["artifacts"]]})
    staged = {"format": "axiom-platform-release-manifest/v1", "status": "staged-not-published",
              "trainId": train_id, "catalogSha256": catalog["sha256"],
              "intentSha256": sha256(canonical(intent)) if intent else None,
              "repositories": plan["repositories"], "components": components}
    write_json(output, staged)
    return staged


def release_notes(plan: dict, catalog: dict, workspace: Path, enforce: bool = False,
                  owner_filter: str | None = None) -> str:
    if plan.get("format") != PLAN_FORMAT or plan.get("catalogSha256") != catalog["sha256"]:
        raise ReleaseError("release notes require a plan from the current catalog")
    if owner_filter and owner_filter not in catalog["repositories"]:
        raise ReleaseError(f"unknown release-note owner: {owner_filter}")
    selected = {item["id"] for item in plan["components"] if item["selected"]
                and (owner_filter is None or item["owner"] == owner_filter)}
    if owner_filter and not selected:
        raise ReleaseError(f"no selected changes for release-note owner: {owner_filter}")
    owners = {item["owner"] for item in plan["components"] if item["id"] in selected}
    catalog_owners = {component["id"]: component["owner"] for component in catalog["components"]}
    fragments: dict[str, list[dict]] = {component_id: [] for component_id in selected}
    for owner in sorted(owners):
        owner_root = repo_path(catalog, owner, workspace)
        folder = owner_root / "release-notes" / "unreleased"
        if not folder.is_dir():
            continue
        for path in sorted(folder.glob("*.json")):
            item = json.loads(path.read_text())
            component_id = item.get("component")
            if component_id not in catalog_owners:
                raise ReleaseError(f"unknown release-note component in {path}: {component_id}")
            if catalog_owners[component_id] != owner:
                raise ReleaseError(f"release-note component {component_id} does not belong to {owner}: {path}")
            if item.get("type") not in {"feature", "fix", "security", "breaking", "internal"}:
                raise ReleaseError(f"invalid release-note type in {path}")
            if not isinstance(item.get("summary"), str) or not item["summary"].strip():
                raise ReleaseError(f"empty release-note summary in {path}")
            if item["type"] == "breaking" and not item.get("migration"):
                raise ReleaseError(f"breaking release note needs migration guidance: {path}")
            if component_id in selected:
                if enforce:
                    relative = str(path.relative_to(owner_root))
                    if run("git", "status", "--porcelain", "--", relative, cwd=owner_root).strip():
                        raise ReleaseError(f"release-note fragment must be committed before release: {path}")
                    if not run("git", "ls-files", "--", relative, cwd=owner_root).strip():
                        raise ReleaseError(f"release-note fragment is not tracked: {path}")
                fragments[component_id].append(item)
    if enforce:
        missing = sorted(component_id for component_id, items in fragments.items() if not items)
        if missing:
            raise ReleaseError("selected components need committed release-note fragments: " + ", ".join(missing))
    title = f"# {owner_filter} release notes" if owner_filter else "# AxiomCore release train"
    lines = [title, "", "Generated from selected component changes; unchanged components retain their prior versions.", ""]
    for entry in plan["components"]:
        if entry["id"] not in selected:
            continue
        lines.extend([f"## {entry['id']} ({entry['owner']})", ""])
        for item in fragments[entry["id"]]:
            lines.append(f"- {item['type']}: {item['summary'].strip()}")
            if item.get("migration"):
                lines.append(f"  Migration: {item['migration'].strip()}")
        if not fragments[entry["id"]]:
            lines.append("- No release-note fragment yet.")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=CATALOG)
    parser.add_argument("--workspace", type=Path, default=WORKSPACE)
    commands = parser.add_subparsers(dest="command", required=True)
    plan_parser = commands.add_parser("plan", help="calculate selected and reusable components; never build")
    plan_parser.add_argument("--baseline", type=Path)
    plan_parser.add_argument("--only", action="append", default=[])
    plan_parser.add_argument("--force", action="append", default=[],
                             help="select a component even when its tracked source inputs match")
    plan_parser.add_argument("--out", type=Path)
    plan_parser.add_argument("--json", action="store_true")
    plan_parser.add_argument("--strict", action="store_true", help="fail when a declared input is dirty")
    commands.add_parser("mount", help="safely mount the existing SSD-backed APFS release image")
    commands.add_parser("preflight", help="check the local external APFS build root")
    build_parser = commands.add_parser("build", help="build one selected component, without publishing")
    build_parser.add_argument("--plan", type=Path, required=True)
    build_parser.add_argument("--component", required=True)
    verify_parser = commands.add_parser("verify", help="verify a local build receipt and artifact checksum")
    verify_parser.add_argument("--receipt", type=Path, required=True)
    record_parser = commands.add_parser("receipt", help="record an independently built CI/local artifact")
    record_parser.add_argument("--plan", type=Path, required=True)
    record_parser.add_argument("--component", required=True)
    record_parser.add_argument("--artifact", type=Path, action="append", required=True)
    record_parser.add_argument("--out", type=Path, required=True)
    stage_parser = commands.add_parser("stage", help="assemble a verified, unpublished release candidate")
    stage_parser.add_argument("--plan", type=Path, required=True)
    stage_parser.add_argument("--train-id", required=True)
    stage_parser.add_argument("--receipt", type=Path, action="append", required=True)
    stage_parser.add_argument("--intent", type=Path,
                              help="bind the candidate to reviewed release versions and changelog intent")
    stage_parser.add_argument("--out", type=Path, required=True)
    notes_parser = commands.add_parser("notes", help="assemble component and release-train notes")
    notes_parser.add_argument("--plan", type=Path, required=True)
    notes_parser.add_argument("--out", type=Path)
    notes_parser.add_argument("--enforce", action="store_true")
    notes_parser.add_argument("--owner", help="render only one owning repository's selected changes")
    args = parser.parse_args()
    try:
        if args.command == "mount":
            print(f"Release build root: {mount_default_build_root()}")
            return 0
        if args.command == "preflight":
            print(f"Release build root: {require_external_build_root()}")
            return 0
        catalog = read_catalog(args.catalog)
        if args.command == "plan":
            plan = make_plan(catalog, args.workspace.resolve(), read_baseline(args.baseline),
                             set(args.only) or None, set(args.force))
            if args.out:
                write_json(args.out, plan)
            if args.json:
                print(json.dumps(plan, indent=2, sort_keys=True))
            else:
                for component in plan["components"]:
                    if component["reason"] != "out of requested scope":
                        disposition = "BLOCK" if component["blocked"] else "BUILD" if component["selected"] else "REUSE"
                        print(f"{disposition:5} {component['id']:22} {component['reason']}")
                if plan["blocked"]:
                    print("BLOCKED for build: dirty repositories: " + ", ".join(plan["blocked"]), file=sys.stderr)
                if args.out:
                    print(f"Plan: {args.out}")
            if args.strict and (plan["blocked"] or plan["blockedVersions"]):
                reasons = []
                if plan["blocked"]:
                    reasons.append("dirty sources: " + ", ".join(plan["blocked"]))
                if plan["blockedVersions"]:
                    reasons.append("unchanged published versions: " + ", ".join(plan["blockedVersions"]))
                print("release-control: " + "; ".join(reasons),
                      file=sys.stderr)
                return 2
            return 0
        if args.command == "build":
            receipt = build_component(args.plan, args.component, catalog, args.workspace.resolve())
            print(json.dumps(receipt, indent=2, sort_keys=True))
            return 0
        if args.command == "verify":
            receipt = verify_receipt(args.receipt)
            print(f"Verified {receipt['component']}: {len(receipt['artifacts'])} artifact(s)")
            return 0
        if args.command == "receipt":
            receipt = record_artifact(json.loads(args.plan.read_text()), catalog, args.workspace.resolve(),
                                      args.component, args.artifact, args.out)
            print(f"Recorded {receipt['component']}: {len(receipt['artifacts'])} artifact(s)")
            return 0
        if args.command == "stage":
            intent = None
            if args.intent:
                from flow import read_intent
                from versions import VERSIONS, read_versions
                intent = read_intent(args.intent, catalog, read_versions(VERSIONS, catalog))
            staged = stage_manifest(json.loads(args.plan.read_text()), catalog, args.workspace.resolve(),
                                    args.train_id, args.receipt, args.out, intent)
            print(f"Staged {staged['trainId']}: {len(staged['components'])} component(s); nothing published")
            return 0
        if args.command == "notes":
            markdown = release_notes(json.loads(args.plan.read_text()), catalog,
                                     args.workspace.resolve(), args.enforce, args.owner)
            if args.out:
                write_new_text(args.out, markdown)
                print(f"Release notes: {args.out}")
            else:
                print(markdown, end="")
            return 0
    except (ReleaseError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"release-control: {error}", file=sys.stderr)
        return 2
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
