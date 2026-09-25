"""Fail-closed checks for selective source staging and Cloud Build evidence."""

import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock

import ci_builders
import ctl


class SelectiveBuilderTests(unittest.TestCase):
    def test_pub_archive_excludes_hidden_and_gitignored_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary) / "package"
            (package / "lib").mkdir(parents=True)
            (package / ".summary_files").mkdir()
            (package / "pubspec.yaml").write_text("name: sample\nversion: 1.0.0\n")
            (package / "lib/main.dart").write_text("void main() {}\n")
            (package / ".gitignore").write_text("*.iml\n")
            (package / ".DS_Store").write_text("local")
            (package / ".summary_files/report.md").write_text("private")
            (package / "sample.iml").write_text("local")
            archive = ci_builders._archive_package(package, Path(temporary) / "sample.tar.gz")
            with tarfile.open(archive, "r:gz") as opened:
                self.assertEqual(set(opened.getnames()), {"pubspec.yaml", "lib/main.dart"})

    def test_flutter_staged_changelog_comes_from_reviewed_intent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "axiom_flutter_generator"
            package.mkdir()
            (package / "pubspec.yaml").write_text("name: axiom_flutter_generator\nversion: 0.146.0\n")
            changelog = package / "CHANGELOG.md"
            changelog.write_text("## 0.0.3\n\n- Old entry.\n")
            (root / "intent.json").write_text(json.dumps({"changes": [
                {"component": "sdk-flutter-generator", "version": "0.146.0", "type": "feature",
                 "summary": "Improve generated models."}]}))
            plan = root / "plan.json"
            ci_builders._prepare_flutter_changelog(package, "sdk-flutter-generator", plan)
            first = changelog.read_text()
            self.assertTrue(first.startswith("## 0.146.0"))
            ci_builders._prepare_flutter_changelog(package, "sdk-flutter-generator", plan)
            self.assertEqual(changelog.read_text(), first)
            (package / "pubspec.yaml").write_text("name: axiom_flutter_generator\nversion: 0.147.0\n")
            with self.assertRaisesRegex(ctl.ReleaseError, "differs from the prepared release intent"):
                ci_builders._prepare_flutter_changelog(package, "sdk-flutter-generator", plan)

    def test_flutter_build_uses_scoped_pub_identity_for_get_and_dry_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            package = source / "axiom-sdk/flutter/axiom_flutter_generator"
            package.mkdir(parents=True)
            (package / "pubspec.yaml").write_text(
                "name: axiom_flutter_generator\nversion: 0.146.2\n")
            artifact = root / "artifact"
            artifact.mkdir()
            with mock.patch.object(ci_builders, "_stage", return_value=source), \
                    mock.patch.object(ci_builders, "_prepare_flutter_changelog"), \
                    mock.patch.object(ci_builders, "_run") as run, \
                    mock.patch.object(ci_builders, "_archive_package", return_value=artifact / "package.tar.gz"), \
                    mock.patch("publish_targets._pub_publish_command",
                               return_value=["python", "pub_publish.py"]) as command:
                ci_builders._local({"id": "sdk-flutter-generator"}, {}, root, root,
                                   artifact, {}, {}, root / "plan.json")
            command.assert_called_once()
            run.assert_called_once_with("python", "pub_publish.py", "build", "dart",
                                        cwd=package, env={})

    def test_stage_accepts_plan_source_groups(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            workspace = root / "workspace"
            workspace.mkdir()
            work = root / "work"
            work.mkdir()
            entry = {"id": "runtime-apple", "owner": "axiom-runtime",
                     "sourceGroups": [{"repo": "axiom-runtime", "paths": ["src/", "Cargo.toml"]},
                                      {"repo": "axiom-lib", "paths": ["src/"]}]}
            catalog = {"repositories": {"axiom-runtime": "axiom-runtime",
                                        "axiom-lib": "axiom-lib"}}
            with mock.patch.object(ci_builders.ctl, "repo_path", side_effect=lambda _catalog, name, _workspace: workspace / name), \
                    mock.patch.object(ci_builders, "_archive_head") as archive:
                source = ci_builders._stage(entry, catalog, workspace, work)
            self.assertEqual({call.args[1].relative_to(source).as_posix()
                              for call in archive.call_args_list}, {"axiom-runtime", "axiom-lib"})
            self.assertTrue(all(len(call.args) == 2 for call in archive.call_args_list))

    def test_dashboard_stage_includes_pinned_cli_embed_files(self):
        catalog = ctl.read_catalog()
        for component_id in ("cli", "dashboard-origin"):
            component = next(item for item in catalog["components"] if item["id"] == component_id)
            selectors = {path for group in component["sources"] if group["repo"] == "acore-diff"
                         for path in group["paths"]}
            self.assertIn("packages/axiom-lynx-runtime/src/index.js", selectors)
            self.assertIn("packages/axiom-lynx-runtime/src/lynx-adapter.js", selectors)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repository = root / "workspace"
            (repository / "acore-diff/src").mkdir(parents=True)
            (repository / "packages/axiom-lynx-runtime/src").mkdir(parents=True)
            (repository / "acore-diff/src/lib.rs").write_text("// pinned source\n")
            for filename in ("index.js", "lynx-adapter.js"):
                (repository / "packages/axiom-lynx-runtime/src" / filename).write_text(filename)
            subprocess.run(["git", "init", "-q", str(repository)], check=True)
            subprocess.run(["git", "-C", str(repository), "config", "user.email", "test@example.com"], check=True)
            subprocess.run(["git", "-C", str(repository), "config", "user.name", "Test"], check=True)
            subprocess.run(["git", "-C", str(repository), "add", "acore-diff", "packages"], check=True)
            subprocess.run(["git", "-C", str(repository), "commit", "-qm", "pinned sources"], check=True)
            work = root / "work"
            work.mkdir()
            entry = {"id": "root-source-stage", "owner": "acore-diff", "sourceGroups": [{
                "repo": "acore-diff", "paths": ["acore-diff/src/",
                                               "packages/axiom-lynx-runtime/src/index.js",
                                               "packages/axiom-lynx-runtime/src/lynx-adapter.js"]}]}
            with mock.patch.object(ci_builders.ctl, "repo_path", return_value=repository):
                source = ci_builders._stage(entry, {"repositories": {"acore-diff": "."}},
                                                  repository, work)
            for filename in ("index.js", "lynx-adapter.js"):
                self.assertEqual((source / "packages/axiom-lynx-runtime/src" / filename).read_text(), filename)

    def test_stage_does_not_archive_unrelated_tracked_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            workspace = root / "workspace"
            repository = workspace / "AxiomCore"
            repository.mkdir(parents=True)
            subprocess.run(["git", "init", "-q", str(repository)], check=True)
            subprocess.run(["git", "-C", str(repository), "config", "user.email", "test@example.com"], check=True)
            subprocess.run(["git", "-C", str(repository), "config", "user.name", "Test"], check=True)
            (repository / "release/scripts").mkdir(parents=True)
            (repository / "release/scripts/build_apple.sh").write_text("#!/bin/sh\n")
            (repository / "node_modules/.bin").mkdir(parents=True)
            (repository / "node_modules/.bin/tsc").symlink_to("../../release/scripts/build_apple.sh")
            subprocess.run(["git", "-C", str(repository), "add", "release", "node_modules"], check=True)
            subprocess.run(["git", "-C", str(repository), "commit", "-qm", "source"], check=True)
            entry = {"id": "runtime-apple", "owner": "AxiomCore", "sourceGroups": [
                {"repo": "AxiomCore", "paths": ["release/scripts/build_apple.sh"]}]}
            catalog = {"repositories": {"AxiomCore": "AxiomCore"}}
            work = root / "work"
            work.mkdir()
            source = ci_builders._stage(entry, catalog, workspace, work)
            self.assertTrue((source / "AxiomCore/release/scripts/build_apple.sh").is_file())
            self.assertFalse((source / "AxiomCore/node_modules").exists())

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
