#!/usr/bin/env python3
"""Selective builders for the formerly CI-only release components.

All local build inputs are copied from clean, pinned Git checkouts to the
release volume. Cloud Build receives only that staged source and creates an
immutable candidate tag; deployment remains the publisher's responsibility.
"""

from __future__ import annotations

import gzip
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time

import ctl
import versions


SUPPORTED = frozenset({
    "runtime-apple", "sdk-atmx-web", "sdk-atmx-react", "sdk-atmx-cli",
    "sdk-flutter-generator", "sdk-flutter", "sdk-swift", "backend-api",
    "extractor-fastapi", "extractor-go", "backend-worker", "mock-runner",
    "contract-test-runner", "dashboard-origin",
})
IMAGES = {
    "backend-api": ("axiom-backend", "axiom-backend", "axiom-backend/Dockerfile"),
    "backend-worker": ("axiom-backend", "axiom-semantic-worker", "axiom-backend/Dockerfile.semantic-worker"),
    "mock-runner": ("axiom-backend", "axiom-mock-runner", "axiom-backend/Dockerfile.mock-runner"),
    "contract-test-runner": ("axiom-backend", "axiom-contract-test-runner", "axiom-backend/Dockerfile.contract-test-runner"),
    "dashboard-origin": ("axiom-frontend", "axiom-dashboard", "axiom-frontend/Dockerfile.dashboard"),
}
TOOLS = {
    "runtime-apple": ("cargo", "rustup", "cbindgen", "xcodebuild", "lipo", "zip"),
    "sdk-atmx-web": ("cargo", "wasm-pack", "npm"),
    "sdk-atmx-react": ("npm",),
    "sdk-atmx-cli": ("npm",),
    "sdk-flutter-generator": ("dart",),
    "sdk-flutter": ("cargo", "wasm-pack", "flutter"),
    "sdk-swift": ("swift",),
    "extractor-fastapi": ("python3",),
    "extractor-go": ("go",),
    "dashboard-origin": ("gcloud", "cargo", "wasm-pack", "npm", "pnpm", "just", "go"),
    "backend-api": ("gcloud",),
    "backend-worker": ("gcloud",),
    "mock-runner": ("gcloud",),
    "contract-test-runner": ("gcloud",),
}


def prerequisite_issues(selected: set[str], environment: dict[str, str] | None = None) -> list[str]:
    environment = os.environ if environment is None else environment
    issues = []
    for component in sorted(selected & SUPPORTED):
        missing = [name for name in TOOLS[component] if not shutil.which(name, path=environment.get("PATH"))]
        if missing:
            issues.append(f"{component}: install required build tools: {', '.join(missing)}")
        if component == "runtime-apple" and platform.system() != "Darwin":
            issues.append("runtime-apple: run this builder on a macOS machine with Xcode")
        if component in IMAGES and not environment.get("AXIOM_GCP_REGION"):
            issues.append(f"{component}: set AXIOM_GCP_REGION for Cloud Build and publication")
    return issues


