#!/usr/bin/env python3
"""Publish only exact, gated release artifacts; never rebuild while publishing."""

from __future__ import annotations

import datetime
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tarfile
import time
import urllib.parse
import urllib.request

import ctl
import gate
import train


HOST_IDS = ("ui-host-web", "ui-host-android", "ui-host-ios")
HOST_REPOSITORY = "AxiomCore/axiom-ui-host"
SIGNING_VARIABLES = ("AXIOM_UI_HOST_SIGNING_PRIVATE_KEY_HEX",
                     "AXIOM_UI_HOST_SIGNING_PUBLIC_KEY_HEX")


def _check_secret_names(names: tuple[str, ...], command_wrapper, env: dict[str, str], cwd: Path) -> None:
    check = [sys.executable, "-c",
             "import os,sys; names=" + repr(names) + "; "
             "missing=[name for name in names if not os.environ.get(name)]; "
             "sys.exit('missing release variables: '+', '.join(missing) if missing else 0)"]
    ctl.run(*command_wrapper(check, env), cwd=cwd, env=env)


def catalog_for_digest(digest: str, workspace: Path = ctl.WORKSPACE) -> dict:
    current = ctl.read_catalog()
    if current["sha256"] == digest:
        return current
    root = workspace / "AxiomCore"
    revisions = ctl.run("git", "log", "--all", "--format=%H", "--",
                        "release/control/catalog.toml", cwd=root).decode().splitlines()
    for revision in revisions:
        try:
            raw = ctl.run("git", "show", f"{revision}:release/control/catalog.toml", cwd=root)
        except ctl.ReleaseError:
            continue
        if ctl.sha256(raw) == digest:
            return ctl.parse_catalog(raw)
    raise ctl.ReleaseError("the staged catalog snapshot is unavailable in Git history; "
                           "do not publish a candidate whose catalog cannot be authenticated")


def load_candidate(component: str, root: Path, workspace: Path = ctl.WORKSPACE,
                   train_id: str | None = None) -> dict:
    # The active train ID is an address only. Authoritative intent, catalog,
    # plan, notes and bytes come from the immutable candidate directory.
    active = json.loads((ctl.CONTROL_DIR / "intent.json").read_text())
    selected_train = train_id or active["trainId"]
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", selected_train):
        raise ctl.ReleaseError("invalid release train ID")
    directory = train.train_paths(root, selected_train)["directory"] / "components" / component
    required = {name: directory / name for name in
                ("intent.json", "plan.json", "staged.json", "gate.json", "notes.md")}
    if any(not path.is_file() or path.is_symlink() for path in required.values()):
        raise ctl.ReleaseError(f"{component} has no complete staged candidate: {directory}")
    intent = json.loads(required["intent.json"].read_text())
    if intent.get("trainId") != selected_train:
        raise ctl.ReleaseError("candidate intent is for a different train")
    plan = json.loads(required["plan.json"].read_text())
    stage = json.loads(required["staged.json"].read_text())
    saved_gate = json.loads(required["gate.json"].read_text())
    catalog = catalog_for_digest(plan["catalogSha256"], workspace)
    if (stage.get("status") != "staged-not-published" or stage.get("trainId") != intent.get("trainId")
            or stage.get("catalogSha256") != catalog["sha256"]
            or saved_gate.get("phase") != "candidate-ready-for-publisher"
            or saved_gate.get("blockers") or saved_gate.get("stageSha256") != ctl.sha256(ctl.canonical(stage))):
        raise ctl.ReleaseError("candidate stage or saved gate is incomplete, altered, or blocked")
    receipt_paths = [ctl.artifact_receipt_path(root, item, plan["repositories"])
                     for item in plan["components"]]
    report = gate.inspect_candidate(intent, plan, catalog, workspace, stage, receipt_paths)
    if report["blockers"]:
        raise ctl.ReleaseError("candidate no longer passes the source/artifact gate: "
                               + "; ".join(report["blockers"]))
    if required["notes.md"].read_text() != ctl.release_notes(
            plan, catalog, workspace, enforce=True, train_id=intent["trainId"]):
        raise ctl.ReleaseError("candidate notes differ from the committed source fragments")
    return {"directory": directory, "intent": intent, "plan": plan, "stage": stage,
            "catalog": catalog, "receipts": receipt_paths, "root": root,
            "workspace": workspace}


