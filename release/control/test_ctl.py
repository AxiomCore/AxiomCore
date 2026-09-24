import json
import plistlib
from unittest.mock import patch
from pathlib import Path
import subprocess
import tempfile
import unittest

import ctl


def git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class ReleaseControlTests(unittest.TestCase):
    def test_release_evidence_writes_do_not_replace_existing_files(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            evidence = Path(temporary) / "plans" / "candidate.json"
            ctl.write_json(evidence, {"first": True})
            with self.assertRaisesRegex(ctl.ReleaseError, "refusing to overwrite"):
                ctl.write_json(evidence, {"second": True})
            self.assertEqual(json.loads(evidence.read_text()), {"first": True})

    def test_default_mount_requires_exact_image_and_mount_point(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            image = Path(temporary) / "AxiomReleaseBuild.sparsebundle"
            mount = Path(temporary) / "AxiomReleaseBuild"
            images = {"images": [{"image-path": str(image), "system-entities": [
                {"mount-point": str(mount)}]}]}
            with patch.object(ctl, "DEFAULT_BUILD_IMAGE", image), \
                 patch.object(ctl, "DEFAULT_BUILD_MOUNT", mount), \
                 patch.object(ctl, "run", return_value=plistlib.dumps(images)):
                self.assertTrue(ctl.default_image_is_mounted())
                images["images"][0]["system-entities"][0]["mount-point"] = str(mount) + "-other"
                with patch.object(ctl, "run", return_value=plistlib.dumps(images)):
                    self.assertFalse(ctl.default_image_is_mounted())
                    self.assertEqual(ctl.default_image_mount_points(), [str(mount) + "-other"])

    def test_catalog_is_acyclic(self):
        catalog = ctl.read_catalog()
        self.assertEqual(len(ctl.topological_components(catalog["components"])),
                         len(catalog["components"]))
        by_id = {component["id"]: component for component in catalog["components"]}
        for component_id in ("runtime-apple", "ui-host-web", "ui-host-android", "ui-host-ios", "sdk-atmx-web"):
            source_repos = {source["repo"] for source in by_id[component_id]["sources"]}
            self.assertTrue({"axiom-runtime", "axiom-lib", "rod"} <= source_repos)

    def test_build_environment_routes_caches_off_internal_disk(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            root = Path(temporary).resolve()
            env = ctl.build_environment(root, "ui-host-android")
            for name in ("CARGO_HOME", "CARGO_TARGET_DIR", "GRADLE_USER_HOME",
                         "npm_config_cache", "npm_config_store_dir", "PIP_CACHE_DIR",
                         "HABITAT_CACHE_ROOT", "XDG_CACHE_HOME", "TMPDIR",
                         "AXIOM_UI_HOST_BUILD_ROOT", "AXIOM_UI_HOST_DIST_ROOT"):
                self.assertTrue(Path(env[name]).is_relative_to(root), name)
            self.assertFalse(Path(env["AXIOM_UI_HOST_SIGNING_TEMP_ROOT"]).is_relative_to(root))

    def test_build_environment_refuses_symlink_outside_release_root(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            root = Path(temporary) / "release"
            elsewhere = Path(temporary) / "elsewhere"
            root.mkdir()
            elsewhere.mkdir()
            (root / "artifacts").symlink_to(elsewhere, target_is_directory=True)
            with self.assertRaisesRegex(ctl.ReleaseError, "escapes the external build root"):
                ctl.build_environment(root, "ui-host-android")

    def test_renderer_checkout_must_match_lock_and_be_clean(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            workspace = Path(temporary)
            host = workspace / "axiom-ui-host"
            engine = workspace / "research" / "lynx"
            (host / "toolchain").mkdir(parents=True)
            engine.mkdir(parents=True)
            git(host, "init", "-q")
            git(engine, "init", "-q")
            git(engine, "config", "user.name", "Release Test")
            git(engine, "config", "user.email", "release-test@example.invalid")
            (engine / "source.txt").write_text("pinned source\n")
            git(engine, "add", ".")
            git(engine, "commit", "-qm", "pinned renderer")
            head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=engine).decode().strip()
            (host / "toolchain" / "engine-source.lock.json").write_text(
                json.dumps({"engine": {"commit": head}}))
            catalog = {"repositories": {"axiom-ui-host": "axiom-ui-host"}}
            with patch.dict("os.environ", {"AXIOM_UI_HOST_ENGINE_SOURCE": str(engine)}):
                ctl.verify_locked_renderer(catalog, workspace)
                (engine / "source.txt").write_text("dirty source\n")
                with self.assertRaisesRegex(ctl.ReleaseError, "uncommitted"):
                    ctl.verify_locked_renderer(catalog, workspace)

    def test_real_workspace_can_be_planned_when_sibling_repositories_are_available(self):
        catalog = ctl.read_catalog()
        required = {source["repo"] for component in catalog["components"] if component["id"] == "cli"
                    for source in component["sources"]}
        if any(not (ctl.WORKSPACE / catalog["repositories"][repo] / ".git").exists()
               for repo in required):
            self.skipTest("the isolated CI checkout does not contain the sibling repositories")
        plan = ctl.make_plan(catalog, only={"cli"})
        self.assertEqual([item["id"] for item in plan["components"]], ["cli"])
        self.assertTrue(plan["components"][0]["selected"])
        self.assertEqual(plan["format"], ctl.PLAN_FORMAT)

    def test_nested_sibling_checkout_does_not_dirty_unrelated_root_inputs(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            workspace = Path(temporary)
            git(workspace, "init", "-q")
            git(workspace, "config", "user.name", "Release Test")
            git(workspace, "config", "user.email", "release-test@example.invalid")
            (workspace / "src").mkdir()
            (workspace / "src" / "main.txt").write_text("source\n")
            git(workspace, "add", "src")
            git(workspace, "commit", "-qm", "source")
            (workspace / "other-repository").mkdir()
            git(workspace / "other-repository", "init", "-q")
            catalog = {"sha256": "test", "repositories": {"root": "."}, "components": [
                {"id": "root-tool", "owner": "root", "kind": "binary", "adapter": "ci-only",
                 "destination": "test", "sources": [{"repo": "root", "paths": ["src/"]}]}
            ]}
            self.assertEqual(ctl.make_plan(catalog, workspace)["blocked"], [])

    def test_dependency_cycle_is_rejected(self):
        with self.assertRaisesRegex(ctl.ReleaseError, "cycle"):
            ctl.topological_components([
                {"id": "a", "depends_on": ["b"]},
                {"id": "b", "depends_on": ["a"]},
            ])

    def test_unrelated_commits_reuse_artifact_but_input_change_rebuilds(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            workspace = Path(temporary)
            repo = workspace / "sample"
            repo.mkdir()
            git(repo, "init", "-q")
            git(repo, "config", "user.name", "Release Test")
            git(repo, "config", "user.email", "release-test@example.invalid")
            (repo / "src").mkdir()
            (repo / "src" / "main.txt").write_text("one\n")
            (repo / "README.md").write_text("read me\n")
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "initial")
            component = {"id": "sample", "owner": "sample", "kind": "binary", "adapter": "ci-only",
                         "destination": "test", "sources": [{"repo": "sample", "paths": ["src/"]}]}
            catalog = {"sha256": "test", "repositories": {"sample": "sample"}, "components": [component]}
            first = ctl.make_plan(catalog, workspace)
            baseline = {"sample": {"fingerprint": first["components"][0]["fingerprint"],
                                   "artifacts": [{"file": "sample.tar.gz", "sha256": ctl.sha256(b"reused")}]}}
            (repo / "README.md").write_text("new documentation\n")
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "docs only")
            unchanged = ctl.make_plan(catalog, workspace, baseline)
            self.assertTrue(unchanged["components"][0]["reuseCandidate"])
            output = workspace / "release-output"
            output.mkdir()
            artifact = output / "sample.tar.gz"
            artifact.write_bytes(b"reused")
            receipt_path = output / "receipt.json"
            receipt_path.write_text(json.dumps({"format": ctl.RECEIPT_FORMAT, "component": "sample",
                                               "fingerprint": first["components"][0]["fingerprint"],
                                               "sourceHeads": first["repositories"],
                                               "artifacts": [{"file": artifact.name, "path": str(artifact),
                                                              "sha256": ctl.sha256(b"reused")}]}) + "\n")
            with patch.dict("os.environ", {"CI": "true", "GITHUB_ACTIONS": "true",
                                        "AXIOM_RELEASE_BUILD_ROOT": str(output)}):
                staged = ctl.stage_manifest(unchanged, catalog, workspace, "2026-09-23.1",
                                            [receipt_path], output / "staged.json")
            self.assertEqual(staged["components"][0]["source"], "reused")
            (repo / "src" / "main.txt").write_text("two\n")
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "change source")
            changed = ctl.make_plan(catalog, workspace, baseline)
            self.assertTrue(changed["components"][0]["selected"])
            self.assertNotEqual(changed["components"][0]["fingerprint"], baseline["sample"]["fingerprint"])
            (repo / "src" / "main.txt").write_text("dirty\n")
            dirty = ctl.make_plan(catalog, workspace, baseline)
            self.assertIn("sample", dirty["blocked"])
            self.assertFalse(dirty["components"][0]["reuseCandidate"])

    def test_dependency_change_rebuilds_dependent(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            workspace = Path(temporary)
            repo = workspace / "sample"
            repo.mkdir()
            git(repo, "init", "-q")
            git(repo, "config", "user.name", "Release Test")
            git(repo, "config", "user.email", "release-test@example.invalid")
            (repo / "core").mkdir()
            (repo / "client").mkdir()
            (repo / "core" / "main.txt").write_text("one\n")
            (repo / "client" / "main.txt").write_text("one\n")
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "initial")
            catalog = {"sha256": "test", "repositories": {"sample": "sample"}, "components": [
                {"id": "core", "owner": "sample", "kind": "library", "adapter": "ci-only", "destination": "test",
                 "sources": [{"repo": "sample", "paths": ["core/"]}]},
                {"id": "client", "owner": "sample", "kind": "library", "adapter": "ci-only", "destination": "test",
                 "depends_on": ["core"], "sources": [{"repo": "sample", "paths": ["client/"]}]},
            ]}
            first = ctl.make_plan(catalog, workspace)
            baseline = {item["id"]: {"fingerprint": item["fingerprint"],
                                     "artifacts": [{"file": item["id"], "sha256": "a" * 64}]}
                        for item in first["components"]}
            (repo / "core" / "main.txt").write_text("two\n")
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "core change")
            changed = ctl.make_plan(catalog, workspace, baseline, only={"client"})
            self.assertEqual([item["id"] for item in changed["components"]], ["core", "client"])
            self.assertTrue(all(item["selected"] for item in changed["components"]))
            forced = ctl.make_plan(catalog, workspace, baseline, only={"client"}, force={"core"})
            self.assertEqual(forced["forced"], ["client", "core"])
            self.assertTrue(all(item["forced"] for item in forced["components"]))

    def test_changed_versioned_package_requires_new_version(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            workspace = Path(temporary)
            repo = workspace / "sample"
            repo.mkdir()
            git(repo, "init", "-q")
            git(repo, "config", "user.name", "Release Test")
            git(repo, "config", "user.email", "release-test@example.invalid")
            (repo / "src").mkdir()
            (repo / "src" / "main.txt").write_text("first\n")
            (repo / "package.json").write_text('{"version":"1.0.0"}\n')
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "first")
            catalog = {"sha256": "test", "repositories": {"sample": "sample"}, "components": [
                {"id": "package", "owner": "sample", "kind": "npm", "adapter": "ci-only",
                 "destination": "test", "version": "package.json",
                 "sources": [{"repo": "sample", "paths": ["src/", "package.json"]}]}
            ]}
            first = ctl.make_plan(catalog, workspace)["components"][0]
            baseline = {"package": {"fingerprint": first["fingerprint"], "version": "1.0.0",
                                    "artifacts": [{"file": "package.tgz", "sha256": "a" * 64}]}}
            (repo / "src" / "main.txt").write_text("changed\n")
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "changed code")
            blocked = ctl.make_plan(catalog, workspace, baseline)
            self.assertEqual(blocked["blockedVersions"], ["package"])
            (repo / "package.json").write_text('{"version":"1.1.0"}\n')
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "bump version")
            ready = ctl.make_plan(catalog, workspace, baseline)
            self.assertEqual(ready["blockedVersions"], [])
            self.assertTrue(ready["components"][0]["selected"])

    def test_receipt_rejects_tampering(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            root = Path(temporary)
            artifact = root / "artifact"
            artifact.write_bytes(b"safe")
            receipt = root / "receipt.json"
            receipt.write_text(json.dumps({"format": ctl.RECEIPT_FORMAT, "component": "sample",
                                           "fingerprint": "fingerprint", "artifacts": [
                                               {"file": "artifact", "path": str(artifact),
                                                "sha256": ctl.sha256(b"safe")}]}) + "\n")
            ctl.verify_receipt(receipt, "fingerprint")
            artifact.write_bytes(b"changed")
            with self.assertRaisesRegex(ctl.ReleaseError, "checksum mismatch"):
                ctl.verify_receipt(receipt, "fingerprint")

    def test_baseline_rejects_duplicate_components_and_unsafe_artifacts(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            path = Path(temporary) / "baseline.json"
            component = {"id": "sample", "fingerprint": "a" * 64,
                         "artifacts": [{"file": "sample.tar.gz", "sha256": "b" * 64}]}
            baseline = {"format": "axiom-platform-release-manifest/v1",
                        "status": "published", "components": [component, component]}
            path.write_text(json.dumps(baseline))
            with self.assertRaisesRegex(ctl.ReleaseError, "duplicate component"):
                ctl.read_baseline(path)
            component["artifacts"][0]["file"] = "../unsafe"
            baseline["components"] = [component]
            path.write_text(json.dumps(baseline))
            with self.assertRaisesRegex(ctl.ReleaseError, "invalid artifact"):
                ctl.read_baseline(path)

    def test_release_notes_require_owned_fragments_and_migration_for_breaking_change(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            workspace = Path(temporary)
            owner = workspace / "sample"
            folder = owner / "release-notes" / "unreleased"
            folder.mkdir(parents=True)
            git(owner, "init", "-q")
            git(owner, "config", "user.name", "Release Test")
            git(owner, "config", "user.email", "release-test@example.invalid")
            catalog = {"sha256": "test", "repositories": {"sample": "sample"},
                       "components": [{"id": "sample-tool", "owner": "sample"}]}
            plan = {"format": ctl.PLAN_FORMAT, "catalogSha256": "test",
                    "components": [{"id": "sample-tool", "owner": "sample", "selected": True}]}
            with self.assertRaisesRegex(ctl.ReleaseError, "need committed release-note fragments"):
                ctl.release_notes(plan, catalog, workspace, enforce=True)
            fragment = folder / "001.json"
            fragment.write_text(json.dumps({"component": "sample-tool", "type": "breaking",
                                            "summary": "Wire format changed."}))
            with self.assertRaisesRegex(ctl.ReleaseError, "migration guidance"):
                ctl.release_notes(plan, catalog, workspace, enforce=True)
            fragment.write_text(json.dumps({"component": "sample-tool", "type": "breaking",
                                            "summary": "Wire format changed.", "migration": "Rebuild guests."}))
            git(owner, "add", ".")
            git(owner, "commit", "-qm", "release note")
            notes = ctl.release_notes(plan, catalog, workspace, enforce=True)
            self.assertIn("Wire format changed", notes)
            self.assertIn("Migration: Rebuild guests", notes)
            owner_notes = ctl.release_notes(plan, catalog, workspace, enforce=True, owner_filter="sample")
            self.assertIn("# sample release notes", owner_notes)
            self.assertIn("Wire format changed", owner_notes)
            with self.assertRaisesRegex(ctl.ReleaseError, "need committed release-note fragments"):
                ctl.release_notes(plan, catalog, workspace, enforce=True, train_id="train.2")
            current = folder / "train.2-sample-tool.json"
            current.write_text(json.dumps({"component": "sample-tool", "type": "fix",
                                           "summary": "Repair current train."}))
            git(owner, "add", ".")
            git(owner, "commit", "-qm", "current release note")
            scoped = ctl.release_notes(plan, catalog, workspace, enforce=True, train_id="train.2")
            self.assertIn("Repair current train", scoped)
            self.assertNotIn("Wire format changed", scoped)

    def test_stage_requires_verified_receipt_and_never_claims_publication(self):
        with tempfile.TemporaryDirectory(prefix="axiom-release-test-") as temporary:
            workspace = Path(temporary)
            repo = workspace / "sample"
            repo.mkdir()
            git(repo, "init", "-q")
            git(repo, "config", "user.name", "Release Test")
            git(repo, "config", "user.email", "release-test@example.invalid")
            (repo / "source.txt").write_text("source\n")
            git(repo, "add", ".")
            git(repo, "commit", "-qm", "source")
            catalog = {"sha256": "test", "repositories": {"sample": "sample"}, "components": [
                {"id": "sample", "owner": "sample", "kind": "binary", "adapter": "ci-only",
                 "required_artifacts": ["sample.bin"],
                 "destination": "test", "sources": [{"repo": "sample", "paths": ["source.txt"]}]}
            ]}
            plan = ctl.make_plan(catalog, workspace)
            root = workspace / "release-output"
            root.mkdir()
            artifact = root / "sample.bin"
            artifact.write_bytes(b"verified bytes")
            receipt_path = root / "receipt.json"
            staged_path = root / "staged.json"
            with patch.dict("os.environ", {"CI": "true", "GITHUB_ACTIONS": "true",
                                        "AXIOM_RELEASE_BUILD_ROOT": str(root)}):
                ctl.record_artifact(plan, catalog, workspace, "sample", [artifact], receipt_path)
                staged = ctl.stage_manifest(plan, catalog, workspace, "2026-09-23.1", [receipt_path], staged_path)
            self.assertEqual(staged["status"], "staged-not-published")
            self.assertEqual(staged["components"][0]["artifacts"][0]["sha256"], ctl.sha256(b"verified bytes"))
            other = root / "other.bin"
            other.write_bytes(b"verified but wrong product")
            other_receipt = root / "other-receipt.json"
            with patch.dict("os.environ", {"CI": "true", "GITHUB_ACTIONS": "true",
                                        "AXIOM_RELEASE_BUILD_ROOT": str(root)}):
                ctl.record_artifact(plan, catalog, workspace, "sample", [other], other_receipt)
                with self.assertRaisesRegex(ctl.ReleaseError, "receipt must contain exactly"):
                    ctl.stage_manifest(plan, catalog, workspace, "2026-09-23.1", [other_receipt], staged_path)
            with self.assertRaisesRegex(ctl.ReleaseError, "published"):
                ctl.read_baseline(staged_path)
            artifact.write_bytes(b"tampered")
            with patch.dict("os.environ", {"CI": "true", "GITHUB_ACTIONS": "true",
                                        "AXIOM_RELEASE_BUILD_ROOT": str(root)}):
                with self.assertRaisesRegex(ctl.ReleaseError, "checksum mismatch"):
                    ctl.stage_manifest(plan, catalog, workspace, "2026-09-23.1", [receipt_path], staged_path)


if __name__ == "__main__":
    unittest.main()
