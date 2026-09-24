"""Focused safety checks for the loopback release dashboard."""

from __future__ import annotations

from http.server import ThreadingHTTPServer
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen

import web_server


def git(repository: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repository).decode().strip()


class ReleaseWebTests(unittest.TestCase):
    def test_review_ignores_untracked_nested_git_checkouts(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "aggregate"
            repository.mkdir()
            git(repository, "init", "-q")
            nested = repository / "nested"
            nested.mkdir()
            git(nested, "init", "-q")
            (nested / "source.txt").write_text("owned by nested repository\n")
            (repository / "review.txt").write_text("owned by aggregate repository\n")
            self.assertEqual([item["path"] for item in web_server.changed_files(repository)],
                             ["review.txt"])

    def test_release_groups_follow_dependencies_and_group_hosts(self):
        catalog = {"components": [
            {"id": "sdk", "depends_on": ["runtime"]},
            {"id": "ui-host-ios"}, {"id": "runtime"},
            {"id": "ui-host-web"}, {"id": "ui-host-android"},
        ]}
        self.assertEqual(web_server.release_groups(catalog, {
            "sdk", "runtime", *web_server.HOST_IDS}), ["runtime", "sdk", "ui-host"])

    def test_optional_rollout_predecessors_do_not_force_unaffected_components(self):
        catalog = {"components": [
            {"id": "worker", "release_after": ["api", "runner"]},
            {"id": "api"}, {"id": "runner"},
        ]}
        self.assertEqual(web_server.release_groups(catalog, {"worker", "api", "runner"}),
                         ["api", "runner", "worker"])
        self.assertEqual(web_server.release_groups(catalog, {"worker"}), ["worker"])

    def test_review_commit_keeps_unselected_work_uncommitted(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "owner"
            repository.mkdir()
            git(repository, "init", "-q")
            git(repository, "config", "user.name", "Release Test")
            git(repository, "config", "user.email", "release@example.test")
            (repository / "selected.txt").write_text("before\n")
            (repository / "other.txt").write_text("before\n")
            git(repository, "add", "--", "selected.txt", "other.txt")
            git(repository, "commit", "-qm", "baseline")
            (repository / "selected.txt").write_text("after\n")
            (repository / "other.txt").write_text("still unreviewed\n")
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            with patch.object(dashboard, "_load", return_value=({"repositories": {"owner": {}}}, {}, {})), \
                    patch.object(dashboard, "_repo", return_value=repository):
                result = dashboard.commit_reviewed("owner", ["selected.txt"], "Review selected change")
            self.assertEqual(git(repository, "show", "HEAD:selected.txt"), "after")
            self.assertEqual(git(repository, "show", "HEAD:other.txt"), "before")
            self.assertEqual([item["path"] for item in result["remaining"]], ["other.txt"])

    def test_api_mutations_require_token_and_loopback_origin(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            server = ThreadingHTTPServer(("127.0.0.1", 0), web_server.handler_for(dashboard))
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            url = f"http://127.0.0.1:{server.server_port}/api/start"
            try:
                for headers in ({"Content-Type": "application/json"},
                                {"Content-Type": "application/json",
                                 "X-Axiom-Release-Token": dashboard.token,
                                 "Origin": "https://untrusted.example"}):
                    with self.assertRaises(HTTPError) as failure:
                        urlopen(Request(url, data=b"{}", headers=headers, method="POST"))
                    self.assertEqual(failure.exception.code, 403)
            finally:
                server.shutdown()
                server.server_close()
                thread.join()

    def test_run_builds_everything_before_first_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            job = {"trainId": "test-train", "status": "running", "completed": [],
                   "order": ["cli", "docs"], "error": None}
            order = []
            with patch.object(web_server.ctl, "read_catalog", return_value={}), \
                    patch.object(dashboard, "_create_cycle", side_effect=lambda *_: order.append("cycle")), \
                    patch.object(dashboard, "_prepare", side_effect=lambda *_: order.append("prepare")), \
                    patch.object(dashboard, "_commit_managed", side_effect=lambda *_: order.append("commit")), \
                    patch.object(dashboard, "_push_sources", side_effect=lambda *_: order.append("push")), \
                    patch.object(dashboard, "_build", side_effect=lambda _, target: order.append(f"build:{target}")), \
                    patch.object(dashboard, "_publish", side_effect=lambda _, target: order.append(f"publish:{target}")):
                dashboard._run(job)
            self.assertEqual(order, ["cycle", "prepare", "commit", "push", "build:cli",
                                     "build:docs", "publish:cli", "publish:docs"])
            self.assertEqual(job["status"], "complete")

    def test_build_failure_stops_before_any_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            job = {"trainId": "test-train", "status": "running", "completed": [],
                   "order": ["cli", "docs"], "error": None}
            with patch.object(web_server.ctl, "read_catalog", return_value={}), \
                    patch.object(dashboard, "_create_cycle"), \
                    patch.object(dashboard, "_prepare"), \
                    patch.object(dashboard, "_commit_managed"), \
                    patch.object(dashboard, "_push_sources"), \
                    patch.object(dashboard, "_build", side_effect=web_server.ctl.ReleaseError("build failed")), \
                    patch.object(dashboard, "_publish") as publish:
                dashboard._run(job)
            publish.assert_not_called()
            self.assertEqual(job["status"], "blocked")
            self.assertIn("build failed", job["error"])

    def test_preparation_uses_ledger_normalized_intent(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            job = {"trainId": "test-train", "intent": {"queued": [{"component": "cli"}]},
                   "ledger": {}}
            normalized = {"queued": [{"component": "cli", "version": "0.147.0"}]}
            report = {"blocked": [], "trainId": "test-train"}
            with patch.object(web_server.flow, "validate_intent", return_value=normalized), \
                    patch.object(web_server.flow, "make_preparation", return_value=(report, {}, {})) as prepare, \
                    patch.object(web_server.flow, "apply_preparation", return_value=Path(temporary) / "backup"):
                dashboard._prepare(job, {})
            self.assertEqual(prepare.call_args.args[0], normalized)
            self.assertEqual(job["intent"], {"queued": [{"component": "cli"}]})


if __name__ == "__main__":
    unittest.main()
