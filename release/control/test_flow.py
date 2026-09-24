import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import ctl
import flow
import gate


def git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class ReleaseFlowTests(unittest.TestCase):
    def test_intent_requires_explicit_versions_and_unique_components(self):
        catalog = {"components": [
            {"id": "cli", "version": "cli/Cargo.toml"},
            {"id": "backend-api"},
            {"id": "ui-host-android"},
        ]}
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            path = Path(temporary) / "intent.json"
            intent = {"format": flow.INTENT_FORMAT, "trainId": "2026.09.23.1", "changes": [
                {"component": "cli", "type": "fix", "summary": "Fix commands."}
            ]}
            path.write_text(json.dumps(intent))
            with self.assertRaisesRegex(ctl.ReleaseError, "explicit release version"):
                flow.read_intent(path, catalog)
            intent["changes"][0]["version"] = "0.147.0"
            intent["changes"].append(dict(intent["changes"][0]))
            path.write_text(json.dumps(intent))
            with self.assertRaisesRegex(ctl.ReleaseError, "duplicate"):
                flow.read_intent(path, catalog)
            intent["changes"][1] = {"component": "backend-api", "type": "feature",
                                     "summary": "Add endpoint.", "version": "1.0.0"}
            path.write_text(json.dumps(intent))
            with self.assertRaisesRegex(ctl.ReleaseError, "deployment digest"):
                flow.read_intent(path, catalog)

    def test_version_editors_preserve_other_versions(self):
        cargo = '[package]\nname = "axiom-cli"\nversion = "0.146.0"\n\n[dependencies]\nfoo = "1"\n'
        changed = flow.set_toml_version(cargo, "package", "0.146.0", "0.147.0", "Cargo.toml")
        self.assertIn('version = "0.147.0"', changed)
        self.assertIn('foo = "1"', changed)
        poetry = '[tool.poetry]\nname = "axiom-fastapi"\nversion = "0.1.0"\n'
        self.assertIn('version = "0.2.0"', flow.set_toml_version(
            poetry, "tool.poetry", "0.1.0", "0.2.0", "pyproject.toml"))
        lock = '[[package]]\nname = "axiom-cli"\nversion = "0.146.0"\n\n[[package]]\nname = "foo"\nversion = "0.146.0"\n'
        updated = flow.set_cargo_lock_version(lock, "axiom-cli", "0.146.0", "0.147.0")
        self.assertIn('name = "axiom-cli"\nversion = "0.147.0"', updated)
        self.assertIn('name = "foo"\nversion = "0.146.0"', updated)

    def test_prepare_and_apply_are_scoped_and_backed_up(self):
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            workspace = Path(temporary).resolve()
            owner = workspace / "package-repo"
            owner.mkdir()
            git(owner, "init", "-q")
            git(owner, "config", "user.name", "Release Test")
            git(owner, "config", "user.email", "release-test@example.invalid")
            package = owner / "package.json"
            lock = owner / "package-lock.json"
            package.write_text('{"name":"example","version":"1.2.3"}\n')
            lock.write_text(json.dumps({"name": "example", "version": "1.2.3",
                                        "packages": {"": {"name": "example", "version": "1.2.3"}}}) + "\n")
            git(owner, "add", ".")
            git(owner, "commit", "-qm", "initial")
            catalog = {"repositories": {"package-repo": "package-repo"}, "components": [
                {"id": "example", "owner": "package-repo", "version": "package.json"}
            ]}
            intent = {"format": flow.INTENT_FORMAT, "trainId": "2026.09.23.1", "changes": [
                {"component": "example", "type": "fix", "summary": "Repair startup.", "version": "1.2.4"}
            ]}
            intent_path = workspace / "intent.json"
            intent_path.write_text(json.dumps(intent))
            parsed = flow.read_intent(intent_path, catalog)
            report, edits, fragments = flow.make_preparation(parsed, catalog, workspace)
            self.assertEqual(report["blocked"], [])
            self.assertEqual(json.loads(package.read_text())["version"], "1.2.3")
            build_root = workspace / "release-root"
            build_root.mkdir()
            with patch.dict("os.environ", {"CI": "true", "GITHUB_ACTIONS": "true",
                                         "AXIOM_RELEASE_BUILD_ROOT": str(build_root)}):
                backup = flow.apply_preparation(report, edits, fragments, catalog, workspace)
            self.assertEqual(json.loads(package.read_text())["version"], "1.2.4")
            self.assertEqual(json.loads(lock.read_text())["packages"][""]["version"], "1.2.4")
            self.assertEqual(json.loads((backup / owner.name / "package.json").read_text())["version"], "1.2.3")
            self.assertEqual(json.loads(next(iter(fragments)).read_text())["component"], "example")
            with self.assertRaisesRegex(ctl.ReleaseError, "must exceed"):
                flow.make_preparation(parsed, catalog, workspace)

    def test_prepare_does_not_edit_dirty_version_file(self):
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            workspace = Path(temporary).resolve()
            owner = workspace / "package-repo"
            owner.mkdir()
            git(owner, "init", "-q")
            git(owner, "config", "user.name", "Release Test")
            git(owner, "config", "user.email", "release-test@example.invalid")
            package = owner / "package.json"
            package.write_text('{"name":"example","version":"1.2.3"}\n')
            git(owner, "add", ".")
            git(owner, "commit", "-qm", "initial")
            package.write_text('{"name":"example","version":"1.2.3","user":"work"}\n')
            catalog = {"repositories": {"package-repo": "package-repo"}, "components": [
                {"id": "example", "owner": "package-repo", "version": "package.json"}
            ]}
            intent = {"trainId": "2026.09.23.1", "changes": [
                {"component": "example", "type": "fix", "summary": "Fix it.", "version": "1.2.4"}
            ]}
            report, edits, fragments = flow.make_preparation(intent, catalog, workspace)
            self.assertTrue(report["blocked"])
            build_root = workspace / "release-root"
            build_root.mkdir()
            with patch.dict("os.environ", {"CI": "true", "GITHUB_ACTIONS": "true",
                                         "AXIOM_RELEASE_BUILD_ROOT": str(build_root)}):
                with self.assertRaisesRegex(ctl.ReleaseError, "dirty/untracked"):
                    flow.apply_preparation(report, edits, fragments, catalog, workspace)
            self.assertIn('"user":"work"', package.read_text())
            self.assertFalse((owner / "release-notes").exists())

    def test_gate_requires_committed_notes_and_verified_staged_bytes(self):
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            workspace = Path(temporary).resolve()
            owner = workspace / "service"
            owner.mkdir()
            git(owner, "init", "-q")
            git(owner, "config", "user.name", "Release Test")
            git(owner, "config", "user.email", "release-test@example.invalid")
            (owner / "src").mkdir()
            (owner / "src" / "api.txt").write_text("api\n")
            git(owner, "add", ".")
            git(owner, "commit", "-qm", "initial")
            catalog = {"sha256": "test", "repositories": {"service": "service"}, "components": [
                {"id": "backend-api", "owner": "service", "kind": "service", "adapter": "ci-only",
                 "destination": "test", "sources": [{"repo": "service", "paths": ["src/"]}]}
            ]}
            intent = {"format": flow.INTENT_FORMAT, "trainId": "2026.09.23.1", "changes": [
                {"component": "backend-api", "type": "fix", "summary": "Repair endpoint."}
            ]}
            plan = ctl.make_plan(catalog, workspace)
            before = gate.inspect_candidate(intent, plan, catalog, workspace)
            self.assertEqual(before["phase"], "blocked")
            self.assertTrue(any("release notes" in reason for reason in before["blockers"]))
            folder = owner / "release-notes" / "unreleased"
            folder.mkdir(parents=True)
            (folder / "2026.09.23.1-backend-api.json").write_text(json.dumps(intent["changes"][0]))
            git(owner, "add", ".")
            git(owner, "commit", "-qm", "release note")
            plan = ctl.make_plan(catalog, workspace)
            ready = gate.inspect_candidate(intent, plan, catalog, workspace)
            self.assertEqual(ready["phase"], "ready-to-build")
            root = workspace / "release-root"
            root.mkdir()
            artifact = root / "api-image.txt"
            artifact.write_bytes(b"immutable image digest")
            receipt_path = root / "receipt.json"
            stage_path = root / "stage.json"
            with patch.dict("os.environ", {"CI": "true", "GITHUB_ACTIONS": "true",
                                         "AXIOM_RELEASE_BUILD_ROOT": str(root)}):
                ctl.record_artifact(plan, catalog, workspace, "backend-api", [artifact], receipt_path)
                staged = ctl.stage_manifest(plan, catalog, workspace, intent["trainId"],
                                            [receipt_path], stage_path, intent)
            candidate = gate.inspect_candidate(intent, plan, catalog, workspace, staged, [receipt_path])
            self.assertEqual(candidate["phase"], "candidate-ready-for-publisher")
            self.assertEqual(candidate["publicationStatus"], "not-published")
            artifact.write_bytes(b"tampered")
            tampered = gate.inspect_candidate(intent, plan, catalog, workspace, staged, [receipt_path])
            self.assertEqual(tampered["phase"], "blocked")
            self.assertTrue(any("checksum mismatch" in reason for reason in tampered["blockers"]))


if __name__ == "__main__":
    unittest.main()
