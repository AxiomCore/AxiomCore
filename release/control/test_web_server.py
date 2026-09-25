"""Focused safety checks for the loopback release dashboard."""

from __future__ import annotations

from http.server import ThreadingHTTPServer
import io
import json
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen

import web_server
import auto_pins
import ci_builders
import dependency_baseline


def git(repository: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repository).decode().strip()


class ReleaseWebTests(unittest.TestCase):
    def test_suggested_version_advances_only_a_verified_published_version(self):
        self.assertEqual(web_server.suggested_version("0.147.0", "0.147.0", "0.147.0"), "0.147.1")
        self.assertEqual(web_server.suggested_version("0.148.0", "0.148.0", "0.147.0"), "0.148.0")
        self.assertEqual(web_server.suggested_version("0.147.0", "0.147.0", None), "0.147.0")

    def test_npm_latest_version_is_cached_and_uses_highest_stable_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(workspace=Path(temporary), root=Path(temporary))
            (Path(temporary) / "AxiomCore").mkdir()
            reply = subprocess.CompletedProcess([], 0, b'["0.146.0", "0.146.1-beta.1", "0.145.9"]', b"")
            with patch.object(web_server.subprocess, "run", return_value=reply) as query:
                self.assertEqual(dashboard._npm_latest_version("atmx-cli"), "0.146.0")
                self.assertEqual(dashboard._npm_latest_version("atmx-cli"), "0.146.0")
            query.assert_called_once()
            self.assertEqual(query.call_args.kwargs["env"]["NPM_CONFIG_USERCONFIG"], web_server.os.devnull)

    def test_pub_latest_version_is_cached_and_uses_highest_stable_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            reply = b'{"versions":[{"version":"0.146.0"},{"version":"0.146.1-dev.1"},{"version":"0.145.9"}]}'
            with patch.object(web_server.urllib.request, "urlopen", return_value=io.BytesIO(reply)) as query:
                self.assertEqual(dashboard._pub_latest_version("axiom_flutter_generator"), "0.146.0")
                self.assertEqual(dashboard._pub_latest_version("axiom_flutter_generator"), "0.146.0")
            query.assert_called_once()

    def test_npm_collision_replan_preserves_original_and_requires_confirmation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dashboard = web_server.ReleaseDashboard(root=root)
            train = "2026.09.25.4"
            job = {"format": "axiom-release-web-run/v1", "trainId": train,
                   "status": "blocked", "currentStep": "publish:sdk-atmx-cli", "completed": [],
                   "intent": {"changes": [{"component": "sdk-atmx-cli", "type": "feature",
                                            "summary": "CLI update in 0.146.0."}]},
                   "followup": {"trainId": "2026.09.25.5", "changes": [
                       {"component": "sdk-atmx-react", "summary": "React update", "type": "feature"}],
                       "order": ["sdk-atmx-react"], "requiredRepos": ["AxiomCore", "atmx-react"]}}
            web_server.atomic_json(dashboard._job_path(train), job)
            recovery = {"trainId": train, "component": "sdk-atmx-cli", "package": "atmx-cli",
                        "occupiedVersion": "0.146.0", "nextVersion": "0.146.1",
                        "followupTrainId": "2026.09.25.5"}
            catalog = {"components": [{"id": "sdk-atmx-cli", "owner": "AxiomCore", "sources": []},
                                      {"id": "sdk-atmx-react", "owner": "atmx-react", "sources": []}]}
            with patch.object(dashboard, "npm_collision_recovery", return_value=recovery), \
                    patch.object(dashboard, "_npm_latest_version", return_value="0.146.0"), \
                    patch.object(web_server.ctl, "read_catalog", return_value=catalog), \
                    patch.object(web_server.publish_targets, "_npm_metadata", return_value=None):
                with self.assertRaisesRegex(web_server.ctl.ReleaseError, "type REPLAN"):
                    dashboard.replan_npm_collision(train, "0.146.1", "CLI update in 0.146.1.", "wrong")
                self.assertEqual(json.loads(dashboard._job_path(train).read_text()), job)
                result = dashboard.replan_npm_collision(
                    train, "0.146.1", "CLI update in 0.146.1.",
                    f"REPLAN {train} sdk-atmx-cli 0.146.1")
            saved = json.loads(dashboard._job_path(train).read_text())
            backup = root / f"ui-runs/recovery-backups/{train}-sdk-atmx-cli.json"
            self.assertEqual(json.loads(backup.read_text()), job)
            self.assertEqual(saved["followup"]["versionOverrides"], {"sdk-atmx-cli": "0.146.1"})
            self.assertEqual(saved["followup"]["order"], ["sdk-atmx-cli", "sdk-atmx-react"])
            self.assertEqual(saved["followup"]["changes"][-1]["summary"], "CLI update in 0.146.1.")
            self.assertIn("publish:sdk-atmx-cli", saved["completed"])
            self.assertEqual(saved["deferredPublications"][0]["nextVersion"], "0.146.1")
            self.assertEqual(result["status"], "ready-to-resume")

    def test_resume_rejects_unchanged_occupied_npm_candidate(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            train = "2026.09.25.4"
            job = {"format": "axiom-release-web-run/v1", "trainId": train,
                   "status": "blocked", "currentStep": "publish:sdk-atmx-cli",
                   "completed": [], "intent": {"trainId": train}}
            web_server.atomic_json(dashboard._job_path(train), job)
            recovery = {"package": "atmx-cli", "occupiedVersion": "0.146.0"}
            with patch.object(dashboard, "npm_collision_recovery", return_value=recovery):
                with self.assertRaisesRegex(web_server.ctl.ReleaseError,
                                            "confirm 'Move package candidate to successor train' first"):
                    dashboard.resume(train, f"RESUME {train}")
            self.assertEqual(json.loads(dashboard._job_path(train).read_text()), job)

    def test_pub_collision_replan_defers_old_flutter_archive_too(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dashboard = web_server.ReleaseDashboard(root=root)
            train = "2026.09.25.4"
            job = {"format": "axiom-release-web-run/v1", "trainId": train,
                   "status": "blocked", "currentStep": "publish:sdk-flutter-generator",
                   "completed": [], "intent": {"changes": [
                       {"component": "sdk-flutter-generator", "type": "feature", "version": "0.146.0",
                        "summary": "Generator update in 0.146.0."},
                       {"component": "sdk-flutter", "type": "feature", "version": "0.147.0",
                        "summary": "Flutter update in 0.147.0."}]}}
            web_server.atomic_json(dashboard._job_path(train), job)
            recovery = {"trainId": train, "component": "sdk-flutter-generator",
                        "package": "axiom_flutter_generator", "occupiedVersion": "0.146.0",
                        "nextVersion": "0.146.1", "followupTrainId": "2026.09.25.5",
                        "rebuildWithSuccessor": ["sdk-flutter"]}
            catalog = {"components": [
                {"id": "sdk-flutter-generator", "owner": "axiom-sdk",
                 "sources": [{"repo": "axiom-sdk", "paths": ["flutter/axiom_flutter_generator/"]}]},
                {"id": "sdk-flutter", "owner": "axiom-sdk",
                 "sources": [{"repo": "axiom-sdk", "paths": ["flutter/axiom_flutter/"]}]}]}
            with patch.object(dashboard, "pub_collision_recovery", return_value=recovery), \
                    patch.object(web_server.ctl, "read_catalog", return_value=catalog), \
                    patch.object(web_server.ctl, "component_version", return_value="0.146.0"), \
                    patch.object(web_server.publish_targets, "_pub_metadata", return_value=None):
                result = dashboard.replan_pub_collision(
                    train, "0.146.1", "Generator update in 0.146.1.",
                    f"REPLAN {train} sdk-flutter-generator 0.146.1")
            saved = json.loads(dashboard._job_path(train).read_text())
            self.assertEqual(result["status"], "ready-to-resume")
            self.assertEqual(saved["followup"]["versionOverrides"],
                             {"sdk-flutter-generator": "0.146.1"})
            self.assertEqual(saved["followup"]["order"], ["sdk-flutter-generator", "sdk-flutter"])
            self.assertEqual(set(saved["completed"]),
                             {"publish:sdk-flutter-generator", "publish:sdk-flutter"})
            self.assertEqual(len(saved["deferredPublications"]), 2)
            self.assertEqual(json.loads((root / f"ui-runs/recovery-backups/{train}-sdk-flutter-generator.json").read_text()), job)

    def test_followup_applies_replanned_npm_version_only_to_successor_ledger(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            control = root / "control"
            control.mkdir()
            original_intent = {"trainId": "2026.09.25.4"}
            original_ledger = {"components": {"sdk-atmx-cli": {"candidateVersion": "0.146.0"}}}
            (control / "intent.json").write_text(json.dumps(original_intent))
            (control / "versions.json").write_text(json.dumps(original_ledger))
            dashboard = web_server.ReleaseDashboard(root=root)
            job = {"trainId": "2026.09.25.4", "intent": original_intent,
                   "ledger": original_ledger, "followup": {
                       "trainId": "2026.09.25.5", "changes": [{"component": "sdk-atmx-cli"}],
                       "versionOverrides": {"sdk-atmx-cli": "0.146.1"}}}
            catalog = {"components": [{"id": "sdk-atmx-cli", "owner": "AxiomCore", "sources": []}]}
            with patch.object(web_server.ctl, "CONTROL_DIR", control), \
                    patch.object(web_server.versions, "VERSIONS", control / "versions.json"), \
                    patch.object(web_server.versions, "validate_versions"), \
                    patch.object(web_server.cycle, "published_evidence", return_value={}), \
                    patch.object(web_server.cycle, "compose_intent", return_value={}), \
                    patch.object(web_server.flow, "validate_intent", return_value={}), \
                    patch.object(web_server.flow, "make_preparation", return_value=({"blocked": []}, {}, {})):
                dashboard._plan_followup(job, catalog)
            self.assertEqual(job["ledger"]["components"]["sdk-atmx-cli"]["candidateVersion"], "0.146.0")
            self.assertEqual(job["followup"]["ledger"]["components"]["sdk-atmx-cli"]["candidateVersion"], "0.146.1")

    def test_replanned_npm_publish_skips_old_train_and_runs_successor(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            job = {"trainId": "old", "status": "running", "completed": ["publish:sdk-atmx-cli"],
                   "order": ["sdk-atmx-cli", "docs"], "error": None,
                   "deferredPublications": [{"component": "sdk-atmx-cli", "nextVersion": "0.146.1"}],
                   "followup": {"trainId": "new", "changes": [{"component": "sdk-atmx-cli"}],
                                "order": ["sdk-atmx-cli"]}}
            calls = []
            with patch.object(web_server.ctl, "read_catalog", return_value={}), \
                    patch.object(dashboard, "_create_cycle"), \
                    patch.object(dashboard, "_prepare"), \
                    patch.object(dashboard, "_commit_managed"), \
                    patch.object(dashboard, "_push_sources"), \
                    patch.object(dashboard, "_build"), \
                    patch.object(dashboard, "_pin_followup"), \
                    patch.object(dashboard, "_plan_followup"), \
                    patch.object(dashboard, "_publish", side_effect=lambda active, target:
                                 calls.append((active.get("phase", "primary"), target))):
                dashboard._run(job)
            self.assertEqual(calls, [("primary", "docs"), ("followup", "sdk-atmx-cli")])
            self.assertEqual(job["status"], "complete")

    def test_swift_pin_parser_accepts_checksum_on_separate_line(self):
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary) / "axiom-sdk/swift/Package.swift"
            package.parent.mkdir(parents=True)
            package.write_text('url: "https://github.com/AxiomCore/AxiomCore/releases/download/v0.148.0/'
                               'AxiomRuntime.xcframework.zip",\n'
                               '// checksum is required\n'
                               f'checksum: "{"a" * 64}"\n')
            ledger = {"components": {"runtime-apple": {"candidateVersion": "0.148.0"}}}
            self.assertEqual(ci_builders.dependency_blockers({"sdk-swift"}, ledger, Path(temporary)), [])

    def test_auto_pin_swift_uses_only_verified_checksum_and_saves_original(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "axiom-sdk/swift/Package.swift"
            package.parent.mkdir(parents=True)
            original = ('url: "https://example.test/releases/download/v0.146.0/'
                        'AxiomRuntime.xcframework.zip",\n'
                        f'checksum: "{"a" * 64}"\n')
            package.write_text(original)
            proof = root / "trains/test-train/components/runtime-apple/publication/published.json"
            proof.parent.mkdir(parents=True)
            proof.write_text(json.dumps({"details": {"files": {
                "AxiomRuntime.xcframework.zip": "b" * 64}}}))
            ledger = {"components": {"runtime-apple": {"candidateVersion": "0.148.0"}}}
            with patch.object(auto_pins.cycle, "published_evidence", return_value={
                "runtime-apple": {"version": "0.148.0", "record": str(proof)}}), \
                    patch.object(auto_pins.ctl, "repo_path", return_value=root / "axiom-sdk"):
                changed = auto_pins.pin_verified_consumers(root, root, {}, "test-train", ledger,
                                                           {"sdk-swift"})
            self.assertEqual(changed, {"axiom-sdk": ["swift/Package.swift"]})
            self.assertIn("v0.148.0/AxiomRuntime", package.read_text())
            self.assertIn('checksum: "' + "b" * 64 + '"', package.read_text())
            self.assertEqual((root / "trains/test-train/auto-pins/axiom-sdk/swift/Package.swift").read_text(), original)

    def test_followup_reuses_only_a_remotely_published_dependency_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "trains/producer/components/sdk-atmx-web"
            directory.mkdir(parents=True)
            fingerprint = "f" * 64
            artifact = {"file": "atmx-web.tgz", "sha256": "a" * 64, "path": str(root / "artifacts/atmx-web.tgz")}
            plan = {"catalogSha256": "catalog", "repositories": {"atmx-web": {"head": "h"}},
                    "components": [{"id": "sdk-atmx-web", "fingerprint": fingerprint}]}
            stage = {"catalogSha256": "catalog", "components": [{"id": "sdk-atmx-web",
                     "fingerprint": fingerprint, "version": "0.147.0",
                     "artifacts": [{"file": artifact["file"], "sha256": artifact["sha256"]}]}]}
            (directory / "plan.json").write_text(json.dumps(plan))
            (directory / "staged.json").write_text(json.dumps(stage))
            catalog = {"sha256": "catalog", "components": [
                {"id": "sdk-atmx-react", "depends_on": ["sdk-atmx-web"]},
                {"id": "sdk-atmx-web", "depends_on": []}]}
            proof = {"sdk-atmx-web": {"trainId": "producer", "version": "0.147.0"}}
            receipt = {"component": "sdk-atmx-web", "artifacts": [artifact]}
            with patch.object(dependency_baseline.cycle, "published_evidence", return_value=proof), \
                    patch.object(dependency_baseline.ctl, "verify_receipt", return_value=receipt):
                baseline = dependency_baseline.published_dependency_baseline(
                    root, catalog, root, {"sdk-atmx-react"}, {"sdk-atmx-react"})
            self.assertEqual(baseline["sdk-atmx-web"]["fingerprint"], fingerprint)
            self.assertEqual(baseline["sdk-atmx-web"]["artifacts"], [artifact])
            self.assertEqual(dependency_baseline.receipt_for(root, {
                "reuseCandidate": True, "reusedReceiptPath": baseline["sdk-atmx-web"]["receiptPath"]}, {}),
                Path(baseline["sdk-atmx-web"]["receiptPath"]))

    def test_unpublished_dependency_pin_consumers_are_deferred_not_dropped(self):
        changes = [{"component": "runtime-apple", "summary": "Runtime"},
                   {"component": "sdk-swift", "summary": "Swift"},
                   {"component": "sdk-atmx-react", "summary": "React"}]
        with patch.object(web_server.ci_builders, "dependency_blockers",
                          side_effect=lambda selected, *_: ["pin unavailable"] if selected != {"runtime-apple"} else []):
            ready, deferred = web_server.split_deferred_changes(changes, {}, Path("/unused"))
        self.assertEqual([item["component"] for item in ready], ["runtime-apple"])
        self.assertEqual([item["id"] for item in deferred], ["sdk-swift", "sdk-atmx-react"])
        self.assertEqual([item["change"]["summary"] for item in deferred], ["Swift", "React"])

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

    def test_automatic_pin_phase_runs_only_after_all_primary_publications(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            job = {"trainId": "test-train", "status": "running", "completed": [],
                   "order": ["runtime-apple", "sdk-atmx-web"], "error": None,
                   "followup": {"trainId": "test-next", "changes": [
                       {"component": "sdk-swift"}, {"component": "sdk-atmx-react"}],
                       "order": ["sdk-swift", "sdk-atmx-react"]}}
            order = []
            with patch.object(web_server.ctl, "read_catalog", return_value={}), \
                    patch.object(dashboard, "_create_cycle", side_effect=lambda *_: order.append("cycle")), \
                    patch.object(dashboard, "_prepare", side_effect=lambda *_: order.append("prepare")), \
                    patch.object(dashboard, "_commit_managed", side_effect=lambda *_: order.append("commit")), \
                    patch.object(dashboard, "_push_sources", side_effect=lambda *_: order.append("push")), \
                    patch.object(dashboard, "_build", side_effect=lambda _, target: order.append(f"build:{target}")), \
                    patch.object(dashboard, "_publish", side_effect=lambda _, target: order.append(f"publish:{target}")), \
                    patch.object(dashboard, "_pin_followup", side_effect=lambda *_: order.append("pins")), \
                    patch.object(dashboard, "_plan_followup", side_effect=lambda *_: order.append("plan")):
                dashboard._run(job)
            self.assertEqual(order, ["cycle", "prepare", "commit", "push", "build:runtime-apple",
                                     "build:sdk-atmx-web", "publish:runtime-apple", "publish:sdk-atmx-web",
                                     "pins", "plan", "cycle", "prepare", "commit", "push",
                                     "build:sdk-swift", "build:sdk-atmx-react",
                                     "publish:sdk-swift", "publish:sdk-atmx-react"])
            self.assertEqual(job["status"], "complete")

    def test_failed_producer_publication_never_edits_consumer_pins(self):
        with tempfile.TemporaryDirectory() as temporary:
            dashboard = web_server.ReleaseDashboard(root=Path(temporary))
            job = {"trainId": "test-train", "status": "running", "completed": [],
                   "order": ["runtime-apple"], "error": None,
                   "followup": {"trainId": "test-next", "changes": [{"component": "sdk-swift"}],
                                "order": ["sdk-swift"]}}
            with patch.object(web_server.ctl, "read_catalog", return_value={}), \
                    patch.object(dashboard, "_create_cycle"), \
                    patch.object(dashboard, "_prepare"), \
                    patch.object(dashboard, "_commit_managed"), \
                    patch.object(dashboard, "_push_sources"), \
                    patch.object(dashboard, "_build"), \
                    patch.object(dashboard, "_publish", side_effect=web_server.ctl.ReleaseError("remote verification failed")), \
                    patch.object(dashboard, "_pin_followup") as pin:
                dashboard._run(job)
            pin.assert_not_called()
            self.assertEqual(job["status"], "blocked")

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
