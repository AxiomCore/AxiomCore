import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import ctl
import flow
import gate
import scan
import train
import versions


def git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class ReleaseFlowTests(unittest.TestCase):
    def test_train_status_uses_fixed_evidence_folder_without_mounted_ssd(self):
        catalog = {"components": [{"id": "cli", "owner": "owner", "version": "Cargo.toml"}]}
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            workspace = Path(temporary)
            owner = workspace / "owner"
            owner.mkdir()
            git(owner, "init", "-q")
            (owner / "Cargo.toml").write_text('[package]\nname = "cli"\nversion = "1.2.3"\n')
            catalog["repositories"] = {"owner": "owner"}
            intent_path = workspace / "intent.json"
            intent_path.write_text(json.dumps({"format": flow.INTENT_FORMAT, "trainId": "test.1",
                                               "changes": [{"component": "cli", "type": "fix",
                                                            "summary": "Fix it."}]}))
            versions_path = workspace / "versions.json"
            versions_path.write_text(json.dumps({"format": versions.FORMAT,
                                                 "components": {"cli": {"candidateVersion": "1.2.4"}}}))
            report = train.status(catalog, workspace, workspace / "absent-volume",
                                  intent_path, versions_path)
            self.assertFalse(report["storageAvailable"])
            self.assertEqual(report["evidenceRoot"], str(workspace / "absent-volume/trains/test.1"))
            self.assertEqual(report["publicationStatus"], "not-proven-published")

    def test_checked_in_intent_uses_one_candidate_version_ledger(self):
        catalog = ctl.read_catalog()
        ledger = versions.read_versions(versions.VERSIONS, catalog)
        self.assertEqual(set(ledger["components"]),
                         {component["id"] for component in catalog["components"]})
        intent = flow.read_intent(ctl.CONTROL_DIR / "intent.json", catalog, ledger)
        changes = {change["component"]: change for change in intent["changes"]}
        queued = {change["component"]: change for change in intent["queued"]}
        self.assertEqual(changes["runtime-apple"]["version"], "0.148.0")
        self.assertEqual(queued["cli"]["version"], "0.147.0")
        self.assertFalse(set(changes) & set(queued))

    def test_local_changed_inputs_are_accounted_for_in_active_or_queued_intent(self):
        catalog = ctl.read_catalog()
        ledger = versions.read_versions(versions.VERSIONS, catalog)
        intent = flow.read_intent(ctl.CONTROL_DIR / "intent.json", catalog, ledger)
        if any(not (ctl.WORKSPACE / path / ".git").exists()
               for path in catalog["repositories"].values()):
            self.skipTest("the isolated CI checkout does not include sibling source repositories")
        inventory = scan.inventory(catalog, ctl.WORKSPACE)
        accounted = {change["component"] for change in [*intent["changes"], *intent["queued"]]}
        self.assertEqual(set(inventory["affected"]), accounted)

    def test_ledger_rejects_missing_components_and_version_on_digest_component(self):
        catalog = {"components": [{"id": "cli", "version": "Cargo.toml"},
                                  {"id": "docs"}]}
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            path = Path(temporary) / "versions.json"
            path.write_text(json.dumps({"format": versions.FORMAT,
                                        "components": {"cli": {"candidateVersion": "1.0.0"}}}))
            with self.assertRaisesRegex(ctl.ReleaseError, "cover every catalog component"):
                versions.read_versions(path, catalog)
            path.write_text(json.dumps({"format": versions.FORMAT,
                                        "components": {"cli": {"candidateVersion": "1.0.0"},
                                                       "docs": {"candidateVersion": "1.0.0"}}}))
            with self.assertRaisesRegex(ctl.ReleaseError, "digest-based"):
                versions.read_versions(path, catalog)

    def test_ui_host_group_version_updates_all_targets_atomically(self):
        ids = ("ui-host-web", "ui-host-android", "ui-host-ios")
        catalog = {"components": [{"id": item} for item in ids]}
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            path = Path(temporary) / "versions.json"
            path.write_text(json.dumps({"format": versions.FORMAT,
                                        "components": {item: {"candidateVersion": None} for item in ids}}))
            updated = versions.set_candidate(path, catalog, "ui-host", "0.6.7")
            self.assertEqual({updated["components"][item]["candidateVersion"] for item in ids}, {"0.6.7"})
            self.assertEqual(versions.read_versions(path, catalog)["components"], updated["components"])

    def test_distinct_components_cannot_claim_one_github_release_tag(self):
        catalog = {"components": [
            {"id": "cli", "version": "Cargo.toml", "destination": "GitHub Releases: AxiomCore/AxiomCore"},
            {"id": "runtime-apple", "destination": "GitHub Releases: AxiomCore/AxiomCore"}]}
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            path = Path(temporary) / "versions.json"
            path.write_text(json.dumps({"format": versions.FORMAT,
                                        "components": {"cli": {"candidateVersion": "0.147.0"},
                                                       "runtime-apple": {"candidateVersion": "0.147.0"}}}))
            with self.assertRaisesRegex(ctl.ReleaseError, "would both claim"):
                versions.read_versions(path, catalog)
            path.write_text(json.dumps({"format": versions.FORMAT,
                                        "components": {"cli": {"candidateVersion": "0.147.0"},
                                                       "runtime-apple": {"candidateVersion": None}}}))
            before = path.read_bytes()
            with self.assertRaisesRegex(ctl.ReleaseError, "would both claim"):
                versions.set_candidate(path, catalog, "runtime-apple", "0.147.0")
            self.assertEqual(path.read_bytes(), before)

    def test_ledger_intent_rejects_conflicting_inline_version(self):
        catalog = {"components": [{"id": "cli", "version": "Cargo.toml"}]}
        ledger = {"components": {"cli": {"candidateVersion": "1.2.4"}}}
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            path = Path(temporary) / "intent.json"
            path.write_text(json.dumps({"format": flow.INTENT_FORMAT, "trainId": "test.1",
                                        "changes": [{"component": "cli", "type": "fix",
                                                     "summary": "Fix it.", "version": "1.2.5"}]}))
            with self.assertRaisesRegex(ctl.ReleaseError, "differs from versions.json"):
                flow.read_intent(path, catalog, ledger)

    def test_queued_change_is_validated_and_not_prepared_in_active_wave(self):
        catalog = {"components": [{"id": "cli", "version": "Cargo.toml"},
                                  {"id": "backend-api"}]}
        ledger = {"components": {"cli": {"candidateVersion": "1.2.4"},
                                  "backend-api": {"candidateVersion": None}}}
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            path = Path(temporary) / "intent.json"
            document = {"format": flow.INTENT_FORMAT, "trainId": "test.1",
                        "changes": [{"component": "backend-api", "type": "fix", "summary": "Fix API."}],
                        "queued": [{"component": "cli", "type": "feature", "summary": "Improve CLI."}]}
            path.write_text(json.dumps(document))
            intent = flow.read_intent(path, catalog, ledger)
            self.assertEqual(intent["queued"][0]["version"], "1.2.4")
            document["queued"][0]["component"] = "backend-api"
            path.write_text(json.dumps(document))
            with self.assertRaisesRegex(ctl.ReleaseError, "duplicate"):
                flow.read_intent(path, catalog, ledger)

    def test_prepared_cli_candidate_with_matching_lock_and_fragment_is_idempotent(self):
        with tempfile.TemporaryDirectory(prefix="axiom-flow-test-") as temporary:
            workspace = Path(temporary)
            owner = workspace / "AxiomCore"
            (owner / "cli").mkdir(parents=True)
            git(owner, "init", "-q")
            (owner / "cli/Cargo.toml").write_text('[package]\nname = "axiom-cli"\nversion = "0.147.0"\n')
            (owner / "cli/Cargo.lock").write_text('[[package]]\nname = "axiom-cli"\nversion = "0.147.0"\n')
            note = owner / "release-notes/unreleased/test.1-cli.json"
            note.parent.mkdir(parents=True)
            change = {"component": "cli", "type": "feature", "summary": "Improve CLI."}
            note.write_text(json.dumps(change))
            catalog = {"repositories": {"AxiomCore": "AxiomCore"}, "components": [
                {"id": "cli", "owner": "AxiomCore", "version": "cli/Cargo.toml"}]}
            report, edits, fragments = flow.make_preparation(
                {"trainId": "test.1", "changes": [{**change, "version": "0.147.0"}]}, catalog, workspace)
            self.assertEqual(edits, {})
            self.assertEqual(fragments, {})
            self.assertEqual(report["existingFragments"], [str(note.resolve())])
            (owner / "cli/Cargo.lock").write_text('[[package]]\nname = "axiom-cli"\nversion = "0.146.0"\n')
            with self.assertRaisesRegex(ctl.ReleaseError, "Cargo.lock does not match"):
                flow.make_preparation(
                    {"trainId": "test.1", "changes": [{**change, "version": "0.147.0"}]},
                    catalog, workspace)

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
            second, second_edits, second_fragments = flow.make_preparation(parsed, catalog, workspace)
            self.assertEqual(second_edits, {})
            self.assertEqual(second_fragments, {})
            self.assertEqual(len(second["existingFragments"]), 1)

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