def dependency_blockers(selected: set[str], ledger: dict, workspace: Path) -> list[str]:
    """Catch source pins that a multi-component build cannot repair safely."""
    if not {"sdk-swift", "sdk-flutter", "sdk-atmx-react"} & selected:
        return []
    blockers = []
    if {"sdk-swift", "sdk-flutter"} & selected:
        expected = ledger["components"]["runtime-apple"]["candidateVersion"]
        if not expected:
            return ["Apple SDK packages need a candidate runtime-apple version"]
    if "sdk-swift" in selected:
        package = (workspace / "axiom-sdk/swift/Package.swift").read_text()
        pinned = re.search(r"releases/download/v([^/]+)/AxiomRuntime\.xcframework\.zip", package)
        checksum = re.search(r'AxiomRuntime\.xcframework\.zip",\s*checksum:\s*"([a-f0-9]{64})"', package)
        if not pinned or pinned.group(1) != expected or not checksum:
            blockers.append("sdk-swift: Package.swift must pin the selected Apple runtime version and its exact published XCFramework checksum; publish runtime first, then update Swift in a successor release")
    if "sdk-flutter" in selected:
        for platform_name in ("ios", "macos"):
            podspec = workspace / f"axiom-sdk/flutter/axiom_flutter/{platform_name}/axiom_flutter.podspec"
            pinned = re.search(r"(?m)^\s*runtime_version\s*=\s*'([^']+)'", podspec.read_text())
            if not pinned or pinned.group(1) != expected:
                blockers.append(f"sdk-flutter: {platform_name} podspec must pin selected runtime-apple {expected}")
    if "sdk-atmx-react" in selected:
        expected_web = ledger["components"]["sdk-atmx-web"]["candidateVersion"]
        package = workspace / "axiom-sdk/web/atmx-react"
        manifest = json.loads((package / "package.json").read_text())
        lock = json.loads((package / "package-lock.json").read_text())
        required = f"^{expected_web}"
        root_pin = lock.get("packages", {}).get("", {}).get("dependencies", {}).get("atmx-web")
        resolved = lock.get("packages", {}).get("node_modules/atmx-web", {}).get("version")
        if (not expected_web or manifest.get("dependencies", {}).get("atmx-web") != required
                or root_pin != required or resolved != expected_web):
            blockers.append("sdk-atmx-react: package.json and package-lock.json must both pin the selected atmx-web version; publish web first, then update React's lockfile in a successor release")
    return blockers


def _run(*args: str, cwd: Path, env: dict[str, str], capture: bool = False) -> bytes:
    return ctl.run(*args, cwd=cwd, env=env, capture=capture)


def _archive_head(repository: Path, destination: Path, selector: str | None = None) -> None:
    """Stream committed Git bytes; unrelated worktree edits cannot enter a build."""
    command = ["git", "archive", "--format=tar", "HEAD"]
    if selector:
        command.extend(["--", selector])
    process = subprocess.Popen(command, cwd=repository, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE)
    try:
        assert process.stdout is not None
        with tarfile.open(fileobj=process.stdout, mode="r|") as archive:
            for member in archive:
                relative = Path(member.name)
                if relative.is_absolute() or ".." in relative.parts or not relative.parts:
                    raise ctl.ReleaseError(f"unsafe tracked release source: {member.name}")
                target = destination / relative
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                if not member.isfile() or member.issym() or member.islnk():
                    raise ctl.ReleaseError(f"linked or unsupported tracked source: {member.name}")
                target.parent.mkdir(parents=True, exist_ok=True)
                content = archive.extractfile(member)
                if content is None:
                    raise ctl.ReleaseError(f"unreadable tracked source: {member.name}")
                with target.open("xb") as output:
                    shutil.copyfileobj(content, output)
                target.chmod(0o755 if member.mode & 0o111 else 0o644)
        error = process.stderr.read().decode(errors="replace") if process.stderr else ""
        if process.wait():
            raise ctl.ReleaseError(f"git archive failed for {repository}: {error.strip()}")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        if process.stdout:
            process.stdout.close()
        if process.stderr:
            process.stderr.close()


def _stage(entry: dict, catalog: dict, workspace: Path, work: Path) -> Path:
    """Copy tracked bytes only. require_current_plan checked their Git heads."""
    source = Path(tempfile.mkdtemp(prefix="source-", dir=work))
    names = {item["repo"] for item in entry["sources"]} | {entry["owner"]}
    if entry["id"] in ("sdk-atmx-web", "sdk-flutter", "dashboard-origin"):
        names |= {"AxiomCore", "axiom-runtime", "axiom-lib", "rod", "axiom-sdk"}
    if entry["id"] == "dashboard-origin":
        names |= {"axiom-backend", "axiom-frontend", "atmx-web"}
        cli = next(item for item in catalog["components"] if item["id"] == "cli")
        names.update(item["repo"] for item in cli["sources"])
    if entry["id"] == "sdk-atmx-react":
        names.add("atmx-web")
    for name in sorted(names):
        repository = ctl.repo_path(catalog, name, workspace)
        if name == "acore-diff":
            _archive_head(repository, source, "acore-diff")
        else:
            destination = source / catalog["repositories"][name]
            destination.mkdir(parents=True, exist_ok=True)
            _archive_head(repository, destination)
    return source


