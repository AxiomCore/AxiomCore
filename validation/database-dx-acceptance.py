#!/usr/bin/env python3
"""Exercise the installed database CLI on disposable SQLite targets and a PTY.

This is a maintainer acceptance test. Application developers only need axiom.
No existing project or database is changed. The report contains no secrets.
"""
import argparse
import errno
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    cli = args.cli.resolve()
    results = []
    with tempfile.TemporaryDirectory(prefix="axiom-database-dx-") as directory:
        work = Path(directory)
        project = work / "project"
        project.mkdir()
        manifest = 'type = "database"\ntargets = ["server"]\n'
        (project / "AxiomDeps.toml").write_text(manifest)
        source = project / "schema.acore"
        source.write_text(
            'project(id: "acceptance.dx.sqlite", version: "1.0.0")\n'
            "module acceptance.dx.sqlite\n"
            "database tasks(engine: DatabaseEngine.sqlite) {\n"
            "  manage(schemas: [Schemas.main], mode: ManagementMode.ownedObjects)\n"
            "  schema main { table tasks(strict: true) {\n"
            "    id: Sqlite.Text()\n    title: Sqlite.Text()\n"
            "    primaryKey(columns: [.id])\n  } }\n}\n"
        )
        file = project / "target.sqlite"
        env = os.environ.copy()
        for name in ["DATABASE_URL", "PGHOST", "PGPORT", "PGDATABASE", "PGUSER",
                     "PGPASSWORD", "PG_BIN", "AXIOM_DATABASE_POSTGRES_BIN",
                     "AXIOM_DATABASE_ENGINE_DIR", "AXIOM_DATABASE_TRUSTED_KEY"]:
            env.pop(name, None)
        env["AXIOM_DATABASE_KEY_DIR"] = str(work / "keys")
        env["CI"] = "true"

        def record(name):
            results.append({"scenario": name, "result": "passed"})
            print("PASS", name, flush=True)

        def run(*tokens, expected=0, root=project, input=None, extra_env=None):
            process = subprocess.run(
                [str(cli), *map(str, tokens)], cwd=root,
                env={**env, **(extra_env or {})}, input=input,
                capture_output=True, text=True, timeout=90,
            )
            assert process.returncode == expected, (
                tokens, process.returncode, process.stdout[-4000:], process.stderr[-4000:]
            )
            return process

        def migrate(*tokens, **options):
            return run("database", "migrate", "schema.acore",
                       "--database-file", file, "--non-interactive", *tokens, **options)

        def status(**options):
            process = run("database", "status", "schema.acore",
                          "--database-file", file, "--json", **options)
            return json.loads(process.stdout)

        def versions():
            return [r["revision"]["identity"]["version"] for r in status()["releases"]]

        def rows():
            with sqlite3.connect(file) as db:
                return db.execute("SELECT title FROM tasks").fetchall()

        def columns():
            with sqlite3.connect(file) as db:
                return [r[1] for r in db.execute("PRAGMA table_info(tasks)")]

        help_text = run("database", "--help").stdout
        assert all(command in help_text for command in
                   ["migrate", "rollback", "status", "history", "diff"])
        assert "\n  release " not in help_text and "\n  apply " not in help_text
        record("limited public database command surface")

        before = source.read_text()
        preview = migrate("--dry-run", "--json")
        assert json.loads(preview.stdout)["targetCreated"] is False
        assert not file.exists() and source.read_text() == before
        assert not (project / ".axiom/database/migrations").exists()
        run("database", "migrate", "schema.acore", "--database-url",
            "postgresql://not-a-real-user@127.0.0.1:1/unused",
            "--non-interactive", expected=1)
        assert not file.exists()
        record("dry run and wrong engine do not create a target")

        run("build", "schema.acore")
        artifact = project / ".axiom/contracts/database.axiom"
        assert artifact.is_file() and artifact.with_suffix(".sources.axiom").is_file()
        run("run", artifact, "--database-file", file, "--no-shell",
            "--execute", "INSERT INTO tasks(id,title) VALUES('one','retained');",
            "--non-interactive")
        assert rows() == [("retained",)] and versions() == ["1.0.0"]
        record("source capsule artifact run migrates and executes native SQL")

        source.write_text("// Comment only\n" + source.read_text())
        migrate("--auto-version")
        assert versions() == ["1.0.0"]
        assert 'version: "1.0.0"' in source.read_text()
        record("unchanged content and comments do not publish another version")

        source.write_text(source.read_text().replace(
            "title: Sqlite.Text()", "title: Sqlite.Text()\n    note: Sqlite.Text(nullable: true)"))
        before = source.read_text()
        conflict = migrate(expected=1)
        assert "ADB1103" in conflict.stderr and "1.0.1" in conflict.stderr
        assert source.read_text() == before and columns() == ["id", "title"]
        bundle = work / "release.axiom"
        migrate("--auto-version", "--export", bundle)
        assert 'version: "1.0.1"' in source.read_text()
        assert versions() == ["1.0.0", "1.0.1"] and rows() == [("retained",)]
        migrate("--auto-version")
        assert versions() == ["1.0.0", "1.0.1"]
        record("strict conflict, CI auto-version and safe populated addition")

        result = status()
        assert result["format"] == "axiom-database-status/v1"
        assert result["pending"] is False and result["drift"] is False
        missing = project / "missing.sqlite"
        run("database", "status", "schema.acore", "--database-file", missing,
            "--json", expected=1)
        assert not missing.exists()
        run("database", "diff", "schema.acore", "--from", "1.0.0",
            "--to", "1.0.1")
        record("read-only status, history and arbitrary archived comparison")

        for name in ["staging", "production"]:
            target = work / (name + ".sqlite")
            run("database", "migrate", bundle, "--database-file", target,
                "--frozen", "--non-interactive")
            result = run("database", "status", bundle, "--database-file", target,
                         "--check", "--json")
            assert [r["revision"]["identity"]["version"]
                    for r in json.loads(result.stdout)["releases"]] == ["1.0.0", "1.0.1"]
        record("one immutable prefix promoted to two fresh targets")

        trusted = migrate(extra_env={"AXIOM_DATABASE_TRUSTED_KEY": "untrusted"}, expected=1)
        assert "ADB1109" in trusted.stderr and versions()[-1] == "1.0.1"
        source.write_text(source.read_text().replace(
            "note: Sqlite.Text(nullable: true)",
            "note: Sqlite.Text(nullable: true)\n    requiredValue: Sqlite.Text()"))
        before = source.read_text()
        failure = migrate("--auto-version", "--allow-data-dependent", expected=1)
        assert "ADB1112" in failure.stderr
        assert source.read_text() == before and "requiredValue" not in columns()
        assert any(a["outcome"] == "validation-failed" for a in status()["attempts"])
        source.write_text(source.read_text().replace("\n    requiredValue: Sqlite.Text()", ""))
        record("trust mismatch and missing backfill refuse mutation and track failure")

        before = source.read_text()
        run("database", "rollback", "schema.acore", "--database-file", file,
            "--steps", "1", "--auto-version", "--non-interactive", expected=1)
        assert source.read_text() == before and "note" in columns()
        run("database", "rollback", "schema.acore", "--database-file", file,
            "--steps", "1", "--auto-version", "--non-interactive", "--allow-destructive")
        assert versions() == ["1.0.0", "1.0.1", "2.0.0"]
        assert source.read_text() == before and rows() == [("retained",)]
        run("database", "rollback", "schema.acore", "--database-file", file,
            "--to", "1.0.0", "--non-interactive", "--json")
        assert versions() == ["1.0.0", "1.0.1", "2.0.0"]
        record("separate destructive authorization and immutable compensation")

        valid = source.read_text()
        source.write_text(valid + "\nthis is an unfinished draft ???\n")
        result = status()
        assert result["workingVersion"] == "invalid" and result["pending"] is True
        run("database", "history", "schema.acore", "--database-file", file)
        run("database", "diff", "schema.acore", "--from", "1.0.0", "--to", "1.0.1")
        source.write_text(valid)
        migrate("--auto-version")
        assert versions()[-1] == "2.0.1" and rows() == [("retained",)]
        record("invalid draft inspection and forward reapply preserve accepted state")

        fresh = work / "fresh-checkout"
        fresh.mkdir()
        (fresh / "AxiomDeps.toml").write_text(manifest)
        (fresh / "schema.acore").write_text(source.read_text())
        run("database", "history", "schema.acore", "--database-file", file, root=fresh)
        assert (fresh / ".axiom/database/migrations/1.0.0/release.axiom").is_file()
        run("database", "diff", "schema.acore", "--from", "1.0.0",
            "--to", "2.0.1", root=fresh)
        record("fresh checkout recovers verified definitions from the target ledger")

        before = source.read_text()
        after = before.replace('version: "2.0.1"', 'version: "2.0.2"')
        journal = project / ".axiom/database/source-edit.json"
        journal.write_text(json.dumps({"format": "axiom-source-edit/v1",
                                      "before": {"schema.acore": before},
                                      "after": {"schema.acore": after}}))
        migrate()
        assert source.read_text() == after and not journal.exists()
        assert versions()[-1] == "2.0.2"
        record("interrupted source version edit is recovered before retry")

        drift = work / "drift.sqlite"
        run("database", "migrate", "schema.acore", "--database-file", drift,
            "--frozen", "--non-interactive", "--allow-destructive")
        with sqlite3.connect(drift) as copy:
            copy.execute("ALTER TABLE tasks ADD COLUMN external TEXT")
        observed = status()  # The original remains current.
        result = run("database", "status", "schema.acore", "--database-file", drift, "--json")
        assert json.loads(result.stdout)["drift"] is True and observed["drift"] is False
        run("database", "migrate", "schema.acore", "--database-file", drift,
            "--non-interactive", expected=1)
        record("external DDL is visible and automatic migration refuses drift")

        race = work / "race.sqlite"
        export = work / "current.axiom"
        migrate("--export", export)
        roots = []
        for name in ["publisher-a", "publisher-b"]:
            root = work / name
            root.mkdir()
            shutil.copy2(export, root / "release.axiom")
            roots.append(root)
        processes = [
            subprocess.Popen([str(cli), "database", "migrate", "release.axiom",
                              "--database-file", str(race), "--frozen",
                              "--non-interactive", "--allow-destructive"],
                             cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True)
            for root in roots
        ]
        for process in processes:
            stdout, stderr = process.communicate(timeout=90)
            assert process.returncode in [0, 1], (stdout, stderr)
        assert any(process.returncode == 0 for process in processes)
        inspected = run("database", "status", export, "--database-file", race, "--json")
        assert len(json.loads(inspected.stdout)["releases"]) == len(versions())
        record("concurrent frozen apply does not duplicate or diverge the ledger")

        adopt = work / "adopt.sqlite"
        with sqlite3.connect(file) as original, sqlite3.connect(adopt) as copy:
            original.backup(copy)
            for name in ["__axiom_targets", "__axiom_revisions", "__axiom_attempts"]:
                copy.execute("DROP TABLE " + name)
        adoption = work / "adoption"
        adoption.mkdir()
        (adoption / "AxiomDeps.toml").write_text(manifest)
        (adoption / "schema.acore").write_text(source.read_text())
        run("database", "migrate", "schema.acore", "--database-file", adopt,
            "--non-interactive", root=adoption, expected=1)
        run("database", "migrate", "schema.acore", "--database-file", adopt,
            "--non-interactive", "--adopt", root=adoption)
        with sqlite3.connect(adopt) as db:
            assert db.execute("SELECT title FROM tasks").fetchall() == [("retained",)]
        record("explicit exact adoption preserves pre-existing data")

        interactive_env = dict(env)
        interactive_env.pop("CI", None)
        interactive_env["TERM"] = "xterm-256color"

        class Terminal:
            def __init__(self, tokens):
                self.pid, self.fd = pty.fork()
                if self.pid == 0:
                    os.chdir(project)
                    os.execve(str(cli), [str(cli), *map(str, tokens)], interactive_env)
                self.output = ""
                self.cursor = 0

            def send(self, value):
                os.write(self.fd, value)

            def expect(self, text, timeout=25):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    position = self.output.find(text, self.cursor)
                    if position >= 0:
                        self.cursor = position + len(text)
                        return
                    ready, _, _ = select.select([self.fd], [], [], .1)
                    if ready:
                        try:
                            data = os.read(self.fd, 65536)
                        except OSError as error:
                            if error.errno == errno.EIO:
                                break
                            raise
                        if not data:
                            break
                        self.output += data.decode(errors="replace")
                raise AssertionError((text, self.output[-5000:]))

            def finish(self):
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    pid, code = os.waitpid(self.pid, os.WNOHANG)
                    if pid:
                        os.close(self.fd)
                        assert os.waitstatus_to_exitcode(code) == 0, self.output[-3000:]
                        return
                    time.sleep(.05)
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
                raise AssertionError("CLI failed to exit after console EOF")

        source.write_text(source.read_text().replace(
            "note: Sqlite.Text(nullable: true)",
            "note: Sqlite.Text(nullable: true)\n    summary: Sqlite.Text(nullable: true)"))
        terminal = Terminal(["database", "migrate", "schema.acore", "--database-file", file])
        terminal.expect("Use next version")
        terminal.send(b"\r")
        terminal.expect("Applied migration 2.0.3")
        terminal.finish()
        assert 'version: "2.0.3"' in source.read_text()
        record("terminal Enter accepts the suggested version and minimal source edit")

        terminal = Terminal(["run", "schema.acore", "--database-file", file])
        terminal.expect("axiom> ")
        migrate()
        terminal.send(b"SELECT 'quoted;value' AS title;\r")
        terminal.expect("quoted;value\r")
        terminal.expect("axiom> ")
        terminal.send(b"\x1b[A\r")
        terminal.expect("(1 rows)")
        terminal.expect("axiom> ")
        terminal.send(b"SELECT\r")
        terminal.expect("...> ")
        terminal.send(b"'multiline';\r")
        terminal.expect("(1 rows)")
        terminal.expect("axiom> ")
        terminal.send(b"SELECT unfinished")
        terminal.send(b"\x03")
        terminal.expect("^C")
        terminal.expect("axiom> ")
        terminal.send(b"\\dt\r")
        terminal.expect("tasks")
        terminal.expect("axiom> ")
        expensive = (b"WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 "
                     b"FROM numbers WHERE n<1000000000) SELECT sum(n) AS total FROM numbers;")
        terminal.send(expensive)
        terminal.expect(expensive.decode())
        terminal.send(b"\r")
        time.sleep(.4)
        terminal.send(b"\x03")
        terminal.expect("Query cancelled.")
        terminal.expect("axiom> ")
        terminal.send(b"\x04")
        terminal.finish()
        record("native SQL PTY multiline, history, running-query cancellation and EOF")

        script = run("run", "schema.acore", "--database-file", file,
                     "--non-interactive",
                     input="SELECT 'script;value';\nSELECT count(*) FROM tasks;\n")
        assert "script;value" in script.stdout and "(1 rows)" in script.stdout
        record("SQL scripts run without an external database shell")

        before = source.read_text()
        run("database", "rollback", "schema.acore", "--database-file", file,
            "--to", "empty", "--auto-version", "--non-interactive", "--allow-destructive")
        with sqlite3.connect(file) as db:
            assert db.execute(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='tasks'"
            ).fetchone() == (0,)
        assert source.read_text() == before
        migrate("--auto-version")
        assert rows() == [] and versions()[-2:] == ["3.0.0", "3.0.1"]
        record("rollback to empty and reapply retain events without inventing deleted rows")

        visible = [p for p in project.rglob("*") if p.is_file()
                   and ".axiom" not in p.relative_to(project).parts
                   and p.suffix not in [".sqlite", ".acore", ".toml"]]
        assert visible == [], visible
        record("generated state and locks stay under the owning .axiom folder")

    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps({"format": "axiom-database-dx-acceptance/v1",
                                      "cli": str(cli), "scenarios": results}, indent=2) + "\n")
    print(f"{len(results)} installed-CLI scenarios passed")


if __name__ == "__main__":
    main()
