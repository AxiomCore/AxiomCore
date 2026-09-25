import io
import gzip
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import ctl
import pub_publish
import publish_menu
import publish_targets
import r2_aws
import release_cli


class PublishTargetTests(unittest.TestCase):
    def test_pub_helper_uses_short_lived_service_account_token(self):
        token = MagicMock(returncode=0, stdout="temporary-identity-token\n")
        success = MagicMock(returncode=0)
        with patch.dict(pub_publish.os.environ, {"AXIOM_PUB_SERVICE_ACCOUNT":
                                               "pub-dev@axiomcore.iam.gserviceaccount.com"}, clear=True), \
                patch.object(pub_publish.subprocess, "run", side_effect=[token, success, success, success]) as run:
            self.assertEqual(pub_publish.main(), 0)
        self.assertEqual(run.call_args_list[0].args[0][:3],
                         ("gcloud", "auth", "print-identity-token"))
        self.assertIn("--audiences=https://pub.dev", run.call_args_list[0].args[0])
        self.assertEqual(run.call_args_list[1].kwargs["env"]["PUB_TOKEN"],
                         "temporary-identity-token")
        self.assertEqual(run.call_args_list[3].args[0][-2:], ("publish", "--force"))

    def test_pub_build_helper_uses_temporary_identity_without_publishing(self):
        token = MagicMock(returncode=0, stdout="temporary-identity-token\n")
        success = MagicMock(returncode=0)
        with patch.dict(pub_publish.os.environ, {"AXIOM_PUB_SERVICE_ACCOUNT":
                                               "pub-dev@axiomcore.iam.gserviceaccount.com"}, clear=True), \
                patch.object(pub_publish.subprocess, "run", side_effect=[token, success, success, success]) as run:
            self.assertEqual(pub_publish.build_main("dart"), 0)
        self.assertEqual(run.call_args_list[1].args[0][-3:],
                         ("https://pub.dev", "--env-var", "PUB_TOKEN"))
        self.assertEqual(run.call_args_list[2].args[0], ("fvm", "dart", "pub", "get"))
        self.assertEqual(run.call_args_list[3].args[0],
                         ("fvm", "dart", "pub", "publish", "--dry-run"))
        self.assertEqual(run.call_args_list[2].kwargs["env"]["PUB_TOKEN"],
                         "temporary-identity-token")

    def test_pub_helper_stops_without_configured_identity(self):
        with patch.dict(pub_publish.os.environ, {}, clear=True), \
                patch.object(pub_publish.subprocess, "run") as run:
            self.assertEqual(pub_publish.main(), 2)
        run.assert_not_called()

    def test_pub_publisher_selects_production_infisical_project(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            configuration = workspace / "AxiomCore/docs/.infisical.json"
            configuration.parent.mkdir(parents=True)
            configuration.write_text('{"workspaceId":"11111111-1111-1111-1111-111111111111"}')
            with patch.object(publish_targets.ctl, "WORKSPACE", workspace), \
                    patch.object(publish_targets.shutil, "which", return_value="/bin/infisical"):
                command = publish_targets._pub_publish_command({"PATH": "/bin"}, "pub_publish.py")
            self.assertEqual(command[:5], ["infisical", "run", "--env=prod",
                                           "--projectId=11111111-1111-1111-1111-111111111111", "--"])
            self.assertEqual(command[5:], [publish_targets.sys.executable, "pub_publish.py"])
            self.assertEqual(publish_targets._pub_publish_command({"PUB_TOKEN": "example"}, "pub_publish.py"),
                             [publish_targets.sys.executable, "pub_publish.py"])
            self.assertEqual(publish_targets._pub_publish_command({
                "AXIOM_PUB_SERVICE_ACCOUNT": "pub-dev@axiomcore.iam.gserviceaccount.com"},
                "pub_publish.py"), [publish_targets.sys.executable, "pub_publish.py"])

    def test_pub_publisher_rejects_unpublishable_staged_files_before_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "axiom_flutter_generator-0.146.1.tar.gz"
            with archive.open("wb") as output:
                with gzip.GzipFile(fileobj=output, mode="wb") as zipped:
                    with tarfile.open(fileobj=zipped, mode="w") as opened:
                        for name, body in {"pubspec.yaml": b"name: axiom_flutter_generator\nversion: 0.146.1\n",
                                           ".DS_Store": b"local"}.items():
                            info = tarfile.TarInfo(name)
                            info.size = len(body)
                            opened.addfile(info, io.BytesIO(body))
            candidate = {"directory": root / "candidate", "stage": {}}
            with patch.object(publish_targets.publisher, "remote_source_heads"), \
                    patch.object(publish_targets, "_single", return_value=archive), \
                    patch.object(publish_targets, "_version", return_value="0.146.1"), \
                    patch.object(publish_targets, "_pub_metadata", return_value=None), \
                    patch.object(publish_targets.ctl, "build_environment") as environment:
                with self.assertRaisesRegex(ctl.ReleaseError, "staged pub package includes files"):
                    publish_targets.publish_pub(candidate, "sdk-flutter-generator")
            environment.assert_not_called()

    def test_dashboard_proxy_uses_listed_pages_domain_and_reuses_partial_workspace(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            owner = root / "owner"
            owner.mkdir()
            directory = root / "candidate"
            publication = directory / "publication"
            site = publication / "site"
            site.mkdir(parents=True)
            (site / "_worker.js").write_bytes(b"staged worker")
            (publication / "tooling").mkdir()
            archive = root / "_worker.js"
            archive.write_bytes(b"staged worker")
            head = "a" * 40
            candidate = {"directory": directory, "stage": {}, "root": root,
                         "catalog": {}, "workspace": root,
                         "plan": {"repositories": {"axiom-frontend": {"head": head}}}}
            projects = [{"Project Name": "axiom-dashboard",
                         "Project Domains": "axiom-dashboard-4g7.pages.dev"}]
            deployed = "https://12345678.axiom-dashboard-4g7.pages.dev"
            with patch.object(publish_targets.publisher, "remote_source_heads"), \
                    patch.object(publish_targets.ctl, "repo_path", return_value=owner), \
                    patch.object(publish_targets.ctl, "run", return_value=head.encode()) as run, \
                    patch.object(publish_targets, "_single", return_value=archive), \
                    patch.object(publish_targets, "_proxy_identity", return_value=("m" * 64, "https://origin.test")), \
                    patch.object(publish_targets.ctl, "build_environment", return_value={}), \
                    patch.object(publish_targets.publisher, "_check_secret_names"), \
                    patch.object(publish_targets.ctl, "stage_tracked_landing_source"), \
                    patch.object(publish_targets.publisher, "_pages_project_list", return_value=projects), \
                    patch.object(publish_targets.publisher, "_cloudflare_command", side_effect=lambda command, _env: command), \
                    patch.object(publish_targets.publisher, "_run_with_combined_output", return_value=deployed), \
                    patch.object(publish_targets, "_verify_proxy") as verify, \
                    patch.object(publish_targets, "_save", return_value={"ok": True}):
                self.assertEqual(publish_targets.publish_dashboard_proxy(candidate), {"ok": True})
            run.assert_called_once_with("git", "rev-parse", "HEAD", cwd=owner)
            self.assertEqual(json.loads((publication / "deployment.json").read_text())["domain"],
                             "axiom-dashboard-4g7.pages.dev")
            self.assertEqual(verify.call_count, 2)
            self.assertEqual(verify.call_args_list[0].args[-1], "axiom-dashboard-4g7.pages.dev")
            self.assertEqual(verify.call_args_list[1].args[0],
                             "https://axiom-dashboard-4g7.pages.dev")

    def test_dashboard_proxy_rejects_url_outside_listed_project_domain(self):
        with self.assertRaisesRegex(ctl.ReleaseError, "not under its Pages project"):
            publish_targets._verify_proxy(
                "https://example.axiom-dashboard.pages.dev", "m" * 64,
                "https://origin.test", "axiom-dashboard-4g7.pages.dev")

    def test_dashboard_proxy_verifier_sends_named_user_agent_to_both_urls(self):
        url = "https://12345678.axiom-dashboard-4g7.pages.dev"
        proof = MagicMock()
        proof.__enter__.return_value.geturl.return_value = url + "/.well-known/axiom-dashboard-release"
        proof.__enter__.return_value.read.return_value = json.dumps({
            "sha256": "m" * 64, "origin": "https://origin.test"}).encode()
        root = MagicMock()
        root.__enter__.return_value.status = 200
        with patch.object(publish_targets.urllib.request, "urlopen", side_effect=[proof, root]) as opened:
            publish_targets._verify_proxy(url, "m" * 64, "https://origin.test",
                                          "axiom-dashboard-4g7.pages.dev")
        self.assertEqual(opened.call_count, 2)
        for call in opened.call_args_list:
            self.assertEqual(call.args[0].get_header("User-agent"), "axiom-release-verifier/1")

    def test_dashboard_proxy_retries_initial_403_then_verifies_exact_marker(self):
        url = "https://12345678.axiom-dashboard-4g7.pages.dev"
        temporary_403 = publish_targets.urllib.error.HTTPError(url, 403, "Forbidden", {}, None)
        with patch.object(publish_targets, "_verify_proxy", side_effect=[temporary_403, None]) as verify, \
                patch.object(publish_targets.time, "sleep") as sleep:
            publish_targets._verify_proxy_ready(url, "m" * 64, "https://origin.test",
                                                "axiom-dashboard-4g7.pages.dev")
        self.assertEqual(verify.call_count, 2)
        sleep.assert_called_once_with(5)

    def test_dashboard_proxy_reports_persistent_403_without_bypassing_proof(self):
        url = "https://12345678.axiom-dashboard-4g7.pages.dev"
        denied = publish_targets.urllib.error.HTTPError(url, 403, "Forbidden", {}, None)
        with patch.object(publish_targets, "_verify_proxy", side_effect=denied) as verify, \
                patch.object(publish_targets.time, "sleep"):
            with self.assertRaisesRegex(ctl.ReleaseError, "did not serve the exact staged marker"):
                publish_targets._verify_proxy_ready(url, "m" * 64, "https://origin.test",
                                                    "axiom-dashboard-4g7.pages.dev")
        self.assertEqual(verify.call_count, 18)

    def test_npm_registry_integrity_retries_then_accepts_exact_bytes(self):
        wrong = {"dist": {"integrity": "sha512-not-yet-converged"}}
        right = {"dist": {"integrity": "sha512-exact"}}
        with patch.object(publish_targets, "_npm_metadata", return_value=right) as query, \
                patch.object(publish_targets.time, "sleep") as sleep:
            result = publish_targets._wait_for_npm_integrity(
                "atmx-web", "0.147.0", "sha512-exact", Path("/tmp"), {}, wrong)
        self.assertIs(result, right)
        query.assert_called_once()
        sleep.assert_called_once_with(5)
        with patch.object(publish_targets.time, "monotonic", side_effect=[0, 301]):
            with self.assertRaisesRegex(ctl.ReleaseError, "did not converge"):
                publish_targets._wait_for_npm_integrity(
                    "atmx-web", "0.147.0", "sha512-exact", Path("/tmp"), {}, wrong)

    def test_npm_metadata_reads_uncached_public_version_endpoint(self):
        value = {"name": "atmx-react", "version": "0.147.0",
                 "dist": {"integrity": "sha512-exact"}}
        with patch.object(publish_targets.urllib.request, "urlopen",
                          return_value=io.BytesIO(json.dumps(value).encode())) as opened:
            self.assertEqual(publish_targets._npm_metadata("atmx-react", "0.147.0",
                                                            Path("/tmp"), {}), value)
        request = opened.call_args.args[0]
        self.assertEqual(request.full_url,
                         "https://registry.npmjs.org/atmx-react/0.147.0")
        self.assertEqual(request.get_header("Cache-control"), "no-cache")
        missing = publish_targets.urllib.error.HTTPError(
            request.full_url, 404, "Not Found", {}, None)
        with patch.object(publish_targets.urllib.request, "urlopen", side_effect=missing):
            self.assertIsNone(publish_targets._npm_metadata("atmx-react", "0.147.1",
                                                             Path("/tmp"), {}))

    def test_npm_verifies_downloaded_registry_tarball(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "atmx-web-1.2.3.tgz"
            archive.write_bytes(b"exact tarball")
            metadata = {"dist": {"tarball":
                        "https://registry.npmjs.org/atmx-web/-/atmx-web-1.2.3.tgz"}}
            with patch.object(publish_targets.urllib.request, "urlopen",
                              return_value=io.BytesIO(archive.read_bytes())) as opened:
                publish_targets._verify_npm_tarball("atmx-web", "1.2.3", metadata, archive)
            request = opened.call_args.args[0]
            self.assertEqual(request.get_header("User-agent"), "AxiomCore-Release-Verifier/1.0")
            with patch.object(publish_targets.urllib.request, "urlopen",
                              return_value=io.BytesIO(b"wrong bytes!")):
                with self.assertRaisesRegex(ctl.ReleaseError, "differs from the exact staged bytes"):
                    publish_targets._verify_npm_tarball("atmx-web", "1.2.3", metadata, archive)

    def test_r2_public_verification_uses_named_verifier_and_exact_bytes(self):
        payload = b"pinned browser asset"
        expected = ctl.sha256(payload)
        with patch.object(publish_targets.urllib.request, "urlopen") as opened:
            opened.return_value.__enter__.return_value.read.return_value = payload
            publish_targets._verify_r2_public("https://atmx.axiomcore.dev/v1/file.js", expected)
            request = opened.call_args.args[0]
            self.assertEqual(request.get_header("User-agent"), "AxiomCore-Release-Verifier/1.0")
            with self.assertRaisesRegex(ctl.ReleaseError, "different bytes"):
                publish_targets._verify_r2_public("https://atmx.axiomcore.dev/v1/file.js", "0" * 64)

    def test_r2_does_not_read_local_env_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            package = workspace / "axiom-sdk/web/atmx"
            package.mkdir(parents=True)
            (package / ".env").write_text(
                "CLOUDFLARE_ACCOUNT_ID=" + "a" * 32 + "\n"
                "AWS_ACCESS_KEY_ID=scoped-id\nAWS_SECRET_ACCESS_KEY=scoped-secret\n")
            with patch.object(publish_targets.shutil, "which", return_value=None):
                with self.assertRaisesRegex(ctl.ReleaseError, "Infisical prod"):
                    publish_targets._r2_auth({}, workspace)
            account, prefix, environment = publish_targets._r2_auth({
                "CLOUDFLARE_ACCOUNT_ID": "a" * 32,
                "AWS_ACCESS_KEY_ID": "explicit-id",
                "AWS_SECRET_ACCESS_KEY": "explicit-secret"}, workspace)
            self.assertEqual(account, "a" * 32)
            self.assertEqual(prefix, [])
            self.assertEqual(environment["AWS_ACCESS_KEY_ID"], "explicit-id")
            self.assertEqual(environment["AWS_SECRET_ACCESS_KEY"], "explicit-secret")
            self.assertEqual(environment["AWS_DEFAULT_REGION"], "auto")
            self.assertEqual(environment["AWS_REGION"], "auto")
            with self.assertRaisesRegex(ctl.ReleaseError, "partial AWS R2 key pair"):
                publish_targets._r2_auth({"AWS_ACCESS_KEY_ID": "unpaired"}, workspace)

    def test_r2_infisical_uses_scoped_child_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            frontend = workspace / "axiom-frontend"
            frontend.mkdir()
            (frontend / ".infisical.json").write_text(
                '{"workspaceId":"11111111-1111-1111-1111-111111111111"}')
            with patch.object(publish_targets.shutil, "which", return_value="/bin/infisical"), \
                    patch.object(publish_targets.ctl, "run", return_value=("a" * 32).encode()):
                account, prefix, environment = publish_targets._r2_auth({}, workspace)
            self.assertEqual(account, "a" * 32)
            self.assertEqual(prefix[:4], ["infisical", "run",
                                          "--projectId=11111111-1111-1111-1111-111111111111", "--env=prod"])
            self.assertTrue(prefix[-1].endswith("r2_aws.py"))
            self.assertNotIn("AWS_ACCESS_KEY_ID", environment)

    def test_r2_aws_wrapper_receives_subcommand_once(self):
        command = ["aws", "s3api", "head-bucket", "--bucket", "atmx"]
        wrapped = ("a" * 32, ["infisical", "run", "--", "python", "r2_aws.py"], {})
        direct = ("a" * 32, [], {})
        self.assertEqual(publish_targets._r2_argv(command, wrapped),
                         [*wrapped[1], "s3api", "head-bucket", "--bucket", "atmx"])
        self.assertEqual(publish_targets._r2_argv(command, direct), command)

    def test_r2_child_requires_atmx_scoped_infisical_pair(self):
        with patch.dict("os.environ", {}, clear=True), patch.object(r2_aws.os, "execvpe") as execute:
            with self.assertRaisesRegex(SystemExit, "AWS_ACCESS_KEY_ID"):
                r2_aws.main()
            execute.assert_not_called()
        with patch.dict("os.environ", {"AWS_ACCESS_KEY_ID": "id",
                                        "AWS_SECRET_ACCESS_KEY": "secret",
                                        "UNRELATED_PRODUCTION_SECRET": "not-for-aws"}, clear=True), \
                patch.object(r2_aws.os, "execvpe") as execute, \
                patch.object(r2_aws.sys, "argv", ["r2_aws.py", "s3api", "head-bucket"]):
            r2_aws.main()
            self.assertEqual(execute.call_args.args[1], ["aws", "s3api", "head-bucket"])
            self.assertEqual(execute.call_args.args[2]["AWS_ACCESS_KEY_ID"], "id")
            self.assertNotIn("UNRELATED_PRODUCTION_SECRET", execute.call_args.args[2])

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
                    patch.object(publish_targets, "_verify_npm_tarball"), \
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
