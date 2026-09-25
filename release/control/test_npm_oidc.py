import base64
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import ctl
import npm_oidc
import publish_targets


class NpmOidcTests(unittest.TestCase):
    def test_all_npm_packages_have_matching_workflows_and_repository_urls(self):
        paths = {
            "sdk-atmx-web": ctl.WORKSPACE / "axiom-sdk/web/atmx",
            "sdk-atmx-react": ctl.WORKSPACE / "axiom-sdk/web/atmx-react",
            "sdk-atmx-cli": ctl.WORKSPACE / "AxiomCore/release/atmx-cli",
        }
        for component, directory in paths.items():
            with self.subTest(component=component):
                repository, _, package = npm_oidc.OWNERS[component]
                manifest = json.loads((directory / "package.json").read_text())
                self.assertEqual(manifest["name"], package)
                self.assertEqual(manifest["repository"]["url"],
                                 f"git+https://github.com/{repository}.git")
                workflow = ((directory if component != "sdk-atmx-cli" else ctl.WORKSPACE / "AxiomCore")
                            / ".github/workflows" / npm_oidc.WORKFLOW).read_text()
                self.assertIn("id-token: write", workflow)
                self.assertIn("npm publish \"$NPM_ARCHIVE\"", workflow)
                self.assertIn("--ignore-scripts", workflow)
                self.assertNotIn("npm login", workflow)
                self.assertNotIn("secrets.NPM", workflow)
                self.assertNotIn("_authToken", workflow)
        legacy = (ctl.WORKSPACE / "AxiomCore/release/justfile").read_text()
        self.assertNotIn("npm publish --access", legacy)
        self.assertIn("Legacy release-atmx", legacy)

    def test_exact_existing_registry_version_needs_no_dispatch_or_token(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "atmx-cli.tgz"
            archive.write_bytes(b"staged")
            integrity = "sha512-" + base64.b64encode(hashlib.sha512(b"staged").digest()).decode()
            candidate = {"root": root, "directory": root / "sdk-atmx-cli",
                         "intent": {"trainId": "train"},
                         "stage": {"components": [{"id": "sdk-atmx-cli", "releaseVersion": "1.2.3"}]}}
            candidate["directory"].mkdir()
            with patch.object(publish_targets.publisher, "remote_source_heads"), \
                    patch.object(publish_targets, "_npm_archive", return_value=(archive, {})), \
                    patch.object(publish_targets.ctl, "build_environment",
                                 return_value={"NPM_TOKEN": "old-secret", "NODE_AUTH_TOKEN": "old-secret"}), \
                    patch.object(publish_targets, "_npm_metadata", return_value={
                        "name": "atmx-cli", "version": "1.2.3", "dist": {"integrity": integrity}}) as metadata, \
                    patch.object(publish_targets.npm_oidc, "publish") as dispatch:
                result = publish_targets.publish_npm(candidate, "sdk-atmx-cli")
            self.assertEqual(result["status"], "remote-verified")
            self.assertNotIn("NPM_TOKEN", metadata.call_args.args[3])
            self.assertNotIn("NODE_AUTH_TOKEN", metadata.call_args.args[3])
            dispatch.assert_not_called()

    def test_missing_version_uses_oidc_and_checks_registry_integrity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "atmx-cli.tgz"
            archive.write_bytes(b"staged")
            integrity = "sha512-" + base64.b64encode(hashlib.sha512(b"staged").digest()).decode()
            candidate = {"root": root, "directory": root / "sdk-atmx-cli",
                         "intent": {"trainId": "train"},
                         "stage": {"components": [{"id": "sdk-atmx-cli", "releaseVersion": "1.2.3"}]}}
            candidate["directory"].mkdir()
            with patch.object(publish_targets.publisher, "remote_source_heads"), \
                    patch.object(publish_targets, "_npm_archive", return_value=(archive, {})), \
                    patch.object(publish_targets.ctl, "build_environment", return_value={}), \
                    patch.object(publish_targets, "_npm_metadata", side_effect=[None, {
                        "name": "atmx-cli", "version": "1.2.3", "dist": {"integrity": integrity}}]), \
                    patch.object(publish_targets.npm_oidc, "publish",
                                 return_value={"workflowRun": "https://example.test/run"}) as dispatch:
                result = publish_targets.publish_npm(candidate, "sdk-atmx-cli")
            dispatch.assert_called_once_with(candidate, "sdk-atmx-cli", archive)
            self.assertEqual(result["details"]["oidc"]["workflowRun"], "https://example.test/run")

    def test_changed_handoff_marker_blocks_another_dispatch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "candidate"
            (directory / "publication").mkdir(parents=True)
            (directory / "publication/oidc-dispatch.json").write_text("{}")
            archive = root / "package.tgz"
            archive.write_bytes(b"staged")
            candidate = {"directory": directory,
                         "intent": {"trainId": "train"},
                         "stage": {"components": [{"id": "sdk-atmx-cli", "releaseVersion": "1.2.3"}]},
                         "plan": {"repositories": {"AxiomCore": {"head": "a" * 40}}}}
            with patch.object(npm_oidc.ctl, "run") as run:
                with self.assertRaisesRegex(ctl.ReleaseError, "different candidate bytes"):
                    npm_oidc.publish(candidate, "sdk-atmx-cli", archive)
            run.assert_not_called()

    def test_prerelease_handoff_dispatches_once_and_resume_reuses_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "sdk-atmx-cli"
            directory.mkdir()
            archive = root / "atmx-cli-1.2.3.tgz"
            archive.write_bytes(b"exact staged npm archive")
            sha = "a" * 40
            tag = "npm-candidate-sdk-atmx-cli-train-v1.2.3"
            candidate = {"directory": directory,
                         "intent": {"trainId": "train"},
                         "stage": {"components": [{"id": "sdk-atmx-cli", "releaseVersion": "1.2.3"}]},
                         "plan": {"repositories": {"AxiomCore": {"head": sha}}}}
            release = {"draft": False, "prerelease": True, "target_commitish": sha,
                       "assets": [{"name": archive.name, "size": archive.stat().st_size}]}
            run = {"id": 42, "status": "completed", "conclusion": "success",
                   "html_url": "https://github.com/AxiomCore/AxiomCore/actions/runs/42"}
            commands = []

            def command(*args, **kwargs):
                commands.append(args)
                if args[:3] == ("gh", "release", "download"):
                    shutil.copyfile(archive, Path(args[args.index("--dir") + 1]) / archive.name)
                return b""

            def github(*args, **_):
                if args[0] == "repos/AxiomCore/AxiomCore":
                    return {"default_branch": "main"}
                if args[0].endswith("/actions/runs/42"):
                    return run
                return {"content": "workflow exists"}

            with patch.object(npm_oidc, "_gh_json", side_effect=github), \
                    patch.object(npm_oidc, "_release", side_effect=[None, release, release]), \
                    patch.object(npm_oidc, "_workflow_runs", side_effect=[[], [run], [run]]), \
                    patch.object(npm_oidc.ctl, "run", side_effect=command):
                first = npm_oidc.publish(candidate, "sdk-atmx-cli", archive)
                second = npm_oidc.publish(candidate, "sdk-atmx-cli", archive)
            self.assertEqual(first, second)
            self.assertEqual(first["handoffTag"], tag)
            self.assertEqual(len([args for args in commands if args[:3] ==
                                  ("gh", "workflow", "run")]), 1)
            self.assertEqual(json.loads((directory / "publication/oidc-dispatch.json").read_text())["sha256"],
                             ctl.sha256_file(archive))


if __name__ == "__main__":
    unittest.main()
