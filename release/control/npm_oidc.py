#!/usr/bin/env python3
"""Hand an exact staged npm archive to its owner repository's OIDC workflow."""

from __future__ import annotations

import json
from pathlib import Path
import re
import time

import ctl


WORKFLOW = "npm-trusted-publish.yml"
OWNERS = {
    "sdk-atmx-web": ("AxiomCore/atmx", "atmx-web", "atmx-web"),
    "sdk-atmx-react": ("AxiomCore/atmx-react", "atmx-react", "atmx-react"),
    "sdk-atmx-cli": ("AxiomCore/AxiomCore", "AxiomCore", "atmx-cli"),
}


def _gh_json(*args: str, cwd: Path) -> dict | list:
    return json.loads(ctl.run("gh", "api", *args, cwd=cwd))


def _release(repository: str, tag: str, cwd: Path) -> dict | None:
    try:
        return _gh_json(f"repos/{repository}/releases/tags/{tag}", cwd=cwd)
    except ctl.ReleaseError as error:
        if "HTTP 404" in str(error):
            return None
        raise


def _workflow_runs(repository: str, tag: str, cwd: Path) -> list[dict]:
    result = _gh_json(
        f"repos/{repository}/actions/workflows/{WORKFLOW}/runs?event=workflow_dispatch&per_page=100",
        cwd=cwd,
    )
    return [run for run in result.get("workflow_runs", [])
            if run.get("display_title") == f"npm {tag}"]


def _wait_for_run(repository: str, tag: str, cwd: Path) -> dict:
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        matching = _workflow_runs(repository, tag, cwd)
        if matching:
            return max(matching, key=lambda run: run["id"])
        time.sleep(5)
    raise ctl.ReleaseError(
        f"GitHub did not report the OIDC publish run for {tag}; inspect {repository} Actions "
        "before attempting another dispatch"
    )


def publish(candidate: dict, component: str, archive: Path) -> dict:
    """Stage immutable bytes once; resume/watch one workflow run on retries."""
    if component not in OWNERS:
        raise ctl.ReleaseError(f"no OIDC owner for {component}")
    repository, source_name, package = OWNERS[component]
    source = candidate["plan"]["repositories"].get(source_name)
    if not source or not re.fullmatch(r"[a-f0-9]{40}", source.get("head", "")):
        raise ctl.ReleaseError(f"no pinned {source_name} source head for npm publication")
    source_sha = source["head"]
    version = next(item["releaseVersion"] for item in candidate["stage"]["components"]
                   if item["id"] == component)
    train_id = candidate["intent"]["trainId"]
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", train_id):
        raise ctl.ReleaseError("invalid train ID for npm OIDC handoff")
    tag = f"npm-candidate-{component}-{train_id}-v{version}"
    sha256 = ctl.sha256_file(archive)
    directory = candidate["directory"] / "publication"
    marker = directory / "oidc-dispatch.json"
    expected = {"repository": repository, "tag": tag, "asset": archive.name,
                "sha256": sha256, "sourceSha": source_sha, "package": package,
                "version": version}
    if marker.exists() and json.loads(marker.read_text()) != expected:
        raise ctl.ReleaseError("saved npm OIDC dispatch belongs to different candidate bytes")

    # workflow_dispatch only works when the workflow exists on the default branch.
    repo_info = _gh_json(f"repos/{repository}", cwd=candidate["directory"])
    default_branch = repo_info["default_branch"]
    try:
        _gh_json(f"repos/{repository}/contents/.github/workflows/{WORKFLOW}?ref={default_branch}",
                 cwd=candidate["directory"])
    except ctl.ReleaseError as error:
        raise ctl.ReleaseError(
            f"{repository} must have {WORKFLOW} on its default branch {default_branch} "
            "before npm OIDC publication; merge and push the reviewed workflow"
        ) from error

    release = _release(repository, tag, candidate["directory"])
    if release is None:
        ctl.run("gh", "release", "create", tag, str(archive), "--repo", repository,
                "--target", source_sha, "--prerelease", "--title", f"npm handoff {package}@{version}",
                "--notes", f"Exact staged tarball for Axiom release train {train_id}; "
                "transport artifact for npm Trusted Publishing.", cwd=candidate["directory"], capture=False)
        release = _release(repository, tag, candidate["directory"])
    if (not release or release.get("draft") or not release.get("prerelease")
            or release.get("target_commitish") != source_sha):
        raise ctl.ReleaseError(f"npm handoff {tag} is not a prerelease at pinned source {source_sha}")
    assets = release.get("assets", [])
    if not assets:
        # gh release create performs release creation and asset upload as
        # separate calls. Recover a crash between them without overwriting.
        ctl.run("gh", "release", "upload", tag, str(archive), "--repo", repository,
                cwd=candidate["directory"], capture=False)
        release = _release(repository, tag, candidate["directory"])
        assets = release.get("assets", []) if release else []
    if len(assets) != 1 or assets[0].get("name") != archive.name or assets[0].get("size") != archive.stat().st_size:
        raise ctl.ReleaseError(f"npm handoff {tag} does not contain the one expected archive")
    directory.mkdir(parents=True, exist_ok=True)
    download = directory / "github-handoff"
    download.mkdir(exist_ok=True)
    remote_archive = download / archive.name
    if remote_archive.exists():
        remote_archive.unlink()
    ctl.run("gh", "release", "download", tag, "--repo", repository, "--pattern", archive.name,
            "--dir", str(download), cwd=candidate["directory"])
    if ctl.sha256_file(remote_archive) != sha256:
        raise ctl.ReleaseError(f"GitHub handoff asset differs from staged {component} archive")

    existing = _workflow_runs(repository, tag, candidate["directory"])
    if marker.exists() and not existing:
        run = _wait_for_run(repository, tag, candidate["directory"])
    elif existing:
        run = max(existing, key=lambda item: item["id"])
    else:
        ctl.write_json(marker, expected)  # A crash after dispatch must not create a duplicate run.
        ctl.run("gh", "workflow", "run", WORKFLOW, "--repo", repository,
                "--ref", default_branch,
                "-f", f"release_tag={tag}", "-f", f"archive_sha256={sha256}",
                "-f", f"source_sha={source_sha}", "-f", f"package_name={package}",
                "-f", f"package_version={version}", cwd=candidate["directory"])
        run = _wait_for_run(repository, tag, candidate["directory"])
    run_id = run["id"]
    if run.get("status") != "completed":
        try:
            ctl.run("gh", "run", "watch", str(run_id), "--repo", repository,
                    "--interval", "10", "--exit-status", cwd=candidate["directory"], capture=False)
        except ctl.ReleaseError as error:
            raise ctl.ReleaseError(
                f"npm OIDC workflow did not succeed: https://github.com/{repository}/actions/runs/{run_id}"
            ) from error
    final = _gh_json(f"repos/{repository}/actions/runs/{run_id}", cwd=candidate["directory"])
    if final.get("conclusion") != "success":
        raise ctl.ReleaseError(f"npm OIDC workflow failed: {final.get('html_url')}")
    return {"repository": repository, "workflowRun": final.get("html_url"),
            "handoffTag": tag, "archiveSha256": sha256}