def remote_source_heads(candidate: dict) -> None:
    """Require each pinned source commit to be the tracked remote branch tip."""
    catalog = candidate["catalog"]
    for name, snapshot in candidate["plan"]["repositories"].items():
        repository = ctl.repo_path(catalog, name, candidate["workspace"])
        upstream = ctl.run("git", "rev-parse", "--abbrev-ref", "--symbolic-full-name",
                           "@{upstream}", cwd=repository).decode().strip()
        if "/" not in upstream:
            raise ctl.ReleaseError(f"{name} has no remote tracking branch")
        remote, branch = upstream.split("/", 1)
        remote_head = ctl.run("git", "ls-remote", remote, f"refs/heads/{branch}",
                              cwd=repository).decode().split()
        if not remote_head or remote_head[0] != snapshot["head"]:
            raise ctl.ReleaseError(f"{name} source {snapshot['head']} is not the current {upstream} tip; "
                                   "push the reviewed commit before publication")


def signing_command(command: list[str], env: dict[str, str]) -> list[str]:
    present = [name for name in SIGNING_VARIABLES if env.get(name)]
    if len(present) == len(SIGNING_VARIABLES):
        return command
    if present:
        raise ctl.ReleaseError("partial UI Host signing environment; provide both keys or neither")
    if not shutil.which("infisical", path=env.get("PATH")):
        raise ctl.ReleaseError("UI Host manifest signing requires authenticated Infisical prod access")
    return ["infisical", "run", "--env=prod", "--", *command]


def _bundle_checksums(bundle: Path) -> dict[str, str]:
    files = sorted(bundle.iterdir())
    if not files or any(not file.is_file() or file.is_symlink() for file in files):
        raise ctl.ReleaseError(f"Host publication bundle has missing, linked, or non-file assets: {bundle}")
    return {file.name: ctl.sha256_file(file) for file in files}


def prepare_host_bundle(candidate: dict) -> tuple[Path, dict[str, str]]:
    directory = candidate["directory"] / "publication"
    bundle = directory / "bundle"
    marker = directory / "bundle-ready.json"
    stage = candidate["stage"]
    actual_ids = {item["id"] for item in stage["components"]}
    if actual_ids != set(HOST_IDS):
        raise ctl.ReleaseError("UI Host publication needs exactly web, Android, and iOS receipts")
    versions = {item["releaseVersion"] for item in stage["components"]}
    if len(versions) != 1:
        raise ctl.ReleaseError("UI Host targets have different release versions")
    if marker.exists():
        recorded = json.loads(marker.read_text())
        if (recorded.get("stageSha256") != ctl.sha256(ctl.canonical(stage))
                or recorded.get("files") != _bundle_checksums(bundle)):
            raise ctl.ReleaseError("previous publication bundle changed; inspect it before retrying")
        return bundle, recorded["files"]
    if directory.exists():
        raise ctl.ReleaseError(f"incomplete publication bundle exists; inspect before retrying: {directory}")
    host = ctl.repo_path(candidate["catalog"], "axiom-ui-host", candidate["workspace"])
    environment = ctl.build_environment(candidate["root"], "ui-host-web")
    _check_secret_names(SIGNING_VARIABLES, signing_command, environment, host)
    bundle.mkdir(parents=True)
    receipts = {json.loads(path.read_text())["component"]: ctl.verify_receipt(path)
                for path in candidate["receipts"]}
    assets = []
    for component_id in HOST_IDS:
        record = receipts[component_id]["artifacts"]
        if len(record) != 1:
            raise ctl.ReleaseError(f"{component_id} must have exactly one host archive")
        source = Path(record[0]["path"])
        destination = bundle / record[0]["file"]
        with source.open("rb") as input_file, destination.open("xb") as output_file:
            shutil.copyfileobj(input_file, output_file)
        if ctl.sha256_file(destination) != record[0]["sha256"]:
            raise ctl.ReleaseError(f"Host asset changed while packaging: {component_id}")
        target = component_id.removeprefix("ui-host-")
        variant = {"web": "browser", "android": "emulator", "ios": "simulator"}[target]
        assets.append({"target": target, "variant": variant, "file": destination.name,
                       "sha256": record[0]["sha256"]})
    engine = json.loads((host / "toolchain/engine-source.lock.json").read_text())["engine"]
    runtime_head = candidate["plan"]["repositories"]["axiom-runtime"]["head"]
    manifest = {"format": "axiom-ui-host-release/v1", "version": next(iter(versions)),
                "generatedAt": datetime.datetime.now(datetime.timezone.utc).replace(
                    microsecond=0).isoformat().replace("+00:00", "Z"),
                "engine": engine, "runtime": {"revision": runtime_head}, "assets": assets}
    ctl.write_json(bundle / "host-manifest.json", manifest)
    with (directory / "notes.md").open("x") as output_file:
        output_file.write((candidate["directory"] / "notes.md").read_text())
    signer = host / "scripts/sign-release.sh"
    verifier = host / "scripts/verify-release.sh"
    ctl.run(*signing_command([str(signer), str(bundle / "host-manifest.json")], environment),
            cwd=host, env=environment)
    ctl.run(*signing_command([str(verifier), str(bundle / "host-manifest.json")], environment),
            cwd=host, env=environment)
    files = _bundle_checksums(bundle)
    ctl.write_json(marker, {"stageSha256": ctl.sha256(ctl.canonical(stage)), "files": files})
    return bundle, files


