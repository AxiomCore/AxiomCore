"""Fail-closed checks for selective source staging and Cloud Build evidence."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import ci_builders
import ctl


class SelectiveBuilderTests(unittest.TestCase):
    def test_git_archive_uses_head_not_dirty_worktree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repository = root / "repo"
            repository.mkdir()
            subprocess.run(["git", "init", "-q", str(repository)], check=True)
            subprocess.run(["git", "-C", str(repository), "config", "user.email", "test@example.com"], check=True)
            subprocess.run(["git", "-C", str(repository), "config", "user.name", "Test"], check=True)
            (repository / "source.txt").write_text("committed\n")
            subprocess.run(["git", "-C", str(repository), "add", "source.txt"], check=True)
            subprocess.run(["git", "-C", str(repository), "commit", "-qm", "source"], check=True)
            (repository / "source.txt").write_text("dirty\n")
            destination = root / "stage"
            destination.mkdir()
            ci_builders._archive_head(repository, destination)
            self.assertEqual((destination / "source.txt").read_text(), "committed\n")

    def test_cloud_build_records_only_immutable_candidate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            context = root / "source/axiom-backend"
            context.mkdir(parents=True)
            (context / "Dockerfile").write_text("FROM scratch\n")
            artifact = root / "artifacts"
            artifact.mkdir()
            plan = {"repositories": {"axiom-backend": {"head": "a" * 40, "dirty": False}}}
            entry = {"id": "backend-api", "fingerprint": "b" * 64}
            build_id = "11111111-1111-1111-1111-111111111111"
            submitted = []

            def command(*args, **_):
                submitted.append(args)
                if args[1:3] == ("builds", "submit"):
                    return json.dumps({"id": build_id}).encode()
                if args[1:3] == ("builds", "describe"):
                    source_digest = ctl.sha256(ctl.canonical(plan["repositories"]))
                    image = ("asia-south1-docker.pkg.dev/axiomcore/axiom-backend/axiom-backend:"
                             f"candidate-{'b' * 12}-{source_digest[:12]}")
                    return json.dumps({"status": "SUCCESS", "substitutions": {
                        "_AXIOM_SOURCE_HEADS_SHA256": source_digest}, "results": {
                            "images": [{"name": image, "digest": "sha256:" + "c" * 64}]}}).encode()
                self.fail(f"unexpected command: {args}")

            with mock.patch.dict("os.environ", {"AXIOM_GCP_REGION": "asia-south1",
                                                     "AXIOM_GCP_PROJECT_ID": "axiomcore"}), \
                    mock.patch.object(ci_builders, "_stage", return_value=root / "source"), \
                    mock.patch.object(ci_builders, "_run", side_effect=command):
                output = ci_builders._image(entry, {}, root, root, artifact, {}, plan)
            descriptor = json.loads(output[0].read_text())
            self.assertEqual(descriptor["buildId"], build_id)
            self.assertEqual(descriptor["sourceHeads"], plan["repositories"])
            self.assertTrue(descriptor["image"].endswith("@sha256:" + "c" * 64))
            config = json.loads((context / "axiom-release-cloudbuild.json").read_text())
            self.assertEqual(len(config["images"]), 1)
            self.assertNotIn("stable", json.dumps(config).lower())
            self.assertIn("--async", submitted[0])

    def test_apple_sdk_pin_mismatch_blocks_before_build(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            swift = workspace / "axiom-sdk/swift"
            swift.mkdir(parents=True)
            (swift / "Package.swift").write_text(
                'url: "https://github.com/AxiomCore/AxiomCore/releases/download/v0.1.0/'
                'AxiomRuntime.xcframework.zip", checksum: "' + "a" * 64 + '"')
            ledger = {"components": {"runtime-apple": {"candidateVersion": "0.2.0"}}}
            issues = ci_builders.dependency_blockers({"sdk-swift"}, ledger, workspace)
            self.assertEqual(len(issues), 1)
            self.assertIn("successor release", issues[0])

    def test_react_lockfile_must_resolve_selected_core(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            react = workspace / "axiom-sdk/web/atmx-react"
            react.mkdir(parents=True)
            (react / "package.json").write_text(json.dumps({"dependencies": {"atmx-web": "^1.1.0"}}))
            (react / "package-lock.json").write_text(json.dumps({"packages": {
                "": {"dependencies": {"atmx-web": "^1.1.0"}},
                "node_modules/atmx-web": {"version": "1.0.0"}}}))
            ledger = {"components": {"sdk-atmx-web": {"candidateVersion": "1.1.0"}}}
            issues = ci_builders.dependency_blockers({"sdk-atmx-react"}, ledger, workspace)
            self.assertEqual(len(issues), 1)
            self.assertIn("lockfile", issues[0])

    def test_missing_tool_and_region_are_preview_blockers(self):
        with mock.patch.object(ci_builders.shutil, "which", return_value=None):
            issues = ci_builders.prerequisite_issues({"backend-api", "sdk-flutter"},
                                                      {"PATH": "/empty"})
        self.assertTrue(any("AXIOM_GCP_REGION" in item for item in issues))
        self.assertTrue(any("fvm" in item for item in issues))


if __name__ == "__main__":
    unittest.main()
