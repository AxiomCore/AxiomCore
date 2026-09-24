from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import ctl
import release_cli


class CandidatePreflightTests(unittest.TestCase):
    def test_dashboard_launch_uses_pinned_infisical_project(self):
        with patch.object(release_cli.shutil, "which", return_value="/usr/bin/infisical"), \
                patch.object(release_cli.os, "execvpe", side_effect=RuntimeError("exec captured")) as execute:
            with self.assertRaisesRegex(RuntimeError, "exec captured"):
                release_cli.web_with_production_environment()
        command = execute.call_args.args[1]
        environment = execute.call_args.args[2]
        self.assertEqual(command[:3], ["infisical", "run", "--env=prod"])
        self.assertTrue(any(item.startswith("--projectId=") for item in command))
        self.assertEqual(environment["AXIOM_RELEASE_INFISICAL_READY"], "1")

    def test_status_marks_only_current_train_remote_verified_components_published(self):
        catalog = {"components": [{"id": "landing"}, {"id": "docs"}]}
        ledger = {"components": {name: {"candidateVersion": None}
                                 for name in ("landing", "docs")}}
        intent = {"trainId": "2026.09.24.5", "wave": "release",
                  "changes": [{"component": "landing"}],
                  "queued": [{"component": "docs"}]}
        with tempfile.TemporaryDirectory(prefix="axiom-status-test-") as temporary:
            output = io.StringIO()
            with patch.dict(os.environ, {"AXIOM_RELEASE_BUILD_ROOT": temporary}), \
                    patch.object(release_cli.scan, "inventory",
                                 return_value={"affected": ["landing", "docs"]}), \
                    patch.object(release_cli.cycle, "published_evidence",
                                 return_value={"landing": {"trainId": "2026.09.24.5"}}) as evidence, \
                    patch.object(release_cli.train, "status", return_value={
                        "evidenceRoot": temporary, "storageAvailable": True}), \
                    patch.object(release_cli.flow, "make_preparation",
                                 return_value=({"blocked": []}, [], [])), \
                    redirect_stdout(output):
                release_cli.status(catalog, ledger, intent)
            evidence.assert_called_once_with(Path(temporary).resolve(), catalog,
                                             train_id="2026.09.24.5")
            self.assertIn("landing                published", output.getvalue())
            self.assertIn("docs                   queued", output.getvalue())
            self.assertIn("Published in this train: 1/1", output.getvalue())

    def test_queued_only_additions_preserve_prepared_active_wave(self):
        previous = {"format": "axiom-platform-release-intent/v1", "trainId": "train.2",
                    "changes": [{"component": "docs", "type": "fix", "summary": "Fix docs."}],
                    "queued": [{"component": "dashboard-origin", "type": "internal",
                                "summary": "Follow-up."}]}
        current = json.loads(json.dumps(previous))
        current["queued"].insert(0, {"component": "landing", "type": "feature",
                                     "summary": "Deploy landing."})
        catalog = {"components": [{"id": name} for name in ("docs", "dashboard-origin", "landing")]}
        ledger = {"components": {name: {"candidateVersion": None}
                                 for name in ("docs", "dashboard-origin", "landing")}}
        evidence = {"applied": True, "trainId": "train.2",
                    "intentSha256": ctl.sha256(ctl.canonical(previous))}

        def git_history(*args, **kwargs):
            return b"revision\n" if args[1] == "log" else json.dumps(previous).encode()

        with patch.object(release_cli.ctl, "run", side_effect=git_history):
            self.assertTrue(release_cli.prepared_intent_matches(evidence, current, catalog, ledger))
            current["changes"][0]["summary"] = "Changed after preparation."
            self.assertFalse(release_cli.prepared_intent_matches(evidence, current, catalog, ledger))

    def test_clean_prepared_train_reports_ci_gap_without_building(self):
        intent = {"trainId": "train.2", "changes": [{"component": "sdk", "type": "fix",
                                                  "summary": "Update SDK.", "version": "1.0.1"}]}
        plan = {"blocked": [], "blockedVersions": [], "components": [
            {"id": "sdk", "selected": True, "adapter": "ci-only"}]}
        with tempfile.TemporaryDirectory(prefix="axiom-preflight-test-") as temporary:
            root = Path(temporary)
            evidence = root / "trains/train.2/preparation.json"
            evidence.parent.mkdir(parents=True)
            evidence.write_text(json.dumps({"applied": True, "trainId": "train.2",
                                            "intentSha256": ctl.sha256(ctl.canonical(intent))}))
            output = io.StringIO()
            with patch.dict(os.environ, {"AXIOM_RELEASE_BUILD_ROOT": str(root)}), \
                    patch.object(release_cli.ctl, "make_plan", return_value=plan), \
                    redirect_stdout(output):
                release_cli.candidate_preflight({}, intent)
            self.assertIn("Local candidate preflight passed", output.getvalue())
            self.assertIn("CI builder gap: sdk", output.getvalue())
            self.assertIn("No candidate was built or published", output.getvalue())

    def test_matching_interrupted_candidate_can_resume_without_overwriting(self):
        scoped = {"trainId": "train.2", "changes": [{"component": "ui-host-web"}]}
        plan = {"components": [{"id": "ui-host-web", "selected": True}]}
        with tempfile.TemporaryDirectory(prefix="axiom-candidate-test-") as temporary:
            directory = Path(temporary) / "candidate"
            release_cli.open_candidate_directory(directory, scoped, plan)
            (directory / "build-ui-host-web-1.log").write_text("interrupted attempt\n")
            original = (directory / "plan.json").read_bytes()
            with redirect_stdout(io.StringIO()):
                release_cli.open_candidate_directory(directory, scoped, plan)
            self.assertEqual((directory / "plan.json").read_bytes(), original)
            with self.assertRaisesRegex(ctl.ReleaseError, "source plan changed"):
                release_cli.open_candidate_directory(directory, scoped, {"components": []})
            (directory / "staged.json").write_text("already staged\n")
            with self.assertRaisesRegex(ctl.ReleaseError, "not an incomplete build"):
                release_cli.open_candidate_directory(directory, scoped, plan)


if __name__ == "__main__":
    unittest.main()
