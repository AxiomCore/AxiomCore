import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import ctl
import publisher


class PublisherTests(unittest.TestCase):
    def test_pages_verifier_rejects_wrong_project_before_network(self):
        with patch.object(publisher.urllib.request, "urlopen") as opened:
            with self.assertRaisesRegex(ctl.ReleaseError, "URL for axiom-landing"):
                publisher._verify_pages_files("https://other.pages.dev", {"index.html": "x"})
            opened.assert_not_called()

    def test_pages_verifier_ignores_deployment_directives(self):
        with patch.object(publisher.urllib.request, "urlopen") as opened:
            publisher._verify_pages_files("https://axiomcore-docs.pages.dev",
                                          {"_headers": "x", "_redirects": "y"}, "axiomcore-docs.pages.dev")
            opened.assert_not_called()

    def test_extract_static_archive_accepts_only_regular_relative_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "site.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                for name, content in (("index.html", b"<html>ready</html>"),
                                      ("_astro/app.js", b"export {}")):
                    entry = tarfile.TarInfo(name)
                    entry.size = len(content)
                    output.addfile(entry, io.BytesIO(content))
            files = publisher._extract_static_archive(archive, root / "site")
            self.assertEqual(files["index.html"], ctl.sha256(b"<html>ready</html>"))
            self.assertEqual((root / "site/_astro/app.js").read_bytes(), b"export {}")

    def test_extract_static_archive_rejects_path_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "site.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                entry = tarfile.TarInfo("../escape")
                entry.size = 1
                output.addfile(entry, io.BytesIO(b"x"))
            with self.assertRaisesRegex(ctl.ReleaseError, "unsafe static site archive"):
                publisher._extract_static_archive(archive, root / "site")
            self.assertFalse((root / "escape").exists())

    def test_existing_wrong_host_tag_blocks_before_bundle_or_release(self):
        candidate = {"root": Path("/tmp"), "directory": Path("/tmp/ui-host"),
                     "stage": {"components": [{"releaseVersion": "0.6.7"}]},
                     "plan": {"repositories": {"axiom-ui-host": {"head": "correct"}}},
                     "catalog": {}, "workspace": Path("/tmp")}
        with patch.object(publisher.ctl, "repo_path", return_value=Path("/tmp/host")), \
                patch.object(publisher, "_assert_github_origin"), \
                patch.object(publisher, "remote_source_heads"), \
                patch.object(publisher, "prepare_host_bundle") as bundle, \
                patch.object(publisher, "_remote_tag_target", return_value="wrong"), \
                patch.object(publisher, "_github_release_exists") as release:
            with self.assertRaisesRegex(ctl.ReleaseError, "belongs to another source commit"):
                publisher.publish_host(candidate)
            bundle.assert_not_called()
            release.assert_not_called()

    def test_candidate_train_override_rejects_path_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(ctl.ReleaseError, "invalid release train ID"):
                publisher.load_candidate("landing", Path(temporary), train_id="../wrong")

    def test_landing_publishes_exact_archive_to_project_and_records_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "axiom-landing-pages.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                entry = tarfile.TarInfo("index.html")
                entry.size = 5
                output.addfile(entry, io.BytesIO(b"hello"))
            candidate = {
                "root": root, "workspace": root, "catalog": {},
                "directory": root / "trains/test/components/landing",
                "receipts": [root / "receipt.json"],
                "intent": {"trainId": "test"},
                "stage": {"status": "staged-not-published"},
                "plan": {"repositories": {"axiom-frontend": {"head": "abc123"}}},
            }
            calls = []
            created = False

            def run(*args, **kwargs):
                nonlocal created
                calls.append(args)
                if args[-4:] == ("pages", "project", "list", "--json"):
                    return (b'[{"Project Name":"axiom-landing",'
                            b'"Project Domains":"axiom-landing-efz.pages.dev"}]' if created else b'[]')
                if "create" in args:
                    created = True
                return b""

            with patch.object(publisher, "remote_source_heads"), \
                    patch.object(publisher.ctl, "repo_path", return_value=root), \
                    patch.object(publisher.ctl, "verify_receipt", return_value={
                        "artifacts": [{"file": archive.name, "path": str(archive)}]}), \
                    patch.object(publisher.ctl, "build_environment", return_value={
                        "CLOUDFLARE_ACCOUNT_ID": "account", "CLOUDFLARE_API_TOKEN": "token"}), \
                    patch.object(publisher, "_check_secret_names"), \
                    patch.object(publisher.ctl, "stage_tracked_landing_source"), \
                    patch.object(publisher.ctl, "run", side_effect=run), \
                    patch.object(publisher, "_run_with_combined_output",
                                 return_value="https://abc.axiom-landing-efz.pages.dev"), \
                    patch.object(publisher, "_verify_pages_files") as verify:
                result = publisher.publish_landing(candidate)
            self.assertEqual(result["status"], "remote-verified")
            self.assertEqual(result["files"]["index.html"], ctl.sha256(b"hello"))
            self.assertTrue(any("pages" in args and "project" in args for args in calls))
            self.assertTrue(any("create" in args and "--force" in args for args in calls))
            self.assertEqual(verify.call_count, 2)
            self.assertTrue((candidate["directory"] / "publication/published.json").is_file())

    def test_landing_resumes_verified_upload_without_deploying_again(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "axiom-landing-pages.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                entry = tarfile.TarInfo("index.html")
                entry.size = 5
                output.addfile(entry, io.BytesIO(b"hello"))
            directory = root / "trains/test/components/landing"
            publication = directory / "publication"
            (publication / "site").mkdir(parents=True)
            (publication / "tooling").mkdir()
            (publication / "site/index.html").write_bytes(b"hello")
            stage = {"status": "staged-not-published"}
            ctl.write_json(publication / "deploy-intent.json", {
                "stageSha256": ctl.sha256(ctl.canonical(stage)), "project": "axiom-landing",
                "files": {"index.html": ctl.sha256(b"hello")}})
            candidate = {
                "root": root, "workspace": root, "catalog": {}, "directory": directory,
                "receipts": [root / "receipt.json"], "intent": {"trainId": "test"},
                "stage": stage,
                "plan": {"repositories": {"axiom-frontend": {"head": "6c80c500ee98"}}},
            }

            def run(*args, **kwargs):
                if "project" in args and "list" in args:
                    return (b'[{"Project Name":"axiom-landing",'
                            b'"Project Domains":"axiom-landing-efz.pages.dev"}]')
                if "deployment" in args and "list" in args:
                    return (b'[{"Branch":"main","Source":"6c80c50",'
                            b'"Deployment":"https://abc.axiom-landing-efz.pages.dev"}]')
                self.fail(f"unexpected Cloudflare command: {args}")

            with patch.object(publisher, "remote_source_heads"), \
                    patch.object(publisher.ctl, "repo_path", return_value=root), \
                    patch.object(publisher.ctl, "verify_receipt", return_value={
                        "artifacts": [{"file": archive.name, "path": str(archive)}]}), \
                    patch.object(publisher.ctl, "build_environment", return_value={
                        "CLOUDFLARE_ACCOUNT_ID": "account", "CLOUDFLARE_API_TOKEN": "token"}), \
                    patch.object(publisher, "_check_secret_names"), \
                    patch.object(publisher.ctl, "run", side_effect=run), \
                    patch.object(publisher, "_run_with_combined_output") as upload, \
                    patch.object(publisher, "_verify_pages_files") as verify:
                result = publisher.publish_landing(candidate)
            upload.assert_not_called()
            self.assertEqual(verify.call_count, 2)
            self.assertEqual(result["remote"], "https://abc.axiom-landing-efz.pages.dev")
            self.assertTrue((publication / "published.json").is_file())


if __name__ == "__main__":
    unittest.main()