def _github_release_exists(repository: str, tag: str, cwd: Path) -> bool:
    checked = subprocess.run(["gh", "api", f"repos/{repository}/releases/tags/{tag}"],
                             cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if checked.returncode == 0:
        return True
    message = checked.stderr.decode(errors="replace")
    if "HTTP 404" in message:
        return False
    raise ctl.ReleaseError(f"GitHub release lookup failed: {message.strip()}")


def _verify_github_assets(repository: str, tag: str, files: dict[str, str],
                          directory: Path, cwd: Path) -> dict:
    downloaded = Path(tempfile.mkdtemp(prefix="github-verify-", dir=directory))
    ctl.run("gh", "release", "download", tag, "--repo", repository,
            "--dir", str(downloaded), cwd=cwd)
    observed = _bundle_checksums(downloaded)
    if observed != files:
        raise ctl.ReleaseError("published GitHub assets differ from the exact verified candidate: "
                               + json.dumps({"expected": files, "observed": observed}, sort_keys=True))
    metadata = json.loads(ctl.run("gh", "api", f"repos/{repository}/releases/tags/{tag}",
                                  cwd=cwd).decode())
    if metadata.get("tag_name") != tag or metadata.get("draft") or metadata.get("prerelease"):
        raise ctl.ReleaseError("GitHub release is draft, prerelease, or points to the wrong tag")
    return {"url": metadata.get("html_url"), "id": metadata.get("id"),
            "downloadDirectory": str(downloaded)}


def _remote_tag_target(tag: str, cwd: Path) -> str | None:
    lines = ctl.run("git", "ls-remote", "--tags", "origin", f"refs/tags/{tag}",
                    f"refs/tags/{tag}^{{}}", cwd=cwd).decode().splitlines()
    peeled = [line.split()[0] for line in lines if line.endswith(f"refs/tags/{tag}^{{}}")]
    direct = [line.split()[0] for line in lines if line.endswith(f"refs/tags/{tag}")]
    targets = peeled or direct
    if len(targets) > 1:
        raise ctl.ReleaseError(f"multiple remote tag targets for {tag}")
    return targets[0] if targets else None


def _assert_github_origin(repository: str, cwd: Path) -> None:
    origin = ctl.run("git", "remote", "get-url", "origin", cwd=cwd).decode().strip()
    allowed = (f"git@github.com:{repository}.git", f"https://github.com/{repository}.git",
               f"https://github.com/{repository}")
    if origin not in allowed:
        raise ctl.ReleaseError(f"origin for {repository} is not the expected GitHub repository")


def _verify_remote_tag(repository: str, tag: str, expected_head: str, cwd: Path) -> None:
    if _remote_tag_target(tag, cwd) != expected_head:
        raise ctl.ReleaseError(f"{repository} tag {tag} does not point to pinned source {expected_head}")


def publish_host(candidate: dict) -> dict:
    if candidate["directory"].name != "ui-host":
        raise ctl.ReleaseError("publish the three UI Host targets as one ui-host release")
    root = candidate["root"]
    host = ctl.repo_path(candidate["catalog"], "axiom-ui-host", candidate["workspace"])
    _assert_github_origin(HOST_REPOSITORY, host)
    remote_source_heads(candidate)
    version = candidate["stage"]["components"][0]["releaseVersion"]
    tag = f"v{version}"
    host_head = candidate["plan"]["repositories"]["axiom-ui-host"]["head"]
    existing_tag = _remote_tag_target(tag, host)
    if existing_tag is not None and existing_tag != host_head:
        raise ctl.ReleaseError(f"remote UI Host tag {tag} belongs to another source commit")
    bundle, files = prepare_host_bundle(candidate)
    publication = candidate["directory"] / "publication"
    record_path = publication / "published.json"
    if not _github_release_exists(HOST_REPOSITORY, tag, host):
        if record_path.exists():
            raise ctl.ReleaseError("published evidence exists but the remote release is absent")
        args = ["gh", "release", "create", tag, *[str(bundle / name) for name in sorted(files)],
                "--repo", HOST_REPOSITORY, "--target", host_head,
                "--title", f"Axiom UI Host {version}", "--notes-file", str(publication / "notes.md")]
        print(f"Uploading {len(files)} exact Host assets to {HOST_REPOSITORY} {tag}...", flush=True)
        ctl.run(*args, cwd=host, capture=False)
    print(f"Downloading and verifying {len(files)} GitHub release asset(s)...", flush=True)
    _verify_remote_tag(HOST_REPOSITORY, tag, host_head, host)
    remote = _verify_github_assets(HOST_REPOSITORY, tag, files, publication, host)
    environment = ctl.build_environment(root, "ui-host-web")
    downloaded_manifest = Path(remote["downloadDirectory"]) / "host-manifest.json"
    ctl.run(*signing_command([str(host / "scripts/verify-release.sh"), str(downloaded_manifest)], environment),
            cwd=host, env=environment)
    evidence = {"format": "axiom-platform-component-publication/v1", "component": "ui-host",
                "trainId": candidate["intent"]["trainId"], "version": version,
                "stageSha256": ctl.sha256(ctl.canonical(candidate["stage"])),
                "destination": HOST_REPOSITORY, "tag": tag, "files": files, "remote": remote,
                "status": "remote-verified"}
    if record_path.exists():
        if json.loads(record_path.read_text()) != evidence:
            # Download directories vary across retries; compare immutable proof.
            saved = json.loads(record_path.read_text())
            comparable = {key: value for key, value in evidence.items() if key != "remote"}
            if {key: value for key, value in saved.items() if key != "remote"} != comparable:
                raise ctl.ReleaseError("published evidence differs from the verified remote release")
        return json.loads(record_path.read_text())
    ctl.write_json(record_path, evidence)
    return evidence


def _cloudflare_command(command: list[str], env: dict[str, str]) -> list[str]:
    keys = ("CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN")
    present = [key for key in keys if env.get(key)]
    if len(present) == len(keys):
        return command
    if present:
        raise ctl.ReleaseError("partial Cloudflare Pages environment; supply both values or use Infisical")
    if not shutil.which("infisical", path=env.get("PATH")):
        raise ctl.ReleaseError("Cloudflare Pages publication needs Infisical prod access")
    return ["infisical", "run", "--env=prod", "--", *command]


def _extract_static_archive(archive: Path, directory: Path) -> dict[str, str]:
    directory.mkdir(parents=True, exist_ok=False)
    observed = {}
    with tarfile.open(archive, "r:gz") as source:
        for member in source:
            path = Path(member.name)
            if (not member.isfile() or path.is_absolute() or ".." in path.parts
                    or not path.parts or member.name in observed):
                raise ctl.ReleaseError(f"unsafe static site archive member: {member.name}")
            target = directory / path
            target.parent.mkdir(parents=True, exist_ok=True)
            content = source.extractfile(member)
            if content is None:
                raise ctl.ReleaseError(f"static site archive member is unreadable: {member.name}")
            with content, target.open("xb") as output:
                shutil.copyfileobj(content, output)
            observed[member.name] = ctl.sha256_file(target)
    if "index.html" not in observed:
        raise ctl.ReleaseError("static site archive has no index.html")
    return observed


def _verify_pages_files(base_url: str, files: dict[str, str], domain: str = "axiom-landing.pages.dev") -> None:
    parsed = urllib.parse.urlsplit(base_url)
    hostname = parsed.hostname or ""
    if (parsed.scheme != "https" or parsed.port is not None or parsed.path not in ("", "/")
            or parsed.query or parsed.fragment or
            (hostname != domain and not hostname.endswith(f".{domain}"))):
        raise ctl.ReleaseError(f"Cloudflare deployment did not return an HTTPS URL for {domain}")
    count = sum(name not in ("_headers", "_redirects", "_routes.json") for name in files)
    print(f"Verifying {count} served file(s) at {base_url}...", flush=True)
    def verify_one(item: tuple[str, str]) -> None:
        name, expected = item
        if name in ("_headers", "_redirects", "_routes.json"):
            # These are Pages deployment directives, not served assets.
            return
        url = base_url.rstrip("/") + "/" + urllib.parse.quote(name)
        request = urllib.request.Request(url, headers={"User-Agent": "axiom-release-verifier/1"})
        for attempt in range(3):
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    final_host = urllib.parse.urlsplit(response.geturl()).hostname or ""
                    if final_host != hostname:
                        raise ctl.ReleaseError(f"Cloudflare Pages redirected {name} outside the verified deployment")
                    body = response.read()
                if ctl.sha256(body) != expected:
                    raise ctl.ReleaseError(f"Cloudflare Pages served bytes differ from the staged site: {name}")
                return
            except (OSError, ctl.ReleaseError):
                if attempt == 2:
                    raise
                time.sleep(1 + attempt)

    with ThreadPoolExecutor(max_workers=8) as pool:
        for _ in pool.map(verify_one, files.items()):
            pass
    print(f"Verified {count} served file(s) at {base_url}.", flush=True)


def _run_with_combined_output(command: list[str], cwd: Path, env: dict[str, str]) -> str:
    completed = subprocess.run(command, cwd=cwd, env=env, check=False,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = completed.stdout.decode(errors="replace")
    if completed.returncode:
        raise ctl.ReleaseError(f"Cloudflare Pages command failed ({completed.returncode}): {output.strip()}")
    return output


PAGES_COMPONENTS = {
    "landing": ("axiom-frontend", "axiom-landing", "axiom-landing-pages.tar.gz"),
    "docs": ("AxiomCore", "axiomcore-docs", "axiom-docs-pages.tar.gz"),
}


def _pages_project_domain(projects: list[dict], project: str) -> str | None:
    matches = [item for item in projects
               if item.get("name", item.get("Project Name")) == project]
    if len(matches) > 1:
        raise ctl.ReleaseError(f"Cloudflare returned duplicate Pages projects named {project}")
    if not matches:
        return None
    item = matches[0]
    domains = item.get("Project Domains") or item.get("domains") or item.get("subdomain") or ""
    if isinstance(domains, list):
        domains = ",".join(domains)
    candidates = [domain.strip().lower() for domain in domains.split(",")
                  if domain.strip().lower().endswith(".pages.dev")]
    if len(candidates) != 1 or not re.fullmatch(r"[a-z0-9.-]+\.pages\.dev", candidates[0]):
        raise ctl.ReleaseError(f"Cloudflare did not report one Pages domain for {project}")
    return candidates[0]


def _pages_project_list(wrangler: list[str], tool_source: Path, env: dict[str, str]) -> list[dict]:
    listed = ctl.run(*_cloudflare_command([*wrangler, "pages", "project", "list", "--json"], env),
                     cwd=tool_source, env=env).decode()
    projects = json.loads(listed)
    if isinstance(projects, dict):
        projects = projects.get("result", [])
    if not isinstance(projects, list) or any(not isinstance(item, dict) for item in projects):
        raise ctl.ReleaseError("Cloudflare Pages project list was not a JSON array")
    return projects


def _resume_pages_upload(candidate: dict, component: str, project: str,
                         archive: Path, publication: Path, stage_sha: str,
                         tool_source: Path, wrangler: list[str], env: dict[str, str]) -> dict:
    intent = json.loads((publication / "deploy-intent.json").read_text())
    if intent.get("stageSha256") != stage_sha or intent.get("project") != project:
        raise ctl.ReleaseError(f"prior {component} upload intent belongs to another stage")
    files = intent.get("files")
    if not isinstance(files, dict) or not files or any(
            not isinstance(name, str) or Path(name).is_absolute() or ".." in Path(name).parts
            or not isinstance(digest, str) or
            not (publication / "site" / name).is_file() or
            ctl.sha256_file(publication / "site" / name) != digest
            for name, digest in files.items()):
        raise ctl.ReleaseError(f"prior {component} upload files changed; inspect before retrying")
    # Re-read the immutable receipt's archive; a surviving work directory is not evidence.
    with tarfile.open(archive, "r:gz") as source:
        archived = {}
        for member in source:
            content = source.extractfile(member) if member.isfile() else None
            if content is None:
                raise ctl.ReleaseError("candidate archive changed during publication")
            with content:
                archived[member.name] = ctl.sha256(content.read())
    if archived != files:
        raise ctl.ReleaseError(f"prior {component} upload differs from candidate archive")
    domain = _pages_project_domain(_pages_project_list(wrangler, tool_source, env), project)
    if not domain:
        raise ctl.ReleaseError(f"Cloudflare Pages project {project} is missing after upload")
    listed = ctl.run(*_cloudflare_command([*wrangler, "pages", "deployment", "list",
                                           "--project-name", project, "--json"], env),
                     cwd=tool_source, env=env).decode()
    deployments = json.loads(listed)
    if not isinstance(deployments, list):
        raise ctl.ReleaseError("Cloudflare Pages deployment list was not a JSON array")
    head = candidate["plan"]["repositories"][PAGES_COMPONENTS[component][0]]["head"]
    for item in deployments:
        source = item.get("Source", "")
        url = item.get("Deployment", "")
        if (item.get("Branch") != "main" or not isinstance(source, str)
                or len(source) < 7 or not head.startswith(source)
                or not isinstance(url, str)):
            continue
        try:
            _verify_pages_files(url, files, domain)
            _verify_pages_files(f"https://{domain}", files, domain)
        except (OSError, ctl.ReleaseError):
            continue
        ctl.write_json(publication / "deployment.json",
                       {"stageSha256": stage_sha, "url": url, "domain": domain, "files": files})
        result = {"format": "axiom-platform-component-publication/v1", "component": component,
                  "trainId": candidate["intent"]["trainId"], "stageSha256": stage_sha,
                  "destination": f"Cloudflare Pages: {project}", "remote": url,
                  "files": files, "status": "remote-verified"}
        ctl.write_json(publication / "published.json", result)
        return result
    raise ctl.ReleaseError(f"no matching {component} deployment served the exact staged bytes; "
                           "nothing was uploaded again")


def publish_pages(candidate: dict, component: str) -> dict:
    if component not in PAGES_COMPONENTS or candidate["directory"].name != component:
        raise ctl.ReleaseError("Pages publisher received another component")
    owner_name, project, archive_name = PAGES_COMPONENTS[component]
    remote_source_heads(candidate)
    root = candidate["root"]
    owner = ctl.repo_path(candidate["catalog"], owner_name, candidate["workspace"])
    receipt = ctl.verify_receipt(candidate["receipts"][0])
    records = receipt["artifacts"]
    if len(records) != 1 or records[0]["file"] != archive_name:
        raise ctl.ReleaseError(f"{component} candidate needs one exact static site archive")
    publication = candidate["directory"] / "publication"
    deployment = publication / "deployment.json"
    published = publication / "published.json"
    stage_sha = ctl.sha256(ctl.canonical(candidate["stage"]))
    if deployment.exists():
        saved = json.loads(deployment.read_text())
        if saved.get("stageSha256") != stage_sha:
            raise ctl.ReleaseError(f"prior {component} deployment belongs to another stage")
        domain = saved.get("domain", f"{project}.pages.dev")
        _verify_pages_files(saved["url"], saved["files"], domain)
        _verify_pages_files(f"https://{domain}", saved["files"], domain)
        result = {"format": "axiom-platform-component-publication/v1", "component": component,
                  "trainId": candidate["intent"]["trainId"], "stageSha256": stage_sha,
                  "destination": f"Cloudflare Pages: {project}", "remote": saved["url"],
                  "files": saved["files"], "status": "remote-verified"}
        if published.exists():
            prior = json.loads(published.read_text())
            if prior != result:
                raise ctl.ReleaseError(f"{component} publication evidence differs from verified remote bytes")
            return prior
        ctl.write_json(published, result)
        return result
    env = ctl.build_environment(root, component)
    secret_context = owner / "docs" if component == "docs" else owner
    _check_secret_names(("CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN"),
                        _cloudflare_command, env, secret_context)
    if (publication / "deploy-intent.json").exists():
        tool_source = publication / "tooling"
        wrangler = (["pnpm", "--filter", "axiom-landing", "exec", "wrangler"]
                    if component == "landing" else ["pnpm", "exec", "wrangler"])
        if component == "docs":
            tool_source = tool_source / "docs"
        return _resume_pages_upload(candidate, component, project, Path(records[0]["path"]),
                                    publication, stage_sha, tool_source, wrangler, env)
    if publication.exists():
        raise ctl.ReleaseError(f"incomplete {component} publication exists; inspect before retrying: {publication}")
    publication.mkdir(parents=True)
    files = _extract_static_archive(Path(records[0]["path"]), publication / "site")
    tool_source = publication / "tooling"
    tool_source.mkdir()
    if component == "landing":
        ctl.stage_tracked_landing_source(owner, tool_source)
        ctl.run("pnpm", "install", "--frozen-lockfile", "--filter", "axiom-landing...",
                cwd=tool_source, env=env, capture=False)
        wrangler = ["pnpm", "--filter", "axiom-landing", "exec", "wrangler"]
    else:
        ctl.stage_tracked_source(owner, tool_source, ("docs", "README.md", "CONTRIBUTING.md"))
        ctl.run("pnpm", "install", "--frozen-lockfile", cwd=tool_source / "docs",
                env=env, capture=False)
        wrangler = ["pnpm", "exec", "wrangler"]
        tool_source = tool_source / "docs"
    domain = _pages_project_domain(_pages_project_list(wrangler, tool_source, env), project)
    if domain is None:
        print(f"Creating Cloudflare Pages project {project}...", flush=True)
        ctl.run(*_cloudflare_command([*wrangler, "pages", "project", "create",
                                      project, "--production-branch", "main", "--force"], env),
                cwd=tool_source, env=env)
        domain = _pages_project_domain(_pages_project_list(wrangler, tool_source, env), project)
        if domain is None:
            raise ctl.ReleaseError(f"Cloudflare did not list the created Pages project {project}")
    ctl.write_json(publication / "deploy-intent.json",
                   {"stageSha256": stage_sha, "project": project, "files": files})
    head = candidate["plan"]["repositories"][owner_name]["head"]
    print(f"Uploading staged {component} bytes to Cloudflare Pages {project}...", flush=True)
    output = _run_with_combined_output(_cloudflare_command([*wrangler, "pages", "deploy",
                                         str(publication / "site"), "--project-name", project,
                                         "--branch", "main", "--commit-hash", head], env), tool_source, env)
    urls = [url for url in re.findall(r"https://[a-zA-Z0-9.-]+\.pages\.dev", output)
            if (urllib.parse.urlsplit(url).hostname or "").endswith(f".{domain}")]
    if not urls:
        raise ctl.ReleaseError("Wrangler returned no deployment URL; inspect Cloudflare before retrying")
    url = urls[-1]
    ctl.write_json(deployment, {"stageSha256": stage_sha, "url": url, "domain": domain, "files": files})
    _verify_pages_files(url, files, domain)
    _verify_pages_files(f"https://{domain}", files, domain)
    result = {"format": "axiom-platform-component-publication/v1", "component": component,
              "trainId": candidate["intent"]["trainId"], "stageSha256": stage_sha,
              "destination": f"Cloudflare Pages: {project}", "remote": url,
              "files": files, "status": "remote-verified"}
    ctl.write_json(published, result)
    return result


def publish_landing(candidate: dict) -> dict:
    return publish_pages(candidate, "landing")


def publish_docs(candidate: dict) -> dict:
    return publish_pages(candidate, "docs")
