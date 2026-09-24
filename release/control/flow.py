#!/usr/bin/env python3
"""Prepare reviewed platform release changes without publishing anything.

An intent is explicit about the selected components and their versions. The
default is a preview. Applying it touches only clean, tracked version files
and new release-note fragments, after saving byte-for-byte backups on the
external release volume. It never commits, tags, deploys, or publishes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
import tomllib

import ctl
import versions


INTENT_FORMAT = "axiom-platform-release-intent/v1"
PREPARATION_FORMAT = "axiom-platform-release-preparation/v1"
STABLE_VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
TRAIN_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{2,79}")
TAGGED_COMPONENTS = {"runtime-apple", "ui-host-web", "ui-host-android", "ui-host-ios",
                     "sdk-swift", "extractor-go"}
CHANGE_TYPES = {"feature", "fix", "security", "breaking", "internal"}


def stable_tuple(value: str) -> tuple[int, int, int]:
    if not STABLE_VERSION.fullmatch(value):
        raise ctl.ReleaseError(f"release version must be stable X.Y.Z: {value!r}")
    return tuple(int(part) for part in value.split("."))


def read_intent(path: Path, catalog: dict, ledger: dict | None = None) -> dict:
    intent = json.loads(path.read_text())
    if intent.get("format") != INTENT_FORMAT or not TRAIN_ID.fullmatch(str(intent.get("trainId", ""))):
        raise ctl.ReleaseError("release intent needs the supported format and a safe trainId")
    changes = intent.get("changes")
    if not isinstance(changes, list) or not changes:
        raise ctl.ReleaseError("release intent needs at least one change")
    definitions = {component["id"]: component for component in catalog["components"]}
    seen = set()
    host_versions = set()
    for change in changes:
        if not isinstance(change, dict) or change.get("component") not in definitions:
            raise ctl.ReleaseError(f"unknown component in release intent: {change}")
        component_id = change["component"]
        if component_id in seen:
            raise ctl.ReleaseError(f"duplicate release intent for {component_id}")
        seen.add(component_id)
        if change.get("type") not in CHANGE_TYPES:
            raise ctl.ReleaseError(f"invalid change type for {component_id}")
        if not isinstance(change.get("summary"), str) or not change["summary"].strip():
            raise ctl.ReleaseError(f"release intent summary is missing for {component_id}")
        if "\n" in change["summary"] or "\r" in change["summary"]:
            raise ctl.ReleaseError(f"release intent summary must be one line for {component_id}")
        if change["type"] == "breaking" and not str(change.get("migration", "")).strip():
            raise ctl.ReleaseError(f"breaking change needs migration guidance for {component_id}")
        definition = definitions[component_id]
        version = change.get("version")
        if ledger is not None:
            candidate = ledger["components"][component_id]["candidateVersion"]
            if version is not None and version != candidate:
                raise ctl.ReleaseError(f"{component_id} intent version differs from versions.json")
            if candidate is not None:
                change["version"] = candidate
                version = candidate
        if definition.get("version") or component_id in TAGGED_COMPONENTS:
            if not isinstance(version, str):
                raise ctl.ReleaseError(f"explicit release version is required for {component_id}")
            stable_tuple(version)
        elif version is not None:
            raise ctl.ReleaseError(f"{component_id} is identified by a deployment digest, not a SemVer field")
        if "runtimeVersion" in change:
            if component_id != "sdk-flutter":
                raise ctl.ReleaseError("runtimeVersion is only valid for sdk-flutter")
            if not isinstance(change["runtimeVersion"], str):
                raise ctl.ReleaseError("runtimeVersion must be a stable X.Y.Z string")
            stable_tuple(change["runtimeVersion"])
        if component_id.startswith("ui-host-"):
            host_versions.add(version)
    if len(host_versions) > 1:
        raise ctl.ReleaseError("all UI Host targets in one train must share one host version")
    return intent


def unique_substitution(source: str, pattern: str, replacement: str, context: str) -> str:
    result, count = re.subn(pattern, replacement, source, count=0, flags=re.MULTILINE)
    if count != 1:
        raise ctl.ReleaseError(f"expected one {context} version field, found {count}")
    return result


def set_toml_version(source: str, section: str, expected: str, version: str, context: str) -> str:
    parsed = tomllib.loads(source)
    value = parsed
    for key in section.split("."):
        value = value[key]
    if str(value["version"]) != expected:
        raise ctl.ReleaseError(f"{context} changed while preparing release")
    header = f"[{section}]"
    start = source.find(header)
    if start < 0 or (start and source[start - 1] != "\n"):
        raise ctl.ReleaseError(f"missing {header} in {context}")
    end_match = re.search(r"^\[", source[start + len(header):], re.MULTILINE)
    end = start + len(header) + end_match.start() if end_match else len(source)
    body = source[start:end]
    body = unique_substitution(body, r'^(version\s*=\s*")[^"]+("[^\n]*)$',
                               rf'\g<1>{version}\g<2>', context)
    return source[:start] + body + source[end:]


def set_cargo_lock_version(source: str, package: str, expected: str, version: str) -> str:
    pattern = re.compile(r"(?ms)^\[\[package\]\]\n(?P<body>.*?)(?=^\[\[package\]\]|\Z)")
    blocks = list(pattern.finditer(source))
    matches = [match for match in blocks if re.search(rf'^name = "{re.escape(package)}"$',
                                                    match.group("body"), re.MULTILINE)]
    if len(matches) != 1:
        raise ctl.ReleaseError(f"expected one {package} Cargo.lock package, found {len(matches)}")
    match = matches[0]
    body = match.group("body")
    body = unique_substitution(body, rf'^(version = "){re.escape(expected)}("[\s]*)$',
                               rf'\g<1>{version}\g<2>', f"{package} Cargo.lock")
    return source[:match.start("body")] + body + source[match.end("body"):]


def set_package_version(source: str, expected: str, version: str, lock: bool = False) -> str:
    document = json.loads(source)
    if document.get("version") != expected:
        raise ctl.ReleaseError("package version changed while preparing release")
    document["version"] = version
    if lock:
        root = document.get("packages", {}).get("")
        if not isinstance(root, dict) or root.get("version") != expected:
            raise ctl.ReleaseError("npm lockfile root version does not match package.json")
        root["version"] = version
    return json.dumps(document, indent=2, ensure_ascii=False) + "\n"


def make_preparation(intent: dict, catalog: dict, workspace: Path) -> tuple[dict, dict[Path, str], dict[Path, str]]:
    definitions = {component["id"]: component for component in catalog["components"]}
    edits: dict[Path, str] = {}
    originals: dict[Path, str] = {}
    fragments: dict[Path, str] = {}
    blockers: list[str] = []

    def edit(path: Path, transform) -> None:
        if path not in originals:
            if not path.is_file() or path.is_symlink():
                raise ctl.ReleaseError(f"version companion is absent or symlinked: {path}")
            originals[path] = path.read_bytes().decode("utf-8")
        edits[path] = transform(edits.get(path, originals[path]))

    for change in intent["changes"]:
        component_id = change["component"]
        definition = definitions[component_id]
        owner = ctl.repo_path(catalog, definition["owner"], workspace)
        version = change.get("version")
        if definition.get("version"):
            previous = ctl.component_version(definition, catalog, workspace)
            if stable_tuple(version) <= stable_tuple(previous):
                raise ctl.ReleaseError(f"{component_id} version {version} must exceed {previous}")
            version_path = owner / definition["version"]
            if version_path.suffix == ".json":
                edit(version_path, lambda source, old=previous, new=version:
                     set_package_version(source, old, new))
                lock_path = version_path.with_name("package-lock.json")
                if lock_path.is_file():
                    edit(lock_path, lambda source, old=previous, new=version:
                         set_package_version(source, old, new, lock=True))
            elif version_path.suffix in (".yaml", ".yml"):
                edit(version_path, lambda source, old=previous, new=version:
                     unique_substitution(source, rf'^version:\s*{re.escape(old)}(\s*(?:#.*)?)$',
                                         rf'version: {new}\g<1>', str(version_path)))
            elif component_id == "cli":
                edit(version_path, lambda source, old=previous, new=version:
                     set_toml_version(source, "package", old, new, str(version_path)))
                edit(version_path.with_name("Cargo.lock"), lambda source, old=previous, new=version:
                     set_cargo_lock_version(source, "axiom-cli", old, new))
            elif component_id == "extractor-fastapi":
                edit(version_path, lambda source, old=previous, new=version:
                     set_toml_version(source, "tool.poetry", old, new, str(version_path)))
            else:
                raise ctl.ReleaseError(f"no reviewed version editor exists for {component_id}")

            if component_id == "sdk-atmx-web":
                index = owner / "src/index.ts"
                edit(index, lambda source, old=previous, new=version:
                     unique_substitution(source, rf'^(export const ATMX_VERSION = "){re.escape(old)}(";)$',
                                         rf'\g<1>{new}\g<2>', str(index)))
            if component_id in {"sdk-flutter", "sdk-flutter-generator"}:
                util = workspace / "axiom-build/src/core/utils.rs"
                name = "AXIOM_FLUTTER_VERSION" if component_id == "sdk-flutter" else "AXIOM_FLUTTER_GENERATOR_VERSION"
                edit(util, lambda source, old=previous, new=version, constant=name:
                     unique_substitution(source, rf'^(const {constant}: &str = "\^){re.escape(old)}(";)$',
                                         rf'\g<1>{new}\g<2>', str(util)))
            if component_id == "sdk-flutter":
                for platform_name in ("ios", "macos"):
                    podspec = owner / f"flutter/axiom_flutter/{platform_name}/axiom_flutter.podspec"
                    edit(podspec, lambda source, old=previous, new=version:
                         unique_substitution(source, rf"^(\s*s\.version\s*=\s*'){re.escape(old)}('.*)$",
                                             rf'\g<1>{new}\g<2>', str(podspec)))
                    if change.get("runtimeVersion"):
                        runtime_version = change["runtimeVersion"]
                        edit(podspec, lambda source, new=runtime_version:
                             unique_substitution(source, r"^(\s*runtime_version\s*=\s*')[^']+('.*)$",
                                                 rf'\g<1>{new}\g<2>', str(podspec)))

        fragment_path = owner / "release-notes" / "unreleased" / f"{intent['trainId']}-{component_id}.json"
        if not fragment_path.parent.resolve().is_relative_to(owner):
            raise ctl.ReleaseError(f"release-note path escapes its owning repository: {fragment_path}")
        if fragment_path.exists():
            raise ctl.ReleaseError(f"refusing to replace an existing release-note fragment: {fragment_path}")
        fragment = {key: change[key] for key in ("component", "type", "summary", "migration") if key in change}
        fragments[fragment_path] = json.dumps(fragment, indent=2, ensure_ascii=False) + "\n"

    for path, changed in edits.items():
        if changed == originals[path]:
            raise ctl.ReleaseError(f"version editor made no change: {path}")
        repo = containing_repo(catalog, workspace, path)
        relative = str(path.relative_to(repo))
        if ctl.run("git", "status", "--porcelain", "--", relative, cwd=repo).strip():
            blockers.append(f"dirty version file: {path}")
        if not ctl.run("git", "ls-files", "--", relative, cwd=repo).strip():
            blockers.append(f"untracked version file: {path}")

    report = {"format": PREPARATION_FORMAT, "trainId": intent["trainId"],
              "intentSha256": ctl.sha256(ctl.canonical(intent)),
              "versions": [{"component": item["component"], "version": item.get("version")}
                           for item in intent["changes"]],
              "files": [{"path": str(path), "oldSha256": hashlib.sha256(originals[path].encode()).hexdigest(),
                         "newSha256": hashlib.sha256(changed.encode()).hexdigest()}
                        for path, changed in sorted(edits.items())],
              "fragments": [str(path) for path in sorted(fragments)], "blocked": blockers,
              "next": "review, apply, test, commit changed owner repositories, then make a strict scoped plan"}
    return report, edits, fragments


def containing_repo(catalog: dict, workspace: Path, path: Path) -> Path:
    path = path.resolve()
    owners = []
    for name in catalog["repositories"]:
        try:
            owners.append(ctl.repo_path(catalog, name, workspace))
        except ctl.ReleaseError:
            continue
    matches = [owner for owner in owners if path == owner or owner in path.parents]
    if not matches:
        raise ctl.ReleaseError(f"version edit is outside a declared repository: {path}")
    return max(matches, key=lambda owner: len(owner.parts))


def apply_preparation(report: dict, edits: dict[Path, str], fragments: dict[Path, str],
                      catalog: dict, workspace: Path) -> Path:
    if report["blocked"]:
        raise ctl.ReleaseError("release preparation has dirty/untracked target files: " + "; ".join(report["blocked"]))
    root = ctl.require_external_build_root()
    backup = root / "backups" / report["trainId"]
    if not backup.resolve().is_relative_to(root):
        raise ctl.ReleaseError(f"release backup path escapes the external build root: {backup}")
    if backup.exists():
        raise ctl.ReleaseError(f"release backup already exists; use a new train ID: {backup}")
    for path, changed in edits.items():
        expected = next(item["oldSha256"] for item in report["files"] if item["path"] == str(path))
        if ctl.sha256_file(path) != expected or path.is_symlink():
            raise ctl.ReleaseError(f"version file changed after preview: {path}")
    for path in fragments:
        if path.exists():
            raise ctl.ReleaseError(f"release-note path appeared after preview: {path}")
    backup.mkdir(parents=True)
    for path in edits:
        owner = containing_repo(catalog, workspace, path)
        destination = backup / owner.name / path.relative_to(owner)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as out:
            out.write(path.read_bytes())
    ctl.write_json(backup / "preparation.json", report)
    for path, changed in edits.items():
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent,
                                         prefix=".axiom-release-", delete=False) as output:
            output.write(changed)
            temporary = Path(output.name)
        os.chmod(temporary, stat.S_IMODE(path.stat().st_mode))
        os.replace(temporary, path)
    for path, content in fragments.items():
        ctl.write_new_text(path, content)
    return backup


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=ctl.CATALOG)
    parser.add_argument("--workspace", type=Path, default=ctl.WORKSPACE)
    parser.add_argument("--intent", type=Path, default=ctl.CONTROL_DIR / "intent.json")
    parser.add_argument("--versions", type=Path, default=versions.VERSIONS)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--apply", action="store_true", help="apply reviewed version/fragment edits with external backups")
    args = parser.parse_args()
    try:
        catalog = ctl.read_catalog(args.catalog)
        workspace = args.workspace.resolve()
        ledger = versions.read_versions(args.versions, catalog)
        intent = read_intent(args.intent, catalog, ledger)
        report, edits, fragments = make_preparation(intent, catalog, workspace)
        if args.apply:
            root = ctl.require_external_build_root()
            if not args.out:
                args.out = root / "trains" / intent["trainId"] / "preparation.json"
            if root not in args.out.resolve().parents or args.out.exists():
                raise ctl.ReleaseError("preparation report must be a new path under the release build root")
            backup = apply_preparation(report, edits, fragments, catalog, workspace)
            report["applied"] = True
            report["backup"] = str(backup)
        else:
            report["applied"] = False
        if args.out:
            ctl.write_json(args.out, report)
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0 if not report["blocked"] else 2
    except (ctl.ReleaseError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"release-flow: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