def _copy(source: Path, destination: Path) -> Path:
    if not source.is_file() or source.is_symlink():
        raise ctl.ReleaseError(f"selective build did not produce {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with source.open("rb") as original, destination.open("xb") as output:
        shutil.copyfileobj(original, output)
    return destination


def _archive_package(source: Path, destination: Path) -> Path:
    if not (source / "pubspec.yaml").is_file():
        raise ctl.ReleaseError(f"pub package is missing pubspec.yaml: {source}")
    excluded = {".dart_tool", "build", ".git", ".idea", "node_modules"}
    files = sorted(path for path in source.rglob("*") if path.is_file()
                   and not any(part in excluded for part in path.relative_to(source).parts)
                   and path.name != "pubspec.lock")
    if any(path.is_symlink() for path in source.rglob("*")):
        raise ctl.ReleaseError("pub package contains a symlink; refusing to archive")
    with destination.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                for path in files:
                    info = tarfile.TarInfo(path.relative_to(source).as_posix())
                    info.size = path.stat().st_size
                    info.mode = 0o644
                    info.mtime = info.uid = info.gid = 0
                    with path.open("rb") as content:
                        archive.addfile(info, content)
    return destination


def _npm_pack(package: Path, artifact_dir: Path, env: dict[str, str]) -> Path:
    result = json.loads(_run("npm", "pack", "--json", "--ignore-scripts",
                             "--pack-destination", str(artifact_dir), cwd=package,
                             env=env, capture=True))
    if not isinstance(result, list) or len(result) != 1 or not isinstance(result[0].get("filename"), str):
        raise ctl.ReleaseError("npm pack did not return exactly one package archive")
    archive = artifact_dir / result[0]["filename"]
    if not archive.is_file() or archive.is_symlink():
        raise ctl.ReleaseError("npm pack archive is missing")
    return archive


def _build_wasm(source: Path, env: dict[str, str]) -> None:
    # The existing script rewrites only the staged SDK copies on the SSD.
    _run("bash", str(source / "AxiomCore/release/scripts/wasm.sh"),
         cwd=source / "AxiomCore", env=env)


def _local(entry: dict, catalog: dict, workspace: Path, root: Path,
           artifact_dir: Path, env: dict[str, str], plan: dict) -> list[Path]:
    component = entry["id"]
    source = _stage(entry, catalog, workspace, root / "work" / component)
    if component == "runtime-apple":
        if platform.system() != "Darwin":
            raise ctl.ReleaseError("runtime-apple needs a macOS builder with Xcode")
        env["CARGO_TARGET_DIR"] = str(source / "axiom-runtime/target")
        _run("bash", str(source / "AxiomCore/release/scripts/build_apple.sh"),
             cwd=source / "AxiomCore", env=env)
        return [_copy(source / "axiom-runtime/dist/AxiomRuntime.xcframework.zip",
                      artifact_dir / "AxiomRuntime.xcframework.zip")]
    if component in ("sdk-atmx-web", "sdk-flutter"):
        _build_wasm(source, env)
    if component.startswith("sdk-atmx-"):
        package = (source / "AxiomCore/release/atmx-cli" if component == "sdk-atmx-cli"
                   else source / "axiom-sdk/web" / ("atmx" if component == "sdk-atmx-web" else "atmx-react"))
        if component == "sdk-atmx-react":
            web = next(item for item in plan["components"] if item["id"] == "sdk-atmx-web")
            receipt = ctl.artifact_receipt_path(root, web, plan["repositories"])
            verified = ctl.verify_receipt(receipt, web["fingerprint"])
            tarballs = [Path(item["path"]) for item in verified["artifacts"] if item["file"].endswith(".tgz")]
            if len(tarballs) != 1:
                raise ctl.ReleaseError("React build needs exactly one staged atmx-web tarball")
            manifest = package / "package.json"
            lock = package / "package-lock.json"
            original_manifest, original_lock = manifest.read_bytes(), lock.read_bytes()
            modified = json.loads(original_manifest)
            modified["dependencies"]["atmx-web"] = f"file:{tarballs[0]}"
            manifest.write_text(json.dumps(modified, indent=2) + "\n")
            _run("npm", "install", "--package-lock-only", "--ignore-scripts", cwd=package, env=env)
            _run("npm", "ci", "--ignore-scripts", cwd=package, env=env)
            manifest.write_bytes(original_manifest)
            lock.write_bytes(original_lock)
        else:
            _run("npm", "ci", "--ignore-scripts", cwd=package, env=env)
        _run("npm", "run", "build", cwd=package, env=env)
        artifacts = [_npm_pack(package, artifact_dir, env)]
        if component == "sdk-atmx-web":
            artifacts.extend(_copy(package / "dist" / name, artifact_dir / name)
                             for name in ("atmx.umd.js", "atmx.es.js", "axiom_runtime.wasm"))
        return artifacts
    if component in ("sdk-flutter", "sdk-flutter-generator"):
        name = "axiom_flutter" if component == "sdk-flutter" else "axiom_flutter_generator"
        package = source / "axiom-sdk/flutter" / name
        tool = "flutter" if component == "sdk-flutter" else "dart"
        _run(tool, "pub", "get", cwd=package, env=env)
        _run(tool, "pub", "publish", "--dry-run", cwd=package, env=env)
        manifest = package / "pubspec.yaml"
        match = re.search(r"(?m)^version:\s*([0-9]+\.[0-9]+\.[0-9]+)\s*$", manifest.read_text())
        if not match:
            raise ctl.ReleaseError(f"{name} has no stable version")
        return [_archive_package(package, artifact_dir / f"{name}-{match.group(1)}.tar.gz")]
    if component == "sdk-swift":
        package = source / "axiom-sdk/swift"
        _run("swift", "package", "dump-package", cwd=package, env=env, capture=True)
        return [_copy(package / "Package.swift", artifact_dir / "Package.swift")]
    if component == "extractor-go":
        package = source / "axiom-extractor/extractors/go"
        suffix = "macos" if platform.system() == "Darwin" else "linux"
        arch = "arm64" if platform.machine() in ("arm64", "aarch64") else "amd64"
        destination = artifact_dir / f"axiom-go-extractor-{suffix}-{arch}"
        _run("go", "test", "./...", cwd=package, env=env)
        _run("go", "build", "-trimpath", "-o", str(destination),
             "./cmd/axiom-go-extractor", cwd=package, env=env)
        return [destination]
    if component == "extractor-fastapi":
        package = source / "axiom-extractor/extractors/python/frameworks/axiom_fastapi"
        venv = source / ".venv"
        _run(sys.executable, "-m", "venv", str(venv), cwd=package, env=env)
        pip = str(venv / "bin/pip")
        _run(pip, "install", "--no-input", "poetry", "pyinstaller", cwd=package, env=env)
        _run(str(venv / "bin/poetry"), "install", "--no-interaction",
             cwd=package, env={**env, "POETRY_VIRTUALENVS_CREATE": "false", "VIRTUAL_ENV": str(venv),
                               "PATH": str(venv / "bin") + os.pathsep + env["PATH"]})
        _run(str(venv / "bin/pyinstaller"), "--onefile", "--name", "axiom-fastapi",
             "--paths", "src", "--collect-all", "fastapi", "--collect-all", "pydantic",
             "--collect-all", "pydantic_core", "--collect-all", "starlette",
             "--distpath", str(source / "dist"), "build_entrypoint.py", cwd=package, env=env)
        suffix = "macos" if platform.system() == "Darwin" else "linux"
        arch = "arm64" if platform.machine() in ("arm64", "aarch64") else "amd64"
        return [_copy(source / "dist/axiom-fastapi", artifact_dir / f"axiom-fastapi-{suffix}-{arch}")]
    raise ctl.ReleaseError(f"no selective local builder for {component}")


def _image(entry: dict, catalog: dict, workspace: Path, root: Path,
           artifact_dir: Path, env: dict[str, str], plan: dict) -> list[Path]:
    component = entry["id"]
    region = os.environ.get("AXIOM_GCP_REGION", "")
    project = os.environ.get("AXIOM_GCP_PROJECT_ID", "axiomcore")
    if not re.fullmatch(r"[a-z][a-z0-9-]+", region) or not re.fullmatch(r"[a-z][a-z0-9-]+", project):
        raise ctl.ReleaseError("set a valid AXIOM_GCP_REGION and AXIOM_GCP_PROJECT_ID before building images")
    repository, image_name, dockerfile = IMAGES[component]
    source_digest = ctl.sha256(ctl.canonical(plan["repositories"]))
    tag = f"candidate-{entry['fingerprint'][:12]}-{source_digest[:12]}"
    image = f"{region}-docker.pkg.dev/{project}/{repository}/{image_name}:{tag}"
    checkpoint = root / "work" / component / f"cloud-build-{entry['fingerprint']}-{source_digest}.json"
    if checkpoint.is_file():
        saved = json.loads(checkpoint.read_text())
        if saved.get("image") != image or saved.get("sourceHeads") != plan["repositories"]:
            raise ctl.ReleaseError("Cloud Build checkpoint differs from the pinned candidate")
        build_id = saved["buildId"]
    else:
        source = _stage(entry, catalog, workspace, root / "work" / component)
        if component == "dashboard-origin":
            _build_dashboard(source, env)
            context = source / "axiom-frontend"
            dockerfile = "Dockerfile.dashboard"
        elif component == "backend-api":
            context = source / "axiom-backend"
            dockerfile = "Dockerfile"
        else:
            context = source
        config = context / "axiom-release-cloudbuild.json"
        ctl.write_json(config, {"steps": [{"name": "gcr.io/cloud-builders/docker",
                                            "args": ["build", "--file", dockerfile, "--tag", "${_IMAGE_URI}",
                                                     "--label", "axiom.source-heads=${_AXIOM_SOURCE_HEADS_SHA256}", "."]}],
                                "images": ["${_IMAGE_URI}"],
                                "options": {"logging": "CLOUD_LOGGING_ONLY"}, "timeout": "3600s"})
        # A generated ignore file ensures only the reviewed staged tree is uploaded.
        ignore = context / "axiom-release-cloudbuild.ignore"
        ignore.write_text(".git\nnode_modules\n.dart_tool\ntarget\n")
        response = json.loads(_run("gcloud", "builds", "submit", str(context),
                                   "--config", str(config), "--ignore-file", str(ignore),
                                   "--async", "--format=json", "--quiet", f"--project={project}",
                                   "--substitutions", f"_IMAGE_URI={image},_AXIOM_SOURCE_HEADS_SHA256={source_digest}",
                                   cwd=context, env=env, capture=True))
        build_id = response.get("id")
        if not isinstance(build_id, str) or not re.fullmatch(r"[a-fA-F0-9-]{36}", build_id):
            raise ctl.ReleaseError("Cloud Build did not return a build ID; inspect the remote build before retrying")
        ctl.write_json(checkpoint, {"image": image, "sourceHeads": plan["repositories"], "buildId": build_id})
    deadline = time.monotonic() + 90 * 60
    while True:
        result = json.loads(_run("gcloud", "builds", "describe", build_id, "--format=json",
                                 "--quiet", f"--project={project}", cwd=workspace, env=env, capture=True))
        status = result.get("status")
        if status == "SUCCESS":
            break
        if status in ("FAILURE", "CANCELLED", "TIMEOUT", "EXPIRED"):
            raise ctl.ReleaseError(f"Cloud Build {build_id} ended {status}; inspect before retrying")
        if time.monotonic() >= deadline:
            raise ctl.ReleaseError(f"Cloud Build {build_id} is still running; resume this candidate after inspecting it")
        print(f"Cloud Build {build_id}: {status or 'pending'}", flush=True)
        time.sleep(15)
    if result.get("substitutions", {}).get("_AXIOM_SOURCE_HEADS_SHA256") != source_digest:
        raise ctl.ReleaseError("Cloud Build did not retain the source-head binding")
    images = result.get("results", {}).get("images", [])
    matches = [item.get("digest") for item in images if item.get("name") == image]
    if len(matches) != 1 or not re.fullmatch(r"sha256:[a-f0-9]{64}", matches[0] or ""):
        raise ctl.ReleaseError("Cloud Build did not report the exact candidate image digest")
    descriptor = artifact_dir / "image-ref.json"
    ctl.write_json(descriptor, {"image": f"{image.split(':', 1)[0]}@{matches[0]}",
                                "sourceHeads": plan["repositories"], "buildId": build_id})
    return [descriptor]


def _build_dashboard(source: Path, env: dict[str, str]) -> None:
    frontend = source / "axiom-frontend"
    cli = source / "AxiomCore/release/atmx-cli"
    web = source / "axiom-sdk/web/atmx"
    _build_wasm(source, env)
    _run("npm", "ci", "--ignore-scripts", cwd=cli, env=env)
    _run("npm", "run", "build", cwd=cli, env=env)
    _run("npm", "ci", "--ignore-scripts", cwd=web, env=env)
    _run("pnpm", "install", "--frozen-lockfile", cwd=frontend, env=env)
    _run("just", "build-dashboard", cwd=frontend, env=env)


def build_component(plan_path: Path, component_id: str, catalog: dict,
                    workspace: Path = ctl.WORKSPACE) -> dict:
    if component_id not in SUPPORTED:
        raise ctl.ReleaseError(f"no selective builder for {component_id}")
    plan = json.loads(plan_path.read_text())
    if plan.get("format") != ctl.PLAN_FORMAT or plan.get("catalogSha256") != catalog["sha256"]:
        raise ctl.ReleaseError("selective builder requires a current, catalog-bound plan")
    entry = next((item for item in plan["components"] if item["id"] == component_id and item["selected"]), None)
    if entry is None:
        raise ctl.ReleaseError(f"{component_id} is not selected in the release plan")
    problems = prerequisite_issues({component_id})
    if problems:
        raise ctl.ReleaseError("; ".join(problems))
    if component_id in ("sdk-swift", "sdk-flutter"):
        ledger = versions.read_versions(versions.VERSIONS, catalog)
        blocked = dependency_blockers({component_id}, ledger, workspace)
        if blocked:
            raise ctl.ReleaseError("; ".join(blocked))
    ctl.require_current_plan(plan, catalog, workspace)
    root = ctl.require_external_build_root()
    artifact_dir = ctl.artifact_receipt_path(root, entry, plan["repositories"]).parent
    if not artifact_dir.resolve().is_relative_to(root) or artifact_dir.is_symlink():
        raise ctl.ReleaseError("artifact path escapes the release volume")
    if artifact_dir.exists() and any(artifact_dir.iterdir()):
        raise ctl.ReleaseError(f"refusing to overwrite an incomplete artifact directory: {artifact_dir}")
    env = ctl.build_environment(root, component_id)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    if component_id in IMAGES:
        artifacts = _image(entry, catalog, workspace, root, artifact_dir, env, plan)
    else:
        artifacts = _local(entry, catalog, workspace, root, artifact_dir, env, plan)
    return ctl.record_artifact(plan, catalog, workspace, component_id, artifacts,
                               artifact_dir / "receipt.json")
