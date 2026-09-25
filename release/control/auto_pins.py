"""Pin SDK consumers to remotely verified producer bytes during a release run."""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import tempfile

import ctl
import cycle
import publish_targets


def _ordinary(path: Path) -> bytes:
    if not path.is_file() or path.is_symlink():
        raise ctl.ReleaseError(f"dependency pin source is missing or linked: {path}")
    return path.read_bytes()


def verified_upstream(root: Path, catalog: dict, train_id: str,
                      component: str, version: str) -> dict:
    proof = cycle.published_evidence(root, catalog, train_id).get(component)
    if not proof or proof.get("version") != version:
        raise ctl.ReleaseError(f"{component}@{version} must be remotely verified in this run before its SDK pin is updated")
    record_path = Path(proof["record"])
    if root.resolve() not in record_path.resolve().parents or record_path.is_symlink():
        raise ctl.ReleaseError("published dependency evidence is outside the release volume")
    return json.loads(record_path.read_text())


def _backup(root: Path, train_id: str, repository: str, path: Path, owner: Path) -> None:
    target = root / "trains" / train_id / "auto-pins" / repository / path.relative_to(owner)
    target.parent.mkdir(parents=True, exist_ok=True)
    original = _ordinary(path)
    if target.exists():
        if target.is_symlink() or not target.is_file():
            raise ctl.ReleaseError(f"dependency pin backup is invalid: {target}")
        return
    with target.open("xb") as output:
        output.write(original)


def pin_verified_consumers(root: Path, workspace: Path, catalog: dict,
                           train_id: str, ledger: dict, consumers: set[str]) -> dict[str, list[str]]:
    """Edit only declared pin files; caller commits/pushes them before a new plan.

    This function is resumable: a completed pin is left unchanged. It never
    manufactures a checksum or npm integrity value from an unverified build.
    """
    changed: dict[str, list[str]] = {}
    pin_work = root / "trains" / train_id / "auto-pins"
    pin_work.mkdir(parents=True, exist_ok=True)
    if "sdk-swift" in consumers:
        version = ledger["components"]["runtime-apple"]["candidateVersion"]
        record = verified_upstream(root, catalog, train_id, "runtime-apple", version)
        checksum = record.get("details", {}).get("files", {}).get("AxiomRuntime.xcframework.zip")
        if not isinstance(checksum, str) or not re.fullmatch(r"[a-f0-9]{64}", checksum):
            raise ctl.ReleaseError("published Apple runtime has no verified XCFramework checksum")
        owner = ctl.repo_path(catalog, "axiom-sdk", workspace)
        path = owner / "swift/Package.swift"
        before = _ordinary(path).decode()
        updated, urls = re.subn(r"releases/download/v[0-9]+\.[0-9]+\.[0-9]+/AxiomRuntime\.xcframework\.zip",
                                f"releases/download/v{version}/AxiomRuntime.xcframework.zip", before)
        updated, checksums = re.subn(r'(?m)^(\s*checksum:\s*")[a-f0-9]{64}("\s*)$',
                                     lambda match: match.group(1) + checksum + match.group(2), updated)
        if urls != 1 or checksums != 1:
            raise ctl.ReleaseError("Swift Package.swift has an unrecognized runtime pin; inspect it before release")
        if updated != before:
            _backup(root, train_id, "axiom-sdk", path, owner)
            path.write_text(updated)
            changed.setdefault("axiom-sdk", []).append("swift/Package.swift")
    if "sdk-atmx-react" in consumers:
        version = ledger["components"]["sdk-atmx-web"]["candidateVersion"]
        record = verified_upstream(root, catalog, train_id, "sdk-atmx-web", version)
        integrity = record.get("details", {}).get("integrity")
        if not isinstance(integrity, str) or not integrity.startswith("sha512-"):
            raise ctl.ReleaseError("published atmx-web has no verified npm integrity")
        owner = ctl.repo_path(catalog, "atmx-react", workspace)
        manifest = owner / "package.json"
        lock = owner / "package-lock.json"
        before_manifest = _ordinary(manifest)
        before_lock = _ordinary(lock)
        environment = ctl.build_environment(root, "sdk-atmx-react")
        environment["npm_config_cache"] = str(pin_work / "npm-cache")
        metadata = publish_targets._npm_metadata("atmx-web", version, pin_work, environment)
        if not metadata or metadata.get("dist", {}).get("integrity") != integrity:
            raise ctl.ReleaseError("npm atmx-web integrity differs from the remotely verified publication")
        package = json.loads(before_manifest)
        locked = json.loads(before_lock)
        current = package.get("dependencies", {}).get("atmx-web")
        resolved = locked.get("packages", {}).get("node_modules/atmx-web", {})
        if current != f"^{version}" or resolved.get("version") != version or resolved.get("integrity") != integrity:
            # Resolve in a disposable copy on the external SSD. A failed npm
            # invocation must never leave the owning Git checkout half-edited.
            with tempfile.TemporaryDirectory(prefix="react-lock-", dir=pin_work) as temporary:
                staged = Path(temporary)
                (staged / "package.json").write_bytes(before_manifest)
                (staged / "package-lock.json").write_bytes(before_lock)
                command = ["npm", "install", f"atmx-web@{version}", "--package-lock-only",
                           "--ignore-scripts", "--no-audit", "--no-fund", "--save-prefix=^"]
                result = subprocess.run(command, cwd=staged, env=environment, check=False,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                if result.returncode:
                    raise ctl.ReleaseError("npm could not regenerate React's dependency lockfile: "
                                           + result.stdout.decode(errors="replace")[-2000:])
                generated_manifest = _ordinary(staged / "package.json")
                generated_lock = _ordinary(staged / "package-lock.json")
            package = json.loads(generated_manifest)
            locked = json.loads(generated_lock)
            root_pin = locked.get("packages", {}).get("", {}).get("dependencies", {}).get("atmx-web")
            resolved = locked.get("packages", {}).get("node_modules/atmx-web", {})
            if (package.get("dependencies", {}).get("atmx-web") != f"^{version}"
                    or root_pin != f"^{version}" or resolved.get("version") != version
                    or resolved.get("integrity") != integrity):
                raise ctl.ReleaseError("regenerated React lockfile does not pin the exact published atmx-web bytes")
            _backup(root, train_id, "atmx-react", manifest, owner)
            _backup(root, train_id, "atmx-react", lock, owner)
            manifest.write_bytes(generated_manifest)
            lock.write_bytes(generated_lock)
            if _ordinary(manifest) != before_manifest:
                changed.setdefault("atmx-react", []).append("package.json")
            if _ordinary(lock) != before_lock:
                changed.setdefault("atmx-react", []).append("package-lock.json")
    return changed
