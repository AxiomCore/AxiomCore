#!/usr/bin/env python3
"""Exact-receipt publication adapters for package and infrastructure targets.

These adapters never build, modify an owning source checkout, or invent an
artifact. A missing receipt, remote digest, or deployment proof is an error.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request

import ctl
import npm_oidc
import publisher


GITHUB = {
    "cli": ("AxiomCore", "AxiomCore/AxiomCore", "cli"),
    "runtime-apple": ("AxiomCore", "AxiomCore/AxiomCore", "runtime"),
    "extractor-fastapi": ("AxiomCore", "AxiomCore/AxiomCore", "extractor-fastapi"),
    "extractor-go": ("AxiomCore", "AxiomCore/AxiomCore", "extractor-go"),
}
NPM = {"sdk-atmx-web": "atmx-web", "sdk-atmx-react": "atmx-react",
       "sdk-atmx-cli": "atmx-cli"}
PUB = {"sdk-flutter-generator": "axiom_flutter_generator",
       "sdk-flutter": "axiom_flutter"}
GCP = {
    "backend-api": ("service", "axiom-backend", "axiom-backend", "axiom-backend"),
    "backend-worker": ("job", "axiom-backend", "axiom-backend", "axiom-semantic-worker"),
    "mock-runner": ("tag", "axiom-backend", "axiom-backend", "axiom-mock-runner"),
    "contract-test-runner": ("job", "axiom-backend", "axiom-backend", "axiom-contract-test-runner"),
    "dashboard-origin": ("service", "axiom-frontend", "axiom-frontend", "axiom-dashboard"),
}


def _render_homebrew_formula(before: str, version: str, archive_sha: str) -> str:
    expected_url = (f"https://github.com/AxiomCore/AxiomCore/releases/download/v{version}/"
                    "axiom-macos-arm64.tar.gz")
    modified = before
    patterns = ((r'(?m)^  url "[^"]+"$', f'  url "{expected_url}"'),
                (r'(?m)^  sha256 "[a-fA-F0-9]{64}"$', f'  sha256 "{archive_sha}"'),
                (r'(?m)^  version "[^"]+"$', f'  version "{version}"'))
    for pattern, replacement in patterns:
        modified, count = re.subn(pattern, replacement, modified)
        if count != 1:
            raise ctl.ReleaseError("Homebrew formula has an unrecognized release field")
    return modified


def _homebrew_formula(candidate: dict, version: str, archive_sha: str) -> dict:
    tap = candidate["workspace"] / "AxiomCore/release/homebrew-tap"
    formula = tap / "Formula/axiom.rb"
    if not formula.is_file() or formula.is_symlink():
        raise ctl.ReleaseError("CLI Homebrew tap formula is unavailable")
    origin = ctl.run("git", "remote", "get-url", "origin", cwd=tap).decode().strip()
    if origin not in ("https://github.com/AxiomCore/homebrew-tap.git",
                      "git@github.com:AxiomCore/homebrew-tap.git"):
        raise ctl.ReleaseError("Homebrew tap origin is not AxiomCore/homebrew-tap")
    if ctl.run("git", "status", "--porcelain=v1", cwd=tap).strip():
        raise ctl.ReleaseError("Homebrew tap has uncommitted changes; review before CLI publication")
    remote = ctl.run("git", "ls-remote", "origin", "refs/heads/main", cwd=tap).decode().split()
    if len(remote) != 2:
        raise ctl.ReleaseError("Homebrew tap main branch could not be authenticated")
    remote_head = remote[0]
    head = ctl.run("git", "rev-parse", "HEAD", cwd=tap).decode().strip()
    before = formula.read_text()
    modified = _render_homebrew_formula(before, version, archive_sha)
    if modified != before:
        if head != remote_head:
            raise ctl.ReleaseError("Homebrew tap has local commits not on origin/main; review before changing formula")
        formula.write_text(modified)
        ctl.run("git", "add", "--", "Formula/axiom.rb", cwd=tap)
        ctl.run("git", "commit", "--only", "-m", f"Release axiom CLI v{version}",
                "--", "Formula/axiom.rb", cwd=tap)
        head = ctl.run("git", "rev-parse", "HEAD", cwd=tap).decode().strip()
    if head != remote_head:
        parent = ctl.run("git", "rev-parse", "HEAD^", cwd=tap).decode().strip()
        subject = ctl.run("git", "log", "-1", "--format=%s", cwd=tap).decode().strip()
        if parent != remote_head or subject != f"Release axiom CLI v{version}":
            raise ctl.ReleaseError("Homebrew tap contains an unrelated unpushed commit")
        ctl.run("git", "push", "origin", "HEAD:refs/heads/main", cwd=tap, capture=False)
    response = json.loads(ctl.run("gh", "api",
                                  "repos/AxiomCore/homebrew-tap/contents/Formula/axiom.rb?ref=main",
                                  cwd=tap).decode())
    content = response.get("content")
    if not isinstance(content, str) or base64.b64decode(content).decode() != modified:
        raise ctl.ReleaseError("remote Homebrew formula differs from verified CLI archive")
    return {"repository": "AxiomCore/homebrew-tap", "commit": head,
            "formulaSha256": ctl.sha256(modified.encode()), "archiveSha256": archive_sha}


def _own_stage(candidate: dict, component: str) -> dict:
    if candidate["directory"].name != component:
        raise ctl.ReleaseError(f"publisher needs a staged {component} candidate")
    parts = [item for item in candidate["stage"]["components"] if item["id"] == component]
    if len(parts) != 1:
        raise ctl.ReleaseError(f"staged manifest lacks exactly one {component} component")
    return parts[0]


def _artifacts(candidate: dict, component: str) -> dict[str, dict]:
    _own_stage(candidate, component)
    receipts = [ctl.verify_receipt(path) for path in candidate["receipts"]]
    matching = [item for item in receipts if item["component"] == component]
    if len(matching) != 1:
        raise ctl.ReleaseError(f"missing exact {component} receipt")
    return {item["file"]: item for item in matching[0]["artifacts"]}


def _publication(candidate: dict, component: str) -> tuple[Path, Path, str]:
    directory = candidate["directory"] / "publication"
    return directory, directory / "published.json", ctl.sha256(ctl.canonical(candidate["stage"]))


def _save(candidate: dict, component: str, destination: str, remote: str,
          details: dict) -> dict:
    directory, published, stage_sha = _publication(candidate, component)
    result = {"format": "axiom-platform-component-publication/v1", "component": component,
              "trainId": candidate["intent"]["trainId"], "stageSha256": stage_sha,
              "destination": destination, "remote": remote, "details": details,
              "status": "remote-verified"}
    if published.exists():
        if json.loads(published.read_text()) != result:
            raise ctl.ReleaseError(f"{component} published evidence differs from the verified remote")
        return result
    directory.mkdir(parents=True, exist_ok=True)
    ctl.write_json(published, result)
    return result


def _version(candidate: dict, component: str) -> str:
    value = _own_stage(candidate, component).get("releaseVersion")
    if not isinstance(value, str) or not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?", value):
        raise ctl.ReleaseError(f"{component} needs an explicit staged SemVer release version")
    return value


def _single(candidate: dict, component: str, name: str) -> Path:
    artifacts = _artifacts(candidate, component)
    if set(artifacts) != {name}:
        raise ctl.ReleaseError(f"{component} needs exactly {name} in its receipt")
    return Path(artifacts[name]["path"])


def publish_github(candidate: dict, component: str) -> dict:
    if component not in GITHUB:
        raise ctl.ReleaseError(f"no GitHub publisher for {component}")
    source_owner, repository, tag_prefix = GITHUB[component]
    publisher.remote_source_heads(candidate)
    owner = ctl.repo_path(candidate["catalog"], source_owner, candidate["workspace"])
    publisher._assert_github_origin(repository, owner)
    version = _version(candidate, component) if component != "extractor-go" else None
    tag = f"v{version}" if component in ("cli", "runtime-apple") else (
        f"{tag_prefix}-v{version}" if version else f"{tag_prefix}-{candidate['intent']['trainId']}")
    head = candidate["plan"]["repositories"][source_owner]["head"]
    old_tag = publisher._remote_tag_target(tag, owner)
    if old_tag is not None and old_tag != head:
        raise ctl.ReleaseError(f"{repository} tag {tag} points to a different source commit")
    records = _artifacts(candidate, component)
    names = set(records)
    if component == "runtime-apple" and names != {"AxiomRuntime.xcframework.zip"}:
        raise ctl.ReleaseError("Apple runtime needs one exact AxiomRuntime.xcframework.zip")
    if component == "cli" and (not names or any(not re.fullmatch(
            r"axiom-(macos|linux|windows)-(arm64|amd64)\.tar\.gz", name) for name in names)):
        raise ctl.ReleaseError("CLI receipt has missing or unexpected platform archives")
    if component == "cli" and "axiom-macos-arm64.tar.gz" not in names:
        raise ctl.ReleaseError("CLI publication needs axiom-macos-arm64.tar.gz for the Homebrew tap")
    if component.startswith("extractor-") and (not names or any(not name.startswith(
            "axiom-fastapi-" if component == "extractor-fastapi" else "axiom-go-extractor-") for name in names)):
        raise ctl.ReleaseError("extractor receipt has missing or unexpected platform binaries")
    directory, published, stage_sha = _publication(candidate, component)
    bundle = directory / "bundle"
    marker = directory / "bundle-ready.json"
    expected = {name: item["sha256"] for name, item in records.items()}
    if marker.exists():
        if json.loads(marker.read_text()) != {"stageSha256": stage_sha, "files": expected}:
            raise ctl.ReleaseError("prior GitHub bundle belongs to different candidate bytes")
        if publisher._bundle_checksums(bundle) != expected:
            raise ctl.ReleaseError("prior GitHub bundle bytes changed")
    else:
        if directory.exists():
            raise ctl.ReleaseError(f"incomplete publication bundle; inspect before retrying: {directory}")
        bundle.mkdir(parents=True)
        for name, item in records.items():
            with Path(item["path"]).open("rb") as source, (bundle / name).open("xb") as target:
                shutil.copyfileobj(source, target)
        if publisher._bundle_checksums(bundle) != expected:
            raise ctl.ReleaseError("artifact changed during GitHub bundle staging")
        ctl.write_json(marker, {"stageSha256": stage_sha, "files": expected})
    if not publisher._github_release_exists(repository, tag, owner):
        if published.exists():
            raise ctl.ReleaseError("published evidence exists but GitHub release is missing")
        ctl.run("gh", "release", "create", tag, *[str(bundle / name) for name in sorted(expected)],
                "--repo", repository, "--target", head,
                "--title", f"{component} {version or candidate['intent']['trainId']}",
                "--notes-file", str(candidate["directory"] / "notes.md"), cwd=owner, capture=False)
    publisher._verify_remote_tag(repository, tag, head, owner)
    remote = publisher._verify_github_assets(repository, tag, expected, directory, owner)
    tap = _homebrew_formula(candidate, version, expected["axiom-macos-arm64.tar.gz"]) if component == "cli" else None
    return _save(candidate, component, repository, remote["url"],
                 {"tag": tag, "files": expected, "releaseId": remote["id"], "homebrew": tap})


def _npm_metadata(package: str, version: str, cwd: Path, env: dict[str, str]) -> dict | None:
    completed = subprocess.run(["npm", "view", f"{package}@{version}", "--json", "--registry=https://registry.npmjs.org"],
                               cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if completed.returncode:
        if "E404" in completed.stderr.decode(errors="replace"):
            return None
        raise ctl.ReleaseError(f"npm registry lookup failed for {package}@{version}: "
                               + completed.stderr.decode(errors="replace").strip())
    data = json.loads(completed.stdout)
    if data.get("name") != package or data.get("version") != version:
        raise ctl.ReleaseError("npm registry returned another package or version")
    return data


def _tar_member(archive: Path, name: str) -> bytes:
    with tarfile.open(archive, "r:gz") as source:
        member = source.getmember(name)
        if not member.isfile():
            raise ctl.ReleaseError(f"package archive member is not a file: {name}")
        stream = source.extractfile(member)
        if stream is None:
            raise ctl.ReleaseError(f"package archive member is unreadable: {name}")
        return stream.read()


def _verified_apple_runtime(candidate: dict, version: str, expected_sha: str | None = None) -> str:
    release_tag = f"v{version}"
    records = list((candidate["root"] / "trains").glob("*/components/runtime-apple/publication/published.json"))
    matching = [json.loads(path.read_text()) for path in records if path.is_file() and not path.is_symlink()]
    matching = [item for item in matching if item.get("status") == "remote-verified" and
                item.get("details", {}).get("tag") == release_tag and
                (expected_sha is None or item.get("details", {}).get("files", {}).get(
                    "AxiomRuntime.xcframework.zip") == expected_sha)]
    if not matching:
        raise ctl.ReleaseError(f"publish and verify runtime-apple {version} before this SDK")
    sha = matching[-1]["details"]["files"]["AxiomRuntime.xcframework.zip"]
    main = ctl.repo_path(candidate["catalog"], "AxiomCore", candidate["workspace"])
    publisher._verify_github_assets("AxiomCore/AxiomCore", release_tag,
                                    {"AxiomRuntime.xcframework.zip": sha},
                                    candidate["directory"], main)
    return sha


def _npm_archive(candidate: dict, component: str) -> tuple[Path, dict[str, dict]]:
    records = _artifacts(candidate, component)
    tgz = [item for name, item in records.items() if name.endswith(".tgz")]
    expected_names = 4 if component == "sdk-atmx-web" else 1
    if len(tgz) != 1 or len(records) != expected_names:
        raise ctl.ReleaseError(f"{component} needs one npm tarball"
                               + (" and three exact browser assets" if expected_names == 4 else ""))
    archive = Path(tgz[0]["path"])
    manifest = json.loads(_tar_member(archive, "package/package.json"))
    if manifest.get("name") != NPM[component] or manifest.get("version") != _version(candidate, component):
        raise ctl.ReleaseError("npm tarball identity differs from staged component version")
    if component == "sdk-atmx-web":
        for name in ("atmx.umd.js", "atmx.es.js", "axiom_runtime.wasm"):
            item = records.get(name)
            if not item or ctl.sha256(_tar_member(archive, f"package/dist/{name}")) != item["sha256"]:
                raise ctl.ReleaseError(f"npm tarball and R2 asset differ: {name}")
    if component == "sdk-atmx-react":
        pinned = manifest.get("dependencies", {}).get("atmx-web", "")
        if not re.fullmatch(r"\^\d+\.\d+\.\d+", pinned):
            raise ctl.ReleaseError("React package must pin a published atmx-web SemVer range")
    return archive, records


def _r2_command(args: list[str], env: dict[str, str], cwd: Path) -> bytes:
    keys = ("CLOUDFLARE_ACCOUNT_ID", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY")
    present = [key for key in keys if env.get(key)]
    if len(present) != len(keys):
        if present:
            raise ctl.ReleaseError("partial R2 credentials")
        if not shutil.which("infisical", path=env.get("PATH")):
            raise ctl.ReleaseError("R2 upload requires Infisical prod or scoped S3 credentials")
        return ctl.run("infisical", "run", "--env=prod", "--", *args, cwd=cwd, env=env)
    return ctl.run(*args, cwd=cwd, env=env)


def _r2_account(env: dict[str, str], cwd: Path) -> str:
    value = env.get("CLOUDFLARE_ACCOUNT_ID")
    if not value:
        if not shutil.which("infisical", path=env.get("PATH")):
            raise ctl.ReleaseError("R2 upload requires CLOUDFLARE_ACCOUNT_ID")
        value = ctl.run("infisical", "run", "--env=prod", "--", "printenv",
                        "CLOUDFLARE_ACCOUNT_ID", cwd=cwd, env=env).decode().strip()
    if not re.fullmatch(r"[a-fA-F0-9]{32}", value):
        raise ctl.ReleaseError("Cloudflare account ID is not a 32-character hex identifier")
    return value


def _r2_probe(args: list[str], env: dict[str, str], cwd: Path) -> subprocess.CompletedProcess:
    keys = ("CLOUDFLARE_ACCOUNT_ID", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY")
    present = [key for key in keys if env.get(key)]
    if present and len(present) != len(keys):
        raise ctl.ReleaseError("partial R2 credentials")
    command = args if present else ["infisical", "run", "--env=prod", "--", *args]
    return subprocess.run(command, cwd=cwd, env=env, check=False,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def _publish_r2(candidate: dict, version: str, records: dict[str, dict], env: dict[str, str]) -> dict:
    account = _r2_account(env, candidate["directory"])
    endpoint = f"https://{account}.r2.cloudflarestorage.com"
    directory, _, _ = _publication(candidate, "sdk-atmx-web")
    directory.mkdir(parents=True, exist_ok=True)
    proof = {}
    for name in ("atmx.umd.js", "atmx.es.js", "axiom_runtime.wasm"):
        key = f"v{version}/{name}"
        uri = f"s3://atmx/{key}"
        local = directory / f"remote-{name}"
        fetch = ["aws", "s3", "cp", uri, str(local), "--endpoint-url", endpoint, "--no-progress"]
        # A missing versioned key may be created; an existing key is never replaced.
        exists = _r2_probe(["aws", "s3api", "head-object", "--bucket", "atmx", "--key", key,
                            "--endpoint-url", endpoint], env, candidate["directory"])
        if exists.returncode:
            message = exists.stderr.decode(errors="replace")
            if "404" not in message and "Not Found" not in message:
                raise ctl.ReleaseError(f"cannot inspect R2 {key}: {message.strip()}")
            mime = "application/wasm" if name.endswith(".wasm") else "application/javascript"
            _r2_command(["aws", "s3", "cp", records[name]["path"], uri,
                         "--endpoint-url", endpoint, "--content-type", mime,
                         "--cache-control", "public,max-age=31536000,immutable", "--no-progress"], env,
                        candidate["directory"])
        _r2_command(fetch, env, candidate["directory"])
        if ctl.sha256_file(local) != records[name]["sha256"]:
            raise ctl.ReleaseError(f"R2 object differs from exact staged asset: {key}")
        public_url = f"https://atmx.axiomcore.dev/{urllib.parse.quote(key)}"
        with urllib.request.urlopen(public_url, timeout=30) as response:
            if ctl.sha256(response.read()) != records[name]["sha256"]:
                raise ctl.ReleaseError(f"public R2 domain serves different bytes: {key}")
        proof[key] = records[name]["sha256"]
    return proof


def publish_npm(candidate: dict, component: str) -> dict:
    if component not in NPM:
        raise ctl.ReleaseError(f"no npm publisher for {component}")
    publisher.remote_source_heads(candidate)
    archive, records = _npm_archive(candidate, component)
    version = _version(candidate, component)
    package = NPM[component]
    env = ctl.build_environment(candidate["root"], component)
    if component == "sdk-atmx-react":
        manifest = json.loads(_tar_member(archive, "package/package.json"))
        web_version = manifest["dependencies"]["atmx-web"].removeprefix("^")
        if _npm_metadata("atmx-web", web_version, candidate["directory"], env) is None:
            raise ctl.ReleaseError(f"publish atmx-web@{web_version} before the React adapter")
    if component == "sdk-atmx-web":
        r2 = _publish_r2(candidate, version, records, env)
    else:
        r2 = {}
    expected_integrity = "sha512-" + base64.b64encode(hashlib.sha512(archive.read_bytes()).digest()).decode()
    # Public registry reads never need the Infisical npm token. Prevent npm's
    # token fallback even when an old operator shell still exports it.
    npm_env = {key: value for key, value in env.items()
               if key not in ("NPM_TOKEN", "NODE_AUTH_TOKEN", "NPM_CONFIG_USERCONFIG")}
    npm_env["NPM_CONFIG_USERCONFIG"] = os.devnull
    metadata = _npm_metadata(package, version, candidate["directory"], npm_env)
    oidc = {}
    if metadata is None:
        oidc = npm_oidc.publish(candidate, component, archive)
        metadata = _npm_metadata(package, version, candidate["directory"], npm_env)
    if metadata is None or metadata.get("dist", {}).get("integrity") != expected_integrity:
        raise ctl.ReleaseError(f"npm {package}@{version} integrity differs from exact staged tarball")
    details = {"version": version, "integrity": expected_integrity, "r2": r2}
    if oidc:
        details["oidc"] = oidc
    else:
        _, published, _ = _publication(candidate, component)
        if published.exists():
            previous_oidc = json.loads(published.read_text()).get("details", {}).get("oidc")
            if previous_oidc:
                details["oidc"] = previous_oidc
    return _save(candidate, component, f"npm: {package}",
                 f"https://www.npmjs.com/package/{package}/v/{version}",
                 details)


def publish_pub(candidate: dict, component: str) -> dict:
    if component not in PUB:
        raise ctl.ReleaseError(f"no pub.dev publisher for {component}")
    publisher.remote_source_heads(candidate)
    package = PUB[component]
    version = _version(candidate, component)
    archive = _single(candidate, component, f"{package}-{version}.tar.gz")
    if component == "sdk-flutter":
        pins = set()
        for platform_name in ("ios", "macos"):
            body = _tar_member(archive, f"{platform_name}/axiom_flutter.podspec").decode()
            found = re.search(r"(?m)^\s*runtime_version\s*=\s*'([^']+)'", body)
            if not found:
                raise ctl.ReleaseError(f"Flutter {platform_name} podspec has no pinned Apple runtime")
            pins.add(found.group(1))
        if len(pins) != 1:
            raise ctl.ReleaseError("Flutter podspecs disagree on Apple runtime version")
        _verified_apple_runtime(candidate, next(iter(pins)))
    directory, _, _ = _publication(candidate, component)
    source = directory / "package"
    if not source.exists():
        if directory.exists():
            raise ctl.ReleaseError(f"incomplete pub publication workspace: {directory}")
        source.mkdir(parents=True)
        with tarfile.open(archive, "r:gz") as opened:
            for member in opened:
                path = Path(member.name)
                if path.is_absolute() or ".." in path.parts:
                    raise ctl.ReleaseError(f"unsafe pub package archive member: {member.name}")
                if member.isdir():
                    continue
                if not member.isfile():
                    raise ctl.ReleaseError(f"unsafe pub package archive member: {member.name}")
                target = source / path
                target.parent.mkdir(parents=True, exist_ok=True)
                stream = opened.extractfile(member)
                if stream is None:
                    raise ctl.ReleaseError(f"unreadable pub package member: {member.name}")
                with stream, target.open("xb") as output:
                    shutil.copyfileobj(stream, output)
    if not re.search(rf"^name:\s*{re.escape(package)}\s*$", (source / "pubspec.yaml").read_text(), re.M) or not re.search(
            rf"^version:\s*{re.escape(version)}\s*$", (source / "pubspec.yaml").read_text(), re.M):
        raise ctl.ReleaseError("pubspec identity differs from staged component")
    url = f"https://pub.dev/api/packages/{package}/versions/{version}"
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            data = json.load(response)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        data = None
    if data is None:
        env = ctl.build_environment(candidate["root"], component)
        helper = str(Path(__file__).with_name("pub_publish.py"))
        command = [sys.executable, helper]
        if not env.get("PUB_TOKEN"):
            if not shutil.which("infisical", path=env.get("PATH")):
                raise ctl.ReleaseError("pub.dev publisher needs PUB_TOKEN from Infisical prod or environment")
            command = ["infisical", "run", "--env=prod", "--", *command]
        ctl.run(*command, cwd=source, env=env, capture=False)
        with urllib.request.urlopen(url, timeout=30) as response:
            data = json.load(response)
    remote_archive = data.get("archive_url") if isinstance(data, dict) else None
    parsed_archive = urllib.parse.urlsplit(remote_archive or "")
    if (parsed_archive.scheme != "https" or not parsed_archive.hostname or
            parsed_archive.username or parsed_archive.password or parsed_archive.fragment):
        raise ctl.ReleaseError("pub.dev did not return a trusted HTTPS archive URL")
    with urllib.request.urlopen(remote_archive, timeout=60) as response:
        remote_bytes = response.read()
    remote_digest = data.get("archive_sha256")
    if remote_digest and remote_digest != ctl.sha256(remote_bytes):
        raise ctl.ReleaseError("pub.dev archive checksum differs from its metadata")
    def manifest(path: Path) -> dict[str, str]:
        with tarfile.open(path, "r:gz") as opened:
            return {member.name.removeprefix("./"): ctl.sha256(opened.extractfile(member).read())
                    for member in opened if member.isfile() and opened.extractfile(member)}
    remote_path = directory / "pub-remote.tar.gz"
    if not remote_path.exists():
        with remote_path.open("xb") as output:
            output.write(remote_bytes)
    elif remote_path.read_bytes() != remote_bytes:
        raise ctl.ReleaseError("pub.dev archive changed between verification attempts")
    if manifest(archive) != manifest(remote_path):
        raise ctl.ReleaseError("pub.dev package files differ from the staged package archive")
    return _save(candidate, component, f"pub.dev: {package}",
                 f"https://pub.dev/packages/{package}/versions/{version}",
                 {"version": version, "fileSha256": manifest(archive)})


def publish_swift(candidate: dict) -> dict:
    component = "sdk-swift"
    publisher.remote_source_heads(candidate)
    package_file = _single(candidate, component, "Package.swift")
    sdk = ctl.repo_path(candidate["catalog"], "axiom-sdk", candidate["workspace"])
    publisher._assert_github_origin("AxiomCore/axiom-sdk", sdk)
    version = _version(candidate, component)
    source_head = candidate["plan"]["repositories"]["axiom-sdk"]["head"]
    if package_file.read_bytes() != (sdk / "swift/Package.swift").read_bytes():
        raise ctl.ReleaseError("staged Swift package differs from the reviewed source")
    body = package_file.read_text()
    match = re.search(r'(?m)^\s*checksum:\s*"([a-f0-9]{64})"\s*$', body)
    runtime = re.search(r"releases/download/v([^/]+)/AxiomRuntime\.xcframework\.zip", body)
    if not match or not runtime:
        raise ctl.ReleaseError("Swift Package.swift lacks a pinned Apple runtime URL and checksum")
    # The Apple artifact must already have been remotely verified by its owner.
    release_tag = f"v{runtime.group(1)}"
    _verified_apple_runtime(candidate, runtime.group(1), match.group(1))
    tag = f"v{version}"
    existing = publisher._remote_tag_target(tag, sdk)
    if existing is not None and existing != source_head:
        raise ctl.ReleaseError(f"Swift tag {tag} points to a different source commit")
    if existing is None:
        ctl.run("git", "push", "origin", f"{source_head}:refs/tags/{tag}", cwd=sdk, capture=False)
    publisher._verify_remote_tag("AxiomCore/axiom-sdk", tag, source_head, sdk)
    return _save(candidate, component, "Git source tag: AxiomCore/axiom-sdk",
                 f"https://github.com/AxiomCore/axiom-sdk/tree/{tag}",
                 {"tag": tag, "sourceHead": source_head, "runtimeTag": release_tag})


def _gcloud_json(*command: str, cwd: Path) -> dict:
    return json.loads(ctl.run("gcloud", *command, "--format=json", "--quiet", cwd=cwd).decode())


def _worker_trigger_state(kind: str, name: str, project: str, region: str,
                          owner: Path) -> str:
    command = (("scheduler", "jobs", "describe") if kind == "scheduler"
               else ("tasks", "queues", "describe"))
    try:
        state = _gcloud_json(*command, name, f"--project={project}",
                             f"--location={region}", cwd=owner).get("state")
    except ctl.ReleaseError as exc:
        if re.search(r"\bNOT_FOUND\b|not found|does not exist", str(exc), re.I):
            return "MISSING"
        raise
    if not isinstance(state, str):
        raise ctl.ReleaseError(f"{kind} {name} has no verifiable state")
    return state


def _pause_release_worker_triggers(project: str, region: str, owner: Path) -> None:
    """Fail closed before and after publishing API/worker images."""
    scheduler = os.environ.get("AXIOM_GCP_RELEASE_SCHEDULER_JOB", os.environ.get(
        "AXIOM_GCP_SEMANTIC_SCHEDULER_JOB", "axiom-release-worker-drain"))
    queue = os.environ.get("AXIOM_CLOUD_TASKS_QUEUE", "axiom-release-pipeline")
    for kind, name, active in (("scheduler", scheduler, "ENABLED"),
                               ("queue", queue, "RUNNING")):
        state = _worker_trigger_state(kind, name, project, region, owner)
        if state == active:
            command = (("scheduler", "jobs", "pause") if kind == "scheduler"
                       else ("tasks", "queues", "pause"))
            ctl.run("gcloud", *command, name, f"--project={project}",
                    f"--location={region}", "--quiet", cwd=owner, capture=False)
            state = _worker_trigger_state(kind, name, project, region, owner)
        if state not in ("PAUSED", "MISSING"):
            raise ctl.ReleaseError(f"release worker {kind} {name} is {state}, not paused")


def _container_images(value: object) -> list[str]:
    if isinstance(value, dict):
        found = [value["image"]] if isinstance(value.get("image"), str) else []
        return found + [image for nested in value.values() for image in _container_images(nested)]
    if isinstance(value, list):
        return [image for nested in value for image in _container_images(nested)]
    return []


def _environment_values(value: object, name: str) -> list[str]:
    if isinstance(value, dict):
        found = [value["value"]] if value.get("name") == name and isinstance(value.get("value"), str) else []
        return found + [item for nested in value.values() for item in _environment_values(nested, name)]
    if isinstance(value, list):
        return [item for nested in value for item in _environment_values(nested, name)]
    return []


def publish_gcp(candidate: dict, component: str) -> dict:
    if component not in GCP:
        raise ctl.ReleaseError(f"no GCP publisher for {component}")
    publisher.remote_source_heads(candidate)
    kind, owner_name, repository, default_name = GCP[component]
    descriptor = json.loads(_single(candidate, component, "image-ref.json").read_text())
    image = descriptor.get("image")
    if not isinstance(image, str) or not re.fullmatch(
            rf"[a-z][a-z0-9-]*-docker\.pkg\.dev/[a-z][a-z0-9-]*/{repository}/[a-z0-9-]+@sha256:[a-f0-9]{{64}}", image):
        raise ctl.ReleaseError(f"{component} image-ref.json lacks an immutable Artifact Registry digest")
    if descriptor.get("sourceHeads") != candidate["plan"]["repositories"]:
        raise ctl.ReleaseError("image descriptor is not bound to pinned source commits")
    build_id = descriptor.get("buildId")
    if not isinstance(build_id, str) or not re.fullmatch(r"[a-fA-F0-9-]{36}", build_id):
        raise ctl.ReleaseError("image descriptor needs its successful Cloud Build ID")
    region, project = image.split("-docker.pkg.dev/", 1)
    project = project.split("/", 1)[0]
    configured_project = os.environ.get("AXIOM_GCP_PROJECT_ID", "axiomcore")
    configured_region = os.environ.get("AXIOM_GCP_REGION")
    if project != configured_project or region != configured_region:
        raise ctl.ReleaseError("image project/region differs from explicit release deployment configuration")
    owner = ctl.repo_path(candidate["catalog"], owner_name, candidate["workspace"])
    build = _gcloud_json("builds", "describe", build_id, f"--project={project}", cwd=owner)
    expected_source_digest = ctl.sha256(ctl.canonical(candidate["plan"]["repositories"]))
    images = build.get("results", {}).get("images", [])
    if (build.get("status") != "SUCCESS" or build.get("substitutions", {}).get(
            "_AXIOM_SOURCE_HEADS_SHA256") != expected_source_digest or not any(
                item.get("digest") == image.rsplit("@", 1)[1] for item in images)):
        raise ctl.ReleaseError("Cloud Build proof does not bind the successful image to pinned source heads")
    image_info = _gcloud_json("artifacts", "docker", "images", "describe", image,
                              f"--project={project}", cwd=owner)
    digest = image.rsplit("@", 1)[1]
    if digest not in json.dumps(image_info):
        raise ctl.ReleaseError("Artifact Registry did not confirm the exact image digest")
    if component in ("backend-api", "backend-worker"):
        _pause_release_worker_triggers(project, region, owner)
    name = os.environ.get({"backend-api": "AXIOM_GCP_SERVICE",
                           "backend-worker": "AXIOM_GCP_RELEASE_WORKER_JOB",
                           "mock-runner": "AXIOM_GCP_MOCK_RUNNER_IMAGE_NAME",
                           "contract-test-runner": "AXIOM_GCP_CONTRACT_TEST_RUNNER_JOB",
                           "dashboard-origin": "AXIOM_GCP_DASHBOARD_SERVICE"}[component], default_name)
    image_name = os.environ.get({"backend-api": "AXIOM_GCP_SERVICE",
                                 "backend-worker": "AXIOM_GCP_RELEASE_WORKER_JOB",
                                 "mock-runner": "AXIOM_GCP_MOCK_RUNNER_IMAGE_NAME",
                                 "contract-test-runner": "AXIOM_GCP_CONTRACT_TEST_RUNNER_IMAGE_NAME",
                                 "dashboard-origin": "AXIOM_GCP_DASHBOARD_IMAGE_NAME"}[component], default_name)
    if image.split("@", 1)[0].rsplit("/", 1)[1] != image_name:
        raise ctl.ReleaseError(f"{component} staged image is not its configured image name")
    if kind == "tag":
        managed = image.split("@", 1)[0] + ":managed"
        ctl.run("gcloud", "artifacts", "docker", "tags", "add", image, managed,
                f"--project={project}", "--quiet", cwd=owner, capture=False)
        remote = _gcloud_json("artifacts", "docker", "images", "describe", managed,
                              f"--project={project}", cwd=owner)
        if digest not in json.dumps(remote):
            raise ctl.ReleaseError("managed image tag did not resolve to staged digest")
        address = managed
        details = {"image": image, "managedTag": managed}
    elif kind == "service":
        _gcloud_json("run", "services", "describe", name, f"--project={project}",
                     f"--region={region}", cwd=owner)
        deploy = ["gcloud", "run", "deploy", name, f"--project={project}", f"--region={region}",
                  "--platform=managed", f"--image={image}"]
        if component == "backend-api":
            deploy.append("--update-env-vars=AXIOM_RELEASE_WORKER_ENABLED=false")
        ctl.run(*deploy, "--quiet", cwd=owner, capture=False)
        remote = _gcloud_json("run", "services", "describe", name, f"--project={project}",
                              f"--region={region}", cwd=owner)
        template = remote.get("spec", {}).get("template", {})
        actual = _container_images(template)
        ready = remote.get("status", {}).get("latestReadyRevisionName")
        traffic = remote.get("status", {}).get("traffic", [])
        if image not in actual or not ready or not any(
                item.get("revisionName") == ready and item.get("percent") == 100 for item in traffic):
            raise ctl.ReleaseError("Cloud Run service did not become ready on the exact image digest")
        if component == "backend-api" and _environment_values(
                template, "AXIOM_RELEASE_WORKER_ENABLED") != ["false"]:
            raise ctl.ReleaseError("backend API revision did not retain disabled release-worker dispatch")
        address = remote.get("status", {}).get("url")
        if not isinstance(address, str) or not address.startswith("https://"):
            raise ctl.ReleaseError("Cloud Run service has no HTTPS URL")
        details = {"image": image, "revision": ready}
    else:
        _gcloud_json("run", "jobs", "describe", name, f"--project={project}",
                     f"--region={region}", cwd=owner)
        deploy = ["gcloud", "run", "jobs", "update", name, f"--project={project}",
                  f"--region={region}", f"--image={image}"]
        if component == "backend-worker":
            deploy.extend(("--update-env-vars=AXIOM_RELEASE_WORKER_ENABLED=false",
                           "--max-retries=0"))
        ctl.run(*deploy, "--quiet", cwd=owner, capture=False)
        remote = _gcloud_json("run", "jobs", "describe", name, f"--project={project}",
                              f"--region={region}", cwd=owner)
        actual = _container_images(remote.get("spec", {}).get("template", {}))
        if image not in actual:
            raise ctl.ReleaseError("Cloud Run Job does not reference the exact image digest")
        if component == "backend-worker":
            worker_spec = remote.get("spec", {}).get("template", {}).get("spec", {}).get(
                "template", {}).get("spec", {})
            if (_environment_values(worker_spec, "AXIOM_RELEASE_WORKER_ENABLED") != ["false"]
                    or worker_spec.get("maxRetries") != 0):
                raise ctl.ReleaseError("backend worker Job did not retain disabled mode and zero retries")
        address = f"https://console.cloud.google.com/run/jobs/details/{region}/{name}?project={project}"
        details = {"image": image, "job": name}
    if component in ("backend-api", "backend-worker"):
        _pause_release_worker_triggers(project, region, owner)
    return _save(candidate, component, f"GCP {project}/{region}", address, details)


def _proxy_identity(worker: bytes) -> tuple[str, str]:
    text = worker.decode("utf-8")
    marker = re.search(r'const AXIOM_RELEASE_SHA256 = "([a-f0-9]{64})";', text)
    origin = re.search(r'^const DASHBOARD_ORIGIN = ("(?:[^"\\]|\\.)*");$', text, re.M)
    if not marker or not origin:
        raise ctl.ReleaseError("dashboard proxy lacks a pinned release marker or origin")
    canonical = text.replace(marker.group(1), "__AXIOM_RELEASE_SHA256__", 1).encode()
    if ctl.sha256(canonical) != marker.group(1):
        raise ctl.ReleaseError("dashboard proxy release marker differs from worker bytes")
    url = json.loads(origin.group(1))
    parsed = urllib.parse.urlsplit(url)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.hostname == "app.axiomcore.dev"
            or parsed.username or parsed.password or parsed.query or parsed.fragment):
        raise ctl.ReleaseError("dashboard proxy has an invalid HTTPS origin")
    return marker.group(1), url


def _verify_proxy(url: str, marker: str, origin: str) -> None:
    parsed = urllib.parse.urlsplit(url)
    hostname = parsed.hostname or ""
    if (parsed.scheme != "https" or parsed.port is not None or parsed.path not in ("", "/")
            or (hostname != "axiom-dashboard.pages.dev" and not hostname.endswith(
                ".axiom-dashboard.pages.dev"))):
        raise ctl.ReleaseError("dashboard proxy deployment URL is not under its Pages project")
    proof_url = url.rstrip("/") + "/.well-known/axiom-dashboard-release"
    with urllib.request.urlopen(proof_url, timeout=30) as response:
        if urllib.parse.urlsplit(response.geturl()).hostname != hostname:
            raise ctl.ReleaseError("dashboard proxy release proof redirected elsewhere")
        observed = json.load(response)
    if observed != {"sha256": marker, "origin": origin}:
        raise ctl.ReleaseError("deployed dashboard proxy marker or origin differs from staged worker")
    # The proxy also has to forward a normal request without an upstream 5xx.
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            if response.status >= 500:
                raise ctl.ReleaseError("dashboard proxy origin is unhealthy")
    except urllib.error.HTTPError as error:
        if error.code >= 500:
            raise ctl.ReleaseError("dashboard proxy origin returned a server error") from error


def publish_dashboard_proxy(candidate: dict) -> dict:
    component = "dashboard-proxy"
    publisher.remote_source_heads(candidate)
    worker = _single(candidate, component, "_worker.js").read_bytes()
    marker, origin = _proxy_identity(worker)
    owner = ctl.repo_path(candidate["catalog"], "axiom-frontend", candidate["workspace"])
    directory, published, stage_sha = _publication(candidate, component)
    deployment = directory / "deployment.json"
    if deployment.exists():
        record = json.loads(deployment.read_text())
        if record.get("stageSha256") != stage_sha or record.get("marker") != marker:
            raise ctl.ReleaseError("dashboard proxy deployment evidence belongs to another stage")
        url = record["url"]
    else:
        if directory.exists():
            raise ctl.ReleaseError(f"incomplete dashboard proxy deployment: {directory}")
        environment = ctl.build_environment(candidate["root"], component)
        publisher._check_secret_names(("CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN"),
                                      publisher._cloudflare_command, environment, owner)
        directory.mkdir(parents=True)
        site = directory / "site"
        site.mkdir()
        (site / "_worker.js").write_bytes(worker)
        source = directory / "tooling"
        source.mkdir()
        ctl.stage_tracked_landing_source(owner, source)
        ctl.run("pnpm", "install", "--frozen-lockfile", "--filter", "axiom-landing...",
                cwd=source, env=environment, capture=False)
        wrangler = ["pnpm", "--filter", "axiom-landing", "exec", "wrangler"]
        projects = json.loads(ctl.run(*publisher._cloudflare_command(
            [*wrangler, "pages", "project", "list", "--json"], environment),
            cwd=source, env=environment).decode())
        if isinstance(projects, dict):
            projects = projects.get("result", [])
        if not isinstance(projects, list) or not any(item.get("name") == "axiom-dashboard" for item in projects):
            raise ctl.ReleaseError("Cloudflare Pages project axiom-dashboard is missing")
        head = candidate["plan"]["repositories"]["axiom-frontend"]["head"]
        output = publisher._run_with_combined_output(publisher._cloudflare_command(
            [*wrangler, "pages", "deploy", str(site), "--project-name", "axiom-dashboard",
             "--branch", "main", "--commit-hash", head], environment), source, environment)
        urls = [url for url in re.findall(r"https://[a-zA-Z0-9.-]+\.pages\.dev", output)
                if (urllib.parse.urlsplit(url).hostname or "").endswith(".axiom-dashboard.pages.dev")]
        if not urls:
            raise ctl.ReleaseError("Wrangler did not return a dashboard Pages deployment URL")
        url = urls[-1]
        ctl.write_json(deployment, {"stageSha256": stage_sha, "url": url,
                                    "marker": marker, "origin": origin})
    _verify_proxy(url, marker, origin)
    _verify_proxy("https://axiom-dashboard.pages.dev", marker, origin)
    return _save(candidate, component, "Cloudflare Pages: axiom-dashboard", url,
                 {"workerSha256": ctl.sha256(worker), "marker": marker, "origin": origin})


def publish(candidate: dict, component: str) -> dict:
    if component in GITHUB:
        return publish_github(candidate, component)
    if component in NPM:
        return publish_npm(candidate, component)
    if component in PUB:
        return publish_pub(candidate, component)
    if component == "sdk-swift":
        return publish_swift(candidate)
    if component in GCP:
        return publish_gcp(candidate, component)
    if component == "dashboard-proxy":
        return publish_dashboard_proxy(candidate)
    raise ctl.ReleaseError(f"{component} has no verified destination adapter")
