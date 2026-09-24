from pathlib import Path
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import review


def git(repository: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repository, text=True,
                                   stderr=subprocess.DEVNULL).strip()


class ReviewTests(unittest.TestCase):
    def test_status_parser_skips_rename_source_record(self):
        files = review.parse_status(b"R  renamed.txt\0old.txt\0?? note.json\0")
        self.assertEqual([(item.path, item.status) for item in files],
                         [("note.json", "??"), ("renamed.txt", "R ")])
        self.assertEqual(files[1].previous_path, "old.txt")

    def test_explicit_commit_keeps_other_staged_files_out(self):
        with tempfile.TemporaryDirectory(prefix="axiom-review-test-") as temporary:
            workspace = Path(temporary)
            repository = workspace / "owner"
            repository.mkdir()
            git(repository, "init", "-q")
            git(repository, "config", "user.name", "Release Test")
            git(repository, "config", "user.email", "release@example.test")
            (repository / "one.txt").write_text("before\n")
            (repository / "other.txt").write_text("before\n")
            git(repository, "add", "--", "one.txt", "other.txt")
            git(repository, "commit", "-qm", "Initial")
            (repository / "one.txt").write_text("after\n")
            (repository / "other.txt").write_text("other change\n")
            git(repository, "add", "--", "other.txt")
            catalog = {"repositories": {"owner": "owner"}}

            before = review.changed_repositories(catalog, workspace)
            self.assertEqual(len(before), 1)
            self.assertEqual({item.path for item in before[0].files}, {"one.txt", "other.txt"})
            review.commit_selected(repository, ("one.txt",), "Commit selected change")

            self.assertEqual(git(repository, "show", "--format=", "--name-only", "HEAD"), "one.txt")
            remaining = review.changed_repositories(catalog, workspace)
            self.assertEqual([item.path for item in remaining[0].files], ["other.txt"])
            self.assertEqual(git(repository, "diff", "--cached", "--name-only"), "other.txt")

    def test_selected_rename_commits_old_and_new_paths(self):
        with tempfile.TemporaryDirectory(prefix="axiom-review-test-") as temporary:
            repository = Path(temporary) / "owner"
            repository.mkdir()
            git(repository, "init", "-q")
            git(repository, "config", "user.name", "Release Test")
            git(repository, "config", "user.email", "release@example.test")
            (repository / "old.txt").write_text("content\n")
            git(repository, "add", "--", "old.txt")
            git(repository, "commit", "-qm", "Initial")
            git(repository, "mv", "old.txt", "new.txt")

            review.commit_selected(repository, ("new.txt",), "Rename selected file")

            self.assertEqual(git(repository, "status", "--porcelain"), "")
            self.assertEqual(git(repository, "show", "--format=", "--name-status", "HEAD"),
                             "R100\told.txt\tnew.txt")

    def test_clean_repositories_proceed_without_terminal(self):
        with tempfile.TemporaryDirectory(prefix="axiom-review-test-") as temporary:
            workspace = Path(temporary)
            repository = workspace / "owner"
            repository.mkdir()
            git(repository, "init", "-q")
            git(repository, "config", "user.name", "Release Test")
            git(repository, "config", "user.email", "release@example.test")
            (repository / "README.md").write_text("ready\n")
            git(repository, "add", "--", "README.md")
            git(repository, "commit", "-qm", "Initial")
            catalog = {"repositories": {"owner": "owner"}}

            with patch.object(review.sys.stdin, "isatty", return_value=False):
                self.assertEqual(review.run(catalog, "train.2", workspace), "proceed")
            (repository / "note.txt").write_text("pending\n")
            with patch.object(review.sys.stdin, "isatty", return_value=False):
                self.assertEqual(review.run(catalog, "train.2", workspace), "needs_tty")
            with patch.object(review.sys.stdin, "isatty", return_value=True), \
                    patch.object(review.sys.stdout, "isatty", return_value=True), \
                    patch.dict(os.environ, {"TERM": "dumb"}):
                self.assertEqual(review.run(catalog, "train.2", workspace), "needs_tty")
            self.assertTrue((repository / "note.txt").exists())

    def test_aggregate_workspace_ignores_untracked_sibling_repositories(self):
        with tempfile.TemporaryDirectory(prefix="axiom-review-test-") as temporary:
            workspace = Path(temporary)
            git(workspace, "init", "-q")
            git(workspace, "config", "user.name", "Release Test")
            git(workspace, "config", "user.email", "release@example.test")
            source = workspace / "acore-diff/src/lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text("old\n")
            git(workspace, "add", "--", "acore-diff/src/lib.rs")
            git(workspace, "commit", "-qm", "Initial")
            (workspace / "sibling-repo").mkdir()
            (workspace / "sibling-repo/untracked.txt").write_text("sibling\n")
            catalog = {"repositories": {"acore-diff": "."}, "components": [
                {"sources": [{"repo": "acore-diff", "paths": ["acore-diff/src/"]}]}]}

            self.assertEqual(review.changed_repositories(catalog, workspace), ())
            source.write_text("new\n")
            result = review.changed_repositories(catalog, workspace)
            self.assertEqual([item.path for item in result[0].files], ["acore-diff/src/lib.rs"])


if __name__ == "__main__":
    unittest.main()
