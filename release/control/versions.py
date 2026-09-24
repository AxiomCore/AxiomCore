#!/usr/bin/env python3
"""Canonical candidate-version ledger and read-only component version view.

Package manifests remain the package-manager-facing mirrors. Published
identities come only from authenticated remote evidence, never this ledger.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys
import tempfile

import ctl


VERSIONS = ctl.CONTROL_DIR / "versions.json"
FORMAT = "axiom-platform-component-versions/v1"
STABLE_VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)")
TAGGED_COMPONENTS = {"runtime-apple", "ui-host-web", "ui-host-android", "ui-host-ios",
                     "sdk-swift", "extractor-go"}


def versioned(component: dict) -> bool:
    return bool(component.get("version")) or component["id"] in TAGGED_COMPONENTS


def read_versions(path: Path, catalog: dict) -> dict:
    document = json.loads(path.read_text())
    validate_versions(document, catalog)
    return document


def validate_versions(document: dict, catalog: dict) -> None:
    if document.get("format") != FORMAT or not isinstance(document.get("components"), dict):
        raise ctl.ReleaseError("unsupported component version ledger")
    definitions = {component["id"]: component for component in catalog["components"]}
    entries = document["components"]
    if entries.keys() != definitions.keys():
        missing = sorted(definitions.keys() - entries.keys())
        extra = sorted(entries.keys() - definitions.keys())
        raise ctl.ReleaseError(f"version ledger must cover every catalog component (missing={missing}, extra={extra})")
    for component_id, entry in entries.items():
        if not isinstance(entry, dict) or set(entry) != {"candidateVersion"}:
            raise ctl.ReleaseError(f"invalid version ledger entry for {component_id}")
        candidate = entry["candidateVersion"]
        if candidate is not None:
            if not versioned(definitions[component_id]) or not isinstance(candidate, str) or not STABLE_VERSION.fullmatch(candidate):
                raise ctl.ReleaseError(f"{component_id} needs a stable version or a digest-based release identity")
    host_versions = {entries[item]["candidateVersion"] for item in TAGGED_COMPONENTS
                     if item.startswith("ui-host-") and item in entries and entries[item]["candidateVersion"] is not None}
    if len(host_versions) > 1:
        raise ctl.ReleaseError("all UI Host candidates must use one host release version")
    github_tags: dict[tuple[str, str], str] = {}
    for component_id, definition in definitions.items():
        candidate = entries[component_id]["candidateVersion"]
        destination = definition.get("destination", "")
        if candidate and destination.startswith("GitHub Releases:") and not component_id.startswith("ui-host-"):
            key = (destination.split(";", 1)[0], candidate)
            earlier = github_tags.get(key)
            if earlier:
                raise ctl.ReleaseError(f"{earlier} and {component_id} would both claim {candidate} at {key[0]}")
            github_tags[key] = component_id


def snapshot(ledger: dict, catalog: dict, workspace: Path) -> dict:
    components = []
    for definition in catalog["components"]:
        component_id = definition["id"]
        source = ctl.component_version(definition, catalog, workspace)
        candidate = ledger["components"][component_id]["candidateVersion"]
        if source and candidate and tuple(map(int, candidate.split("."))) <= tuple(map(int, source.split("."))):
            # Equality is expected once a preparation has been applied.
            if candidate != source:
                raise ctl.ReleaseError(f"{component_id} candidate {candidate} is older than source {source}")
        components.append({"id": component_id, "owner": definition["owner"],
                           "identity": "source-version" if definition.get("version") else
                                       "release-tag" if versioned(definition) else "digest-or-deployment",
                           "sourceVersion": source, "candidateVersion": candidate,
                           "publishedVersion": None,
                           "publishedStatus": "unverified; import authenticated baseline"})
    return {"format": FORMAT, "components": components,
            "warning": "candidate versions are not published releases; source versions come from owning manifests"}


def set_candidate(path: Path, catalog: dict, component_id: str, version: str,
                  replace: bool = False, workspace: Path = ctl.WORKSPACE) -> dict:
    definitions = {component["id"]: component for component in catalog["components"]}
    targets = ("ui-host-web", "ui-host-android", "ui-host-ios") if component_id == "ui-host" else (component_id,)
    if any(item not in definitions or not versioned(definitions[item]) for item in targets):
        raise ctl.ReleaseError(f"{component_id} has no SemVer candidate; use an immutable artifact/deployment identity")
    if not STABLE_VERSION.fullmatch(version):
        raise ctl.ReleaseError("candidate version must be stable X.Y.Z")
    ledger = read_versions(path, catalog)
    for target in targets:
        source = ctl.component_version(definitions[target], catalog, workspace)
        if source and tuple(map(int, version.split("."))) < tuple(map(int, source.split("."))):
            raise ctl.ReleaseError(f"{target} candidate {version} is older than source {source}")
        existing = ledger["components"][target]["candidateVersion"]
        if existing and existing != version and not replace:
            raise ctl.ReleaseError(f"{target} already targets {existing}; use --replace after reviewing the intent")
    if any(target.startswith("ui-host-") for target in targets):
        for host_id in ("ui-host-web", "ui-host-android", "ui-host-ios"):
            host_candidate = ledger["components"][host_id]["candidateVersion"]
            if host_id not in targets and host_candidate not in (None, version):
                raise ctl.ReleaseError("all UI Host targets must share the same candidate version")
    for target in targets:
        ledger["components"][target]["candidateVersion"] = version
    validate_versions(ledger, catalog)
    # The file itself is the reviewed ledger. Replace atomically, without
    # touching package manifests, changelogs, or any published asset.
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent,
                                     prefix=".axiom-versions-", delete=False) as output:
        output.write(json.dumps(ledger, indent=2, ensure_ascii=False) + "\n")
        temporary = Path(output.name)
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)
    return ledger


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=ctl.CATALOG)
    parser.add_argument("--workspace", type=Path, default=ctl.WORKSPACE)
    parser.add_argument("--versions", type=Path, default=VERSIONS)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("show")
    setter = commands.add_parser("set")
    setter.add_argument("component")
    setter.add_argument("version")
    setter.add_argument("--replace", action="store_true")
    args = parser.parse_args()
    try:
        catalog = ctl.read_catalog(args.catalog)
        ledger = read_versions(args.versions, catalog)
        if args.command == "set":
            ledger = set_candidate(args.versions, catalog, args.component, args.version,
                                   args.replace, args.workspace.resolve())
        print(json.dumps(snapshot(ledger, catalog, args.workspace.resolve()), indent=2))
        return 0
    except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"release-versions: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
