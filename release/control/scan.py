#!/usr/bin/env python3
"""Inventory local build-input changes against each repository's upstream.

This is a candidate-discovery aid, not an authenticated published baseline.
It never edits an intent, version file, or release artifact.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import ctl


def changed_paths(repo: Path) -> tuple[str, list[str], list[str]]:
    upstream = ctl.run("git", "rev-parse", "--abbrev-ref", "--symbolic-full-name",
                       "@{upstream}", cwd=repo).decode().strip()
    committed = [item.decode("utf-8", "surrogateescape") for item in
                 ctl.run("git", "diff", "--name-only", "-z", f"{upstream}..HEAD", cwd=repo).split(b"\0")
                 if item]
    working = sorted(ctl.git_snapshot(repo)["dirtyPaths"])
    return upstream, sorted(set(committed)), working


def inventory(catalog: dict, workspace: Path) -> dict:
    repositories = {}
    for name in catalog["repositories"]:
        repo = ctl.repo_path(catalog, name, workspace)
        upstream, committed, working = changed_paths(repo)
        repositories[name] = {"path": str(repo), "upstream": upstream,
                              "committedPaths": committed, "workingPaths": working}
    direct = set()
    components = []
    for definition in catalog["components"]:
        matched = []
        for source in definition["sources"]:
            repository = repositories[source["repo"]]
            for key in ("committedPaths", "workingPaths"):
                paths = [path for path in repository[key]
                         if any(ctl.matches(path, selector) for selector in source["paths"])]
                if paths:
                    matched.append({"repo": source["repo"], "state": "committed" if key == "committedPaths" else "working",
                                    "paths": paths})
        if matched:
            direct.add(definition["id"])
        components.append({"id": definition["id"], "directInputsChanged": bool(matched),
                           "inputs": matched, "affectedBy": []})
    affected = set(direct)
    by_id = {item["id"]: item for item in components}
    for definition in ctl.topological_components(catalog["components"]):
        deps = sorted(set(definition.get("depends_on", [])) & affected)
        if deps:
            by_id[definition["id"]]["affectedBy"] = deps
            affected.add(definition["id"])
    return {"format": "axiom-platform-local-change-inventory/v1",
            "comparison": "local upstream branches, not authenticated published artifacts",
            "repositories": repositories, "components": components,
            "affected": sorted(affected)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=ctl.CATALOG)
    parser.add_argument("--workspace", type=Path, default=ctl.WORKSPACE)
    args = parser.parse_args()
    try:
        report = inventory(ctl.read_catalog(args.catalog), args.workspace.resolve())
        for item in report["components"]:
            if item["id"] in report["affected"]:
                origins = ", ".join(f"{match['repo']}:{match['state']}" for match in item["inputs"])
                dependency = ", ".join(item["affectedBy"])
                print(f"CHANGE {item['id']:22} {origins or 'dependency: ' + dependency}")
        print("Comparison is against local upstream refs, not the published release baseline.")
        return 0
    except (ctl.ReleaseError, OSError, ValueError, KeyError) as error:
        print(f"release-scan: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
