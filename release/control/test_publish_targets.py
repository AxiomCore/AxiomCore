import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import ctl
import publish_menu
import publish_targets
import release_cli


class PublishTargetTests(unittest.TestCase):
    def test_every_catalog_target_has_a_dispatch_path(self):
        catalog = ctl.read_catalog()
        covered = (set(publish_targets.GITHUB) | set(publish_targets.NPM) |
                   set(publish_targets.PUB) | set(publish_targets.GCP) |
                   {"sdk-swift", "dashboard-proxy", "landing", "docs"} |
                   publish_menu.HOST_IDS)
        self.assertEqual({item["id"] for item in catalog["components"]}, covered)

    def test_picker_groups_host_and_only_marks_complete_candidates_ready(self):
        catalog = {"components": [{"id": name} for name in (
            "ui-host-web", "ui-host-android", "ui-host-ios", "landing", "cli")]}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            ready = root / "trains/train/components/ui-host"
            ready.mkdir(parents=True)
            for name in ("intent.json", "plan.json", "staged.json", "gate.json", "notes.md"):
                (ready / name).write_text("{}")
            result = publish_menu.entries(catalog, root, "train")
        self.assertEqual(result, [("ui-host", True), ("landing", False), ("cli", False)])

    def test_host_component_alias_loads_group_candidate(self):
        with patch.object(release_cli.publisher, "load_candidate", return_value={}) as loaded, \
                patch.object(release_cli.publisher, "publish_host", return_value={"remote": {"url": "url"}}):
            release_cli.publish_one("ui-host-android", Path("/tmp"), "train")
        self.assertEqual(loaded.call_args.args[0], "ui-host")

    def test_npm_tarball_identity_must_match_staged_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "atmx-react.tgz"
            data = json.dumps({"name": "atmx-react", "version": "1.2.3"}).encode()
            with tarfile.open(archive, "w:gz") as output:
                entry = tarfile.TarInfo("package/package.json")
                entry.size = len(data)
                output.addfile(entry, io.BytesIO(data))
            candidate = {"directory": root / "sdk-atmx-react",
                         "stage": {"components": [{"id": "sdk-atmx-react", "releaseVersion": "1.2.4"}]},
                         "receipts": [root / "receipt.json"]}
            candidate["directory"].mkdir()
            with patch.object(publish_targets.ctl, "verify_receipt", return_value={
                    "component": "sdk-atmx-react", "artifacts": [{"file": archive.name, "path": str(archive)}]}):
                with self.assertRaisesRegex(ctl.ReleaseError, "identity differs"):
                    publish_targets._npm_archive(candidate, "sdk-atmx-react")

    def test_existing_npm_version_is_verified_without_republishing(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "atmx-cli.tgz"
            archive.write_bytes(b"immutable-tarball")
            candidate = {"root": root, "directory": root / "sdk-atmx-cli",
                         "intent": {"trainId": "test"},
                         "stage": {"components": [{"id": "sdk-atmx-cli", "releaseVersion": "1.2.3"}]},
                         "receipts": [root / "receipt.json"]}
            candidate["directory"].mkdir()
            integrity = "sha512-" + publish_targets.base64.b64encode(
                publish_targets.hashlib.sha512(archive.read_bytes()).digest()).decode()
            with patch.object(publish_targets.publisher, "remote_source_heads"), \
                    patch.object(publish_targets, "_npm_archive", return_value=(archive, {})), \
                    patch.object(publish_targets.ctl, "build_environment", return_value={}), \
                    patch.object(publish_targets, "_npm_metadata", return_value={
                        "name": "atmx-cli", "version": "1.2.3", "dist": {"integrity": integrity}}), \
                    patch.object(publish_targets.ctl, "run") as run:
                result = publish_targets.publish_npm(candidate, "sdk-atmx-cli")
            self.assertEqual(result["status"], "remote-verified")
            run.assert_not_called()

    def test_proxy_marker_authenticates_exact_worker_source(self):
        template = ('const DASHBOARD_ORIGIN = "https://origin.example";\n'
                    'const AXIOM_RELEASE_SHA256 = "__AXIOM_RELEASE_SHA256__";\n')
        marker = ctl.sha256(template.encode())
        worker = template.replace("__AXIOM_RELEASE_SHA256__", marker).encode()
        self.assertEqual(publish_targets._proxy_identity(worker), (marker, "https://origin.example"))
        with self.assertRaisesRegex(ctl.ReleaseError, "marker differs"):
            publish_targets._proxy_identity(worker + b"// changed")

    def test_homebrew_formula_updates_only_release_fields(self):
        before = ('class Axiom < Formula\n'
                  '  url "https://github.com/AxiomCore/AxiomCore/releases/download/v1.0.0/axiom-macos-arm64.tar.gz"\n'
                  f'  sha256 "{"a" * 64}"\n'
                  '  version "1.0.0"\n'
                  '  def install\n    bin.install "axiom"\n  end\nend\n')
        result = publish_targets._render_homebrew_formula(before, "1.0.1", "b" * 64)
        self.assertIn("download/v1.0.1/axiom-macos-arm64.tar.gz", result)
        self.assertIn('  sha256 "' + "b" * 64 + '"', result)
        self.assertIn('  version "1.0.1"', result)
        self.assertIn('bin.install "axiom"', result)
        with self.assertRaisesRegex(ctl.ReleaseError, "unrecognized release field"):
            publish_targets._render_homebrew_formula(before.replace('  version "1.0.0"\n', ''),
                                                     "1.0.1", "b" * 64)

    def test_gcp_rejects_mutable_image_before_network(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reference = root / "image-ref.json"
            reference.write_text(json.dumps({"image": "asia-south1-docker.pkg.dev/axiomcore/axiom-backend/api:latest",
                                             "sourceHeads": {}}))
            candidate = {"directory": root / "backend-api", "receipts": [root / "receipt.json"],
                         "stage": {"components": [{"id": "backend-api"}]},
                         "plan": {"repositories": {}}}
            candidate["directory"].mkdir()
            with patch.object(publish_targets.publisher, "remote_source_heads"), \
                    patch.object(publish_targets.ctl, "verify_receipt", return_value={
                        "component": "backend-api", "artifacts": [{"file": "image-ref.json", "path": str(reference)}]}), \
                    patch.object(publish_targets, "_gcloud_json") as cloud:
                with self.assertRaisesRegex(ctl.ReleaseError, "immutable Artifact Registry digest"):
                    publish_targets.publish_gcp(candidate, "backend-api")
                cloud.assert_not_called()

    def test_gcp_rejects_build_without_source_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reference = root / "image-ref.json"
            image = "asia-south1-docker.pkg.dev/axiomcore/axiom-backend/axiom-backend@sha256:" + "a" * 64
            reference.write_text(json.dumps({"image": image, "sourceHeads": {},
                                             "buildId": "11111111-1111-1111-1111-111111111111"}))
            candidate = {"directory": root / "backend-api", "receipts": [root / "receipt.json"],
                         "stage": {"components": [{"id": "backend-api"}]},
                         "plan": {"repositories": {}}, "catalog": {}, "workspace": root}
            candidate["directory"].mkdir()
            with patch.dict("os.environ", {"AXIOM_GCP_REGION": "asia-south1"}), \
                    patch.object(publish_targets.publisher, "remote_source_heads"), \
                    patch.object(publish_targets.ctl, "repo_path", return_value=root), \
                    patch.object(publish_targets.ctl, "verify_receipt", return_value={
                        "component": "backend-api", "artifacts": [{"file": "image-ref.json", "path": str(reference)}]}), \
                    patch.object(publish_targets, "_gcloud_json", return_value={"status": "SUCCESS",
                        "substitutions": {}, "results": {"images": [{"digest": "sha256:" + "a" * 64}]}}) as cloud, \
                    patch.object(publish_targets.ctl, "run") as run:
                with self.assertRaisesRegex(ctl.ReleaseError, "Cloud Build proof"):
                    publish_targets.publish_gcp(candidate, "backend-api")
                self.assertEqual(cloud.call_count, 1)
                run.assert_not_called()

    def test_gcp_api_and_worker_publish_remain_fail_closed(self):
        for component, image_name, command_prefix in (
            ("backend-api", "axiom-backend", ("gcloud", "run", "deploy")),
            ("backend-worker", "axiom-semantic-worker", ("gcloud", "run", "jobs", "update")),
        ):
            with self.subTest(component=component), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                digest = "sha256:" + "a" * 64
                image = f"us-central1-docker.pkg.dev/axiomcore/axiom-backend/{image_name}@{digest}"
                reference = root / "image-ref.json"
                reference.write_text(json.dumps({
                    "image": image, "sourceHeads": {},
                    "buildId": "11111111-1111-1111-1111-111111111111"}))
                candidate = {"plan": {"repositories": {}}, "catalog": {}, "workspace": root}
                source_digest = ctl.sha256(ctl.canonical({}))

                def cloud(*args, **_):
                    if args[:2] == ("builds", "describe"):
                        return {"status": "SUCCESS", "substitutions": {
                            "_AXIOM_SOURCE_HEADS_SHA256": source_digest},
                            "results": {"images": [{"digest": digest}]}}
                    if args[:3] == ("artifacts", "docker", "images"):
                        return {"digest": digest}
                    if args[:2] in (("scheduler", "jobs"), ("tasks", "queues")):
                        return {"state": "PAUSED"}
                    if args[:3] == ("run", "jobs", "describe"):
                        return {"spec": {"template": {"spec": {"template": {"spec": {
                            "maxRetries": 0, "containers": [{"image": image, "env": [
                                {"name": "AXIOM_RELEASE_WORKER_ENABLED", "value": "false"}]}]}}}}}}
                    if args[:3] == ("run", "services", "describe"):
                        return {"spec": {"template": {"spec": {"containers": [{"image": image,
                            "env": [{"name": "AXIOM_RELEASE_WORKER_ENABLED", "value": "false"}]}]}}},
                                "status": {"latestReadyRevisionName": "ready", "url": "https://api.test",
                                           "traffic": [{"revisionName": "ready", "percent": 100}]}}
                    self.fail(f"unexpected gcloud lookup: {args}")

                with patch.dict("os.environ", {"AXIOM_GCP_REGION": "us-central1",
                                            "AXIOM_GCP_PROJECT_ID": "axiomcore"}), \
                        patch.object(publish_targets.publisher, "remote_source_heads"), \
                        patch.object(publish_targets.ctl, "repo_path", return_value=root), \
                        patch.object(publish_targets, "_single", return_value=reference), \
                        patch.object(publish_targets, "_gcloud_json", side_effect=cloud), \
                        patch.object(publish_targets, "_save", return_value={}), \
                        patch.object(publish_targets.ctl, "run", return_value=b"") as run:
                    publish_targets.publish_gcp(candidate, component)
                deploys = [call.args for call in run.call_args_list
                           if call.args[:len(command_prefix)] == command_prefix]
                self.assertEqual(len(deploys), 1)
                self.assertIn("--update-env-vars=AXIOM_RELEASE_WORKER_ENABLED=false", deploys[0])
                if component == "backend-worker":
                    self.assertIn("--max-retries=0", deploys[0])

    def test_gcp_publisher_pauses_both_worker_triggers(self):
        states = {"scheduler": "ENABLED", "queue": "RUNNING"}

        def cloud(*args, **_):
            return {"state": states["scheduler" if args[0] == "scheduler" else "queue"]}

        def pause(*args, **_):
            states["scheduler" if args[1] == "scheduler" else "queue"] = "PAUSED"
            return b""

        with patch.object(publish_targets, "_gcloud_json", side_effect=cloud), \
                patch.object(publish_targets.ctl, "run", side_effect=pause) as run:
            publish_targets._pause_release_worker_triggers("axiomcore", "us-central1", Path("/tmp"))
        self.assertEqual(states, {"scheduler": "PAUSED", "queue": "PAUSED"})
        self.assertEqual(len(run.call_args_list), 2)

    def test_gcp_publisher_refuses_queue_that_remains_running(self):
        def cloud(*args, **_):
            return {"state": "PAUSED" if args[0] == "scheduler" else "RUNNING"}

        with patch.object(publish_targets, "_gcloud_json", side_effect=cloud), \
                patch.object(publish_targets.ctl, "run", return_value=b""):
            with self.assertRaisesRegex(ctl.ReleaseError, "not paused"):
                publish_targets._pause_release_worker_triggers("axiomcore", "us-central1", Path("/tmp"))


if __name__ == "__main__":
    unittest.main()
