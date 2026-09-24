import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import ctl
import cycle


class ReleaseCycleTests(unittest.TestCase):
    def test_update_defaults_preserve_unpublished_work_and_skip_published(self):
        intent = {"trainId": "2026.09.24.5", "changes": [
            {"component": "landing", "type": "feature", "summary": "Published site."},
            {"component": "cli", "type": "breaking", "summary": "CLI change.",
             "migration": "Update scripts."}], "queued": [
                 {"component": "docs", "type": "fix", "summary": "Docs change."}]}
        catalog = {"components": [{"id": name} for name in ("cli", "docs", "landing")]}
        ledger = {"components": {"cli": {"candidateVersion": "0.147.0"},
                                 "docs": {"candidateVersion": None},
                                 "landing": {"candidateVersion": None}}}
        selected, answers = cycle.update_defaults(intent, ledger, catalog, {
            "landing": {"trainId": "2026.09.24.5"}})
        self.assertEqual(selected, ["cli"])
        self.assertEqual(answers["cli"], {
            "type": "breaking", "summaryMode": "write", "summary": "CLI change.",
            "version": "0.147.0", "migration": "Update scripts."})

    def test_update_refuses_unfinished_candidate_but_preserves_its_evidence(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            candidate = root / "trains/2026.09.24.5/components/landing"
            candidate.mkdir(parents=True)
            (candidate / "staged.json").write_text("staged")
            self.assertEqual(cycle.unfinished_candidate_paths(root, "2026.09.24.5", {}),
                             [candidate])
            self.assertEqual(cycle.unfinished_candidate_paths(root, "2026.09.24.5",
                             {"landing": {"trainId": "2026.09.24.5"}}), [])
            self.assertEqual((candidate / "staged.json").read_text(), "staged")

    def test_update_does_not_start_draft_while_candidate_is_unpublished(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            intent_path = root / "intent.json"
            ledger_path = root / "versions.json"
            intent = {"trainId": "2026.09.24.5", "changes": [
                {"component": "landing", "type": "feature", "summary": "Site."}], "queued": []}
            ledger = {"components": {"landing": {"candidateVersion": None}}}
            catalog = {"sha256": "catalog", "components": [{"id": "landing"}]}
            intent_path.write_text(json.dumps(intent))
            ledger_path.write_text(json.dumps(ledger))
            candidate = root / "trains/2026.09.24.5/components/landing"
            candidate.mkdir(parents=True)
            (candidate / "staged.json").write_text("staged")
            with self.assertRaisesRegex(ctl.ReleaseError, "unfinished candidate evidence"):
                cycle.run(catalog, ledger, intent, root, intent_path, ledger_path, update=True)
            self.assertEqual(json.loads(intent_path.read_text()), intent)
            self.assertFalse(cycle.draft_path(root, update=True).exists())

    def test_update_creates_successor_and_activates_queued_component(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            intent_path = root / "intent.json"
            versions_path = root / "versions.json"
            previous = {"format": "axiom-platform-release-intent/v1", "trainId": "2026.09.24.5",
                        "wave": "release", "summary": "Previous", "changes": [
                            {"component": "landing", "type": "feature", "summary": "Live site."}],
                        "queued": [{"component": "docs", "type": "fix", "summary": "Docs."}]}
            ledger = {"format": "axiom-platform-component-versions/v1", "components": {
                "landing": {"candidateVersion": None}, "docs": {"candidateVersion": None}}}
            catalog = {"sha256": "catalog-sha", "components": [
                {"id": "landing"}, {"id": "docs"}]}
            intent_path.write_text(json.dumps(previous))
            versions_path.write_text(json.dumps(ledger))
            change = {"component": "docs", "type": "fix", "summary": "Docs."}

            def choose(*args, **kwargs):
                self.assertEqual(kwargs["initial"], [])
                kwargs["on_change"](["docs"])
                return ["docs"]

            with patch.object(cycle, "choose_components", side_effect=choose), \
                    patch.object(cycle, "collect_changes", return_value=([change], ledger)), \
                    patch.object(cycle, "choose_summary_mode", return_value="template"), \
                    patch.object(cycle, "ask_line", return_value="yes"), \
                    patch.object(cycle, "next_train_id", return_value="2026.09.25.1"), \
                    patch.object(cycle, "published_evidence", return_value={
                        "landing": {"trainId": "2026.09.24.5"}}):
                self.assertTrue(cycle.run(catalog, ledger, previous, root, intent_path,
                                          versions_path, update=True))
            saved = json.loads(intent_path.read_text())
            self.assertEqual(saved["trainId"], "2026.09.25.1")
            self.assertEqual(saved["changes"], [change])
            self.assertEqual(saved["queued"], [])
            self.assertTrue((root / "trains/2026.09.25.1/cycle-backup/intent-before.json").is_file())
            self.assertFalse(cycle.draft_path(root, update=True).exists())

    def test_versioned_summary_template_and_explicit_custom_choice(self):
        self.assertIn("ui-host-web 0.6.7", cycle.component_summary_template("ui-host-web", "0.6.7"))
        self.assertIn("digest-based", cycle.component_summary_template("landing", None))
        with patch("builtins.input", side_effect=["t", "w"]):
            self.assertEqual(cycle.choose_summary_mode("ui-host-web", "Template"), "template")
            self.assertEqual(cycle.choose_summary_mode("landing", "Template"), "write")

    def test_version_suggestion_uses_verified_release_without_overriding_newer_candidate(self):
        self.assertEqual(cycle.suggested_version("0.6.7", None, "0.6.7", "fix"), "0.6.8")
        self.assertEqual(cycle.suggested_version("0.6.7", None, "0.6.7", "feature"), "0.7.0")
        self.assertEqual(cycle.suggested_version("0.8.0", None, "0.6.7", "feature"), "0.8.0")
        self.assertEqual(cycle.suggested_version("0.147.0", "0.146.0", None, "fix"), "0.147.0")

    def test_next_free_version_skips_shared_github_tag(self):
        catalog = {"components": [
            {"id": name, "version": "package.toml", "destination": "GitHub Releases: AxiomCore/AxiomCore"}
            for name in ("extractor-fastapi", "extractor-go")]}
        ledger = {"format": "axiom-platform-component-versions/v1", "components": {
            "extractor-fastapi": {"candidateVersion": "0.1.0"},
            "extractor-go": {"candidateVersion": None}}}
        self.assertEqual(cycle.next_free_version("0.1.0", "extractor-go", ledger, catalog), "0.1.1")

    def test_next_train_id_skips_prepared_and_staged_cycles(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            (root / "trains/2026.09.24.4").mkdir(parents=True)
            (root / "trains/2026.09.24.6").mkdir()
            self.assertEqual(cycle.next_train_id("2026.09.24.3", root, "2026.09.24"),
                             "2026.09.24.7")

    def test_selection_expands_hosts_and_dependencies(self):
        catalog = {"components": [
            {"id": "runtime-apple"}, {"id": "sdk-flutter", "depends_on": ["runtime-apple"]},
            *({"id": name} for name in cycle.HOST_IDS),
        ]}
        self.assertEqual(cycle.expand_selection({"sdk-flutter", "ui-host-web"}, catalog),
                         {"runtime-apple", "sdk-flutter", *cycle.HOST_IDS})

    def test_only_matching_remote_verified_publication_is_displayed(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            directory = root / "trains/2026.09.24.3/components/ui-host"
            (directory / "publication").mkdir(parents=True)
            intent = {"trainId": "2026.09.24.3", "changes": [
                {"component": "ui-host-web", "version": "0.6.7"}]}
            stage = {"status": "staged-not-published", "trainId": "2026.09.24.3"}
            (directory / "intent.json").write_text(json.dumps(intent))
            (directory / "staged.json").write_text(json.dumps(stage))
            record = {"format": "axiom-platform-component-publication/v1", "component": "ui-host",
                      "trainId": "2026.09.24.3", "status": "remote-verified",
                      "stageSha256": ctl.sha256(ctl.canonical(stage))}
            published = directory / "publication/published.json"
            published.write_text(json.dumps(record))
            catalog = {"components": [{"id": "ui-host-web"}]}
            self.assertEqual(cycle.published_evidence(root, catalog)["ui-host-web"]["version"], "0.6.7")
            self.assertEqual(cycle.published_evidence(root, catalog, "2026.09.24.4"), {})
            self.assertIn("ui-host-web", cycle.published_evidence(root, catalog, "2026.09.24.3"))
            record["stageSha256"] = "wrong"
            published.write_text(json.dumps(record))
            self.assertEqual(cycle.published_evidence(root, catalog), {})

    def test_new_intent_selects_landing_and_preserves_unfinished_queue(self):
        old = {"format": "axiom-platform-release-intent/v1", "trainId": "2026.09.24.3",
               "wave": "foundation", "summary": "Old", "changes": [
                   {"component": "docs", "type": "fix", "summary": "Old docs."}],
               "queued": [{"component": "landing", "type": "feature", "summary": "Old landing."}]}
        change = {"component": "landing", "type": "feature", "summary": "Deploy the Astro site."}
        result = cycle.compose_intent(old, "2026.09.24.4", [change], "Launch the site.", {})
        self.assertEqual(result["changes"], [change])
        self.assertEqual(result["queued"], old["changes"])
        self.assertEqual(result["wave"], "release")

    def test_save_backs_up_user_edits_and_refuses_concurrent_change(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            intent_path = root / "intent.json"
            versions_path = root / "versions.json"
            intent_path.write_text('{"trainId":"old"}\n')
            versions_path.write_text('{"candidateVersion":"1.0.0"}\n')
            old_intent = intent_path.read_bytes()
            old_versions = versions_path.read_bytes()
            intent_path.write_text('{"trainId":"changed"}\n')
            with self.assertRaisesRegex(ctl.ReleaseError, "changed while the wizard was open"):
                cycle.save_cycle(root, "2026.09.24.4", intent_path, versions_path,
                                 old_intent, old_versions, {"trainId": "new"}, {})
            self.assertFalse((root / "trains").exists())
            intent_path.write_bytes(old_intent)
            backup = cycle.save_cycle(root, "2026.09.24.4", intent_path, versions_path,
                                      old_intent, old_versions, {"trainId": "new"},
                                      {"candidateVersion": "1.0.1"})
            self.assertEqual((backup / "intent-before.json").read_bytes(), old_intent)
            self.assertEqual((backup / "versions-before.json").read_bytes(), old_versions)
            self.assertEqual(json.loads(intent_path.read_text())["trainId"], "new")
            checkpoint = cycle.draft_path(root)
            checkpoint.parent.mkdir(parents=True)
            checkpoint.write_text(json.dumps({"trainId": "2026.09.24.4"}))
            self.assertTrue(cycle.finish_saved_draft(checkpoint, root,
                                                     intent_path.read_bytes(), versions_path.read_bytes()))
            self.assertTrue((backup / "wizard-draft.json").is_file())
            with self.assertRaisesRegex(ctl.ReleaseError, "already has evidence"):
                cycle.save_cycle(root, "2026.09.24.4", intent_path, versions_path,
                                 intent_path.read_bytes(), versions_path.read_bytes(),
                                 {"trainId": "another"}, {})

    def test_wizard_creates_landing_cycle_without_changing_candidate_ledger(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            intent_path = root / "intent.json"
            versions_path = root / "versions.json"
            previous = {"format": "axiom-platform-release-intent/v1", "trainId": "2026.09.24.3",
                        "wave": "foundation", "summary": "Previous cycle", "changes": [
                            {"component": "docs", "type": "fix", "summary": "Docs update."}],
                        "queued": [{"component": "landing", "type": "feature",
                                    "summary": "Old landing summary."}]}
            ledger = {"format": "axiom-platform-component-versions/v1",
                      "components": {"docs": {"candidateVersion": None},
                                     "landing": {"candidateVersion": None}}}
            catalog = {"sha256": "catalog-sha", "components": [{"id": "docs"}, {"id": "landing"}]}
            intent_path.write_text(json.dumps(previous))
            versions_path.write_text(json.dumps(ledger))
            original_ledger = versions_path.read_bytes()
            change = {"component": "landing", "type": "feature", "summary": "Deploy Astro."}
            with patch.object(cycle, "choose_components", return_value=["landing"]), \
                    patch.object(cycle, "collect_changes", return_value=([change], ledger)), \
                    patch.object(cycle, "choose_summary_mode", return_value="write"), \
                    patch.object(cycle, "ask_line", side_effect=["Launch site", "yes"]), \
                    patch.object(cycle, "next_train_id", return_value="2026.09.24.4"), \
                    patch.object(cycle, "published_evidence", return_value={}):
                self.assertTrue(cycle.run(catalog, ledger, previous, root, intent_path, versions_path))
            saved = json.loads(intent_path.read_text())
            self.assertEqual(saved["trainId"], "2026.09.24.4")
            self.assertEqual(saved["changes"], [change])
            self.assertEqual(saved["queued"], previous["changes"])
            self.assertEqual(versions_path.read_bytes(), original_ledger)
            self.assertTrue((root / "trains/2026.09.24.4/cycle-backup/intent-before.json").is_file())
            self.assertTrue((root / "trains/2026.09.24.4/cycle-backup/wizard-draft.json").is_file())
            self.assertFalse(cycle.draft_path(root).exists())

    def test_interrupted_answer_resumes_without_reasking_saved_type(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            intent_path = root / "intent.json"
            versions_path = root / "versions.json"
            previous = {"format": "axiom-platform-release-intent/v1", "trainId": "2026.09.24.3",
                        "changes": [{"component": "docs", "type": "fix", "summary": "Old docs."}],
                        "queued": [{"component": "landing", "type": "feature", "summary": "Old landing."}]}
            ledger = {"format": "axiom-platform-component-versions/v1",
                      "components": {"docs": {"candidateVersion": None},
                                     "landing": {"candidateVersion": None}}}
            catalog = {"sha256": "catalog-sha", "components": [{"id": "docs"}, {"id": "landing"}]}
            intent_path.write_text(json.dumps(previous))
            versions_path.write_text(json.dumps(ledger))
            with patch.object(cycle, "choose_components", return_value=["landing"]), \
                    patch.object(cycle, "choose_type", return_value="feature"), \
                    patch.object(cycle, "choose_summary_mode", return_value="write"), \
                    patch.object(cycle, "ask_line", side_effect=KeyboardInterrupt), \
                    patch.object(cycle, "next_train_id", return_value="2026.09.24.4"):
                with self.assertRaises(KeyboardInterrupt):
                    cycle.run(catalog, ledger, previous, root, intent_path, versions_path)
            checkpoint = cycle.draft_path(root)
            self.assertEqual(json.loads(checkpoint.read_text())["answers"],
                             {"landing": {"type": "feature", "summaryMode": "write"}})
            with patch.object(cycle, "choose_components", side_effect=AssertionError("selection repeated")), \
                    patch.object(cycle, "choose_type", side_effect=AssertionError("type repeated")), \
                    patch.object(cycle, "choose_summary_mode", return_value="write"), \
                    patch.object(cycle, "ask_line", side_effect=["Deploy Astro", "Launch", "no"]):
                self.assertFalse(cycle.run(catalog, ledger, previous, root, intent_path, versions_path))
            resumed = json.loads(checkpoint.read_text())
            self.assertEqual(resumed["answers"]["landing"]["summary"], "Deploy Astro")
            self.assertEqual(resumed["summary"], "Launch")
            self.assertEqual(intent_path.read_text(), json.dumps(previous))

    def test_selection_is_checkpointed_before_confirmation(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            intent_path = root / "intent.json"
            versions_path = root / "versions.json"
            previous = {"trainId": "2026.09.24.3", "changes": [], "queued": []}
            ledger = {"components": {"landing": {"candidateVersion": None}}}
            catalog = {"sha256": "catalog-sha", "components": [{"id": "landing"}]}
            intent_path.write_text(json.dumps(previous))
            versions_path.write_text(json.dumps(ledger))

            def interrupted_picker(*args, **kwargs):
                kwargs["on_change"](["landing"])
                raise KeyboardInterrupt

            with patch.object(cycle, "choose_components", side_effect=interrupted_picker), \
                    patch.object(cycle, "next_train_id", return_value="2026.09.24.4"):
                with self.assertRaises(KeyboardInterrupt):
                    cycle.run(catalog, ledger, previous, root, intent_path, versions_path)
            saved = json.loads(cycle.draft_path(root).read_text())
            self.assertEqual(saved["selected"], ["landing"])
            self.assertFalse(saved["selectionConfirmed"])

    def test_duplicate_github_version_is_caught_at_second_prompt(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            selected = ["extractor-fastapi", "extractor-go"]
            catalog = {"components": [
                {"id": name, "version": "package.toml", "destination": "GitHub Releases: AxiomCore/AxiomCore"}
                for name in selected]}
            ledger = {"format": "axiom-platform-component-versions/v1",
                      "components": {name: {"candidateVersion": None} for name in selected}}
            previous = {"changes": [{"component": "extractor-fastapi", "type": "feature",
                                      "summary": "Old."}], "queued": []}
            draft = {"answers": {}}
            checkpoint = cycle.draft_path(root)
            with patch.object(cycle.ctl, "component_version", return_value="0.1.0"), \
                    patch.object(cycle, "choose_type", return_value="feature"), \
                    patch.object(cycle, "choose_summary_mode", return_value="write"), \
                    patch.object(cycle, "ask_line", side_effect=["0.1.0", "FastAPI", "0.1.0", "0.1.1", "Go"]):
                changes, updated = cycle.collect_changes(selected, catalog, ledger, previous,
                                                         {}, draft, checkpoint)
            self.assertEqual(len(changes), 2)
            self.assertEqual(updated["components"]["extractor-fastapi"]["candidateVersion"], "0.1.0")
            self.assertEqual(updated["components"]["extractor-go"]["candidateVersion"], "0.1.1")
            self.assertEqual(json.loads(checkpoint.read_text())["answers"]["extractor-go"]["version"], "0.1.1")

    def test_recovered_summary_can_be_replaced_with_versioned_template(self):
        with tempfile.TemporaryDirectory(prefix="axiom-cycle-test-") as temporary:
            root = Path(temporary)
            catalog = {"components": [{"id": "runtime-apple"}]}
            ledger = {"format": "axiom-platform-component-versions/v1",
                      "components": {"runtime-apple": {"candidateVersion": "0.148.0"}}}
            old = {"changes": [{"component": "runtime-apple", "type": "feature", "summary": "Old."}]}
            draft = {"answers": {"runtime-apple": {"summaryMode": "recovered", "summary": "From transcript"}}}
            with patch.object(cycle, "choose_type", return_value="feature"), \
                    patch.object(cycle, "choose_summary_mode", return_value="template"), \
                    patch.object(cycle, "ask_line", return_value="0.148.0"):
                changes, _ = cycle.collect_changes(["runtime-apple"], catalog, ledger, old,
                                                   {}, draft, cycle.draft_path(root))
            self.assertIn("runtime-apple 0.148.0", changes[0]["summary"])
            self.assertEqual(draft["answers"]["runtime-apple"]["summaryMode"], "template")


if __name__ == "__main__":
    unittest.main()
