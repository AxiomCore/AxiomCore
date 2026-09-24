#!/usr/bin/env python3
"""Explicit, keyboard-driven review of local release-source worktrees.

Only files selected by the operator are staged and committed. This module
never pushes, builds, publishes, resets, or discards worktree changes.
"""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import subprocess
import sys

import ctl


@dataclass(frozen=True)
class ChangedFile:
    path: str
    status: str
    previous_path: str | None = None


@dataclass(frozen=True)
class ChangedRepo:
    path: Path
    files: tuple[ChangedFile, ...]


def parse_status(raw: bytes) -> tuple[ChangedFile, ...]:
    records = raw.split(b"\0")
    files: list[ChangedFile] = []
    index = 0
    while index < len(records):
        record = records[index]
        index += 1
        if not record:
            continue
        if len(record) < 4 or record[2:3] != b" ":
            raise ctl.ReleaseError("unexpected git status entry while reviewing release sources")
        status = record[:2].decode("ascii", "replace")
        path = record[3:].decode("utf-8", "surrogateescape")
        previous_path = None
        if "R" in status or "C" in status:
            if index >= len(records) or not records[index]:
                raise ctl.ReleaseError("git status omitted the original rename/copy path")
            previous_path = records[index].decode("utf-8", "surrogateescape")
            index += 1
        files.append(ChangedFile(path, status, previous_path))
    return tuple(sorted(files, key=lambda item: item.path))


def changed_repositories(catalog: dict, workspace: Path = ctl.WORKSPACE) -> tuple[ChangedRepo, ...]:
    paths: dict[Path, set[str]] = {}
    for name in catalog["repositories"]:
        paths.setdefault(ctl.repo_path(catalog, name, workspace), set()).add(name)
    result = []
    for path in sorted(paths):
        files = parse_status(ctl.run("git", "status", "--porcelain=v1", "-z",
                                     "--untracked-files=all", cwd=path))
        if path == workspace.resolve():
            # The aggregate workspace tracks acore-diff but sees sibling Git
            # checkouts as untracked directories. Never offer to commit those.
            selectors = [selector for component in catalog.get("components", [])
                         for source in component.get("sources", [])
                         if source["repo"] in paths[path] for selector in source["paths"]]
            files = tuple(file for file in files
                          if any(ctl.matches(file.path, selector) for selector in selectors))
        if files:
            result.append(ChangedRepo(path, files))
    return tuple(result)


def commit_selected(repository: Path, selected: tuple[str, ...], message: str) -> str:
    message = message.strip()
    if not selected or not message:
        raise ctl.ReleaseError("select at least one file and enter a commit message")
    current = {item.path: item for item in parse_status(ctl.run("git", "status", "--porcelain=v1",
                                                             "-z", "--untracked-files=all", cwd=repository))}
    if not set(selected) <= current.keys():
        raise ctl.ReleaseError("selected files changed since review; refresh the repository before committing")
    exact_paths = tuple(sorted(selected))
    renames = [current[path] for path in selected if "R" in current[path].status]
    if renames:
        allowed = set(selected) | {file.previous_path for file in renames if file.previous_path}
        staged = {path.decode("utf-8", "surrogateescape") for path in
                  ctl.run("git", "diff", "--cached", "--name-only", "--no-renames",
                          "-z", cwd=repository).split(b"\0") if path}
        if not staged <= allowed:
            raise ctl.ReleaseError("a rename is selected while unrelated files are staged; "
                                   "commit or unstage those files manually before retrying")
    ctl.run("git", "add", "--", *exact_paths, cwd=repository)
    if renames:
        staged_after = {path.decode("utf-8", "surrogateescape") for path in
                        ctl.run("git", "diff", "--cached", "--name-only", "--no-renames",
                                "-z", cwd=repository).split(b"\0") if path}
        if not staged_after <= allowed:
            raise ctl.ReleaseError("staged files changed during rename review; inspect the index before committing")
        output = ctl.run("git", "commit", "-m", message, cwd=repository)
    else:
        # --only prevents unrelated paths already in the index from joining this commit.
        output = ctl.run("git", "commit", "--only", "-m", message, "--", *exact_paths,
                         cwd=repository)
    output = output.decode(errors="replace").strip()
    return output.splitlines()[0] if output else "Commit created"


def diff_for(repository: Path, file: ChangedFile) -> list[str]:
    if file.status == "??":
        command = ["git", "diff", "--no-index", "--no-color", "--", "/dev/null", file.path]
    else:
        command = ["git", "diff", "--no-color", "HEAD", "--", file.path]
    completed = subprocess.run(command, cwd=repository, check=False, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE)
    if completed.returncode not in (0, 1):
        return [completed.stderr.decode(errors="replace").strip() or "Diff unavailable"]
    payload = completed.stdout[:65536].decode("utf-8", "replace")
    lines = payload.splitlines() or ["No textual diff available"]
    if len(completed.stdout) > 65536:
        lines.append("…diff truncated after 64 KiB")
    return lines


def _put(screen, row: int, column: int, value: str, *, selected: bool = False) -> None:
    import curses

    height, width = screen.getmaxyx()
    if row < 0 or row >= height or column >= width:
        return
    try:
        screen.addnstr(row, column, value.replace("\n", "\\n").replace("\r", "\\r"),
                       max(0, width - column - 1), curses.A_REVERSE if selected else 0)
    except curses.error:
        pass


def _ask(screen, label: str, initial: str = "") -> str | None:
    import curses

    value = list(initial)
    try:
        curses.curs_set(1)
    except curses.error:
        pass
    while True:
        height, width = screen.getmaxyx()
        screen.move(height - 1, 0)
        screen.clrtoeol()
        display = label + "".join(value)
        _put(screen, height - 1, 0, display[-max(1, width - 1):])
        screen.refresh()
        key = screen.get_wch()
        if key in ("\n", "\r"):
            result = "".join(value).strip()
            break
        if key == "\x1b":
            result = None
            break
        if key in ("\b", "\x7f", curses.KEY_BACKSPACE):
            if value:
                value.pop()
        elif isinstance(key, str) and key.isprintable() and len(value) < 160:
            value.append(key)
    try:
        curses.curs_set(0)
    except curses.error:
        pass
    return result


def _diff_screen(screen, repository: Path, file: ChangedFile) -> None:
    import curses

    lines = diff_for(repository, file)
    offset = 0
    while True:
        height, _ = screen.getmaxyx()
        screen.erase()
        _put(screen, 0, 0, f"Diff: {repository.name}/{file.path}")
        for row, line in enumerate(lines[offset:offset + max(0, height - 3)], 1):
            _put(screen, row, 0, line.expandtabs(4))
        _put(screen, height - 1, 0, "↑/↓ scroll · PgUp/PgDn page · b/Enter back")
        screen.refresh()
        key = screen.getch()
        if key in (ord("b"), ord("q"), 10, 13, 27):
            return
        if key in (curses.KEY_DOWN, ord("j")):
            offset = min(max(0, len(lines) - 1), offset + 1)
        elif key in (curses.KEY_UP, ord("k")):
            offset = max(0, offset - 1)
        elif key == curses.KEY_NPAGE:
            offset = min(max(0, len(lines) - 1), offset + max(1, height - 3))
        elif key == curses.KEY_PPAGE:
            offset = max(0, offset - max(1, height - 3))


def _file_screen(screen, repository: ChangedRepo, train_id: str) -> str:
    import curses

    files = repository.files
    selected: set[str] = set()
    position = 0
    message = ""
    notice = ""
    while True:
        height, _ = screen.getmaxyx()
        screen.erase()
        _put(screen, 0, 0, f"{repository.path} · {len(files)} changed file(s)")
        _put(screen, 1, 0, "↑/↓ move · Space select · a all · d diff · m message · c commit · b back · q quit")
        visible = max(1, height - 5)
        start = max(0, min(position - visible + 1, len(files) - visible))
        for row, file in enumerate(files[start:start + visible], 2):
            mark = "x" if file.path in selected else " "
            name = f"{file.previous_path} → {file.path}" if file.previous_path else file.path
            label = f"[{mark}] {file.status}  {name}"
            _put(screen, row, 0, label, selected=(start + row - 2 == position))
        _put(screen, height - 3, 0, f"Selected: {len(selected)} · Message: {message or '(press m to enter)'}")
        _put(screen, height - 2, 0, notice)
        screen.refresh()
        key = screen.getch()
        if key in (curses.KEY_DOWN, ord("j")):
            position = min(len(files) - 1, position + 1)
        elif key in (curses.KEY_UP, ord("k")):
            position = max(0, position - 1)
        elif key == curses.KEY_NPAGE:
            position = min(len(files) - 1, position + visible)
        elif key == curses.KEY_PPAGE:
            position = max(0, position - visible)
        elif key == ord(" "):
            path = files[position].path
            selected.remove(path) if path in selected else selected.add(path)
        elif key == ord("a"):
            selected = set() if len(selected) == len(files) else {file.path for file in files}
        elif key in (ord("d"), 10, 13):
            _diff_screen(screen, repository.path, files[position])
        elif key == ord("m"):
            entered = _ask(screen, "Commit message: ", message)
            if entered is not None:
                message = entered
        elif key == ord("c"):
            if not selected or not message.strip():
                notice = "Select files and enter a commit message first."
                continue
            approved = _ask(screen, f"Commit {len(selected)} selected file(s)? Type yes: ")
            if approved != "yes":
                notice = "Commit cancelled."
                continue
            try:
                notice = commit_selected(repository.path, tuple(sorted(selected)), message)
                return notice
            except ctl.ReleaseError as error:
                notice = str(error)
        elif key in (ord("b"), 27):
            return ""
        elif key == ord("q"):
            return "quit"


def _session(screen, catalog: dict, workspace: Path, train_id: str) -> str:
    import curses

    try:
        curses.curs_set(0)
    except curses.error:
        pass
    screen.keypad(True)
    position = 0
    notice = ""
    started_dirty = bool(changed_repositories(catalog, workspace))
    while True:
        repositories = changed_repositories(catalog, workspace)
        if not repositories:
            if not started_dirty:
                return "proceed"
            screen.erase()
            _put(screen, 0, 0, "All declared repositories are clean.")
            _put(screen, 2, 0, "Enter: continue to safe release preflight · q: exit")
            screen.refresh()
            key = screen.getch()
            if key in (10, 13):
                return "proceed"
            if key in (ord("q"), 27):
                return "exit"
            continue
        position = min(position, len(repositories) - 1)
        height, _ = screen.getmaxyx()
        screen.erase()
        _put(screen, 0, 0, f"AxiomCore release {train_id} · {len(repositories)} repository worktree(s) to review")
        _put(screen, 1, 0, "↑/↓ choose repository · Enter inspect files · q exit (nothing committed automatically)")
        visible = max(1, height - 5)
        start = max(0, min(position - visible + 1, len(repositories) - visible))
        for row, repository in enumerate(repositories[start:start + visible], 2):
            _put(screen, row, 0, f"{repository.path}  ({len(repository.files)} changed)",
                 selected=(start + row - 2 == position))
        _put(screen, height - 2, 0, notice)
        screen.refresh()
        key = screen.getch()
        if key in (curses.KEY_DOWN, ord("j")):
            position = min(len(repositories) - 1, position + 1)
        elif key in (curses.KEY_UP, ord("k")):
            position = max(0, position - 1)
        elif key == curses.KEY_NPAGE:
            position = min(len(repositories) - 1, position + visible)
        elif key == curses.KEY_PPAGE:
            position = max(0, position - visible)
        elif key in (10, 13):
            notice = _file_screen(screen, repositories[position], train_id)
            if notice == "quit":
                return "exit"
        elif key in (ord("q"), 27):
            return "exit"


def run(catalog: dict, train_id: str, workspace: Path = ctl.WORKSPACE) -> str:
    """Return proceed, exit, or needs_tty; never commits without UI confirmation."""
    repositories = changed_repositories(catalog, workspace)
    if not repositories:
        return "proceed"
    if not (sys.stdin.isatty() and sys.stdout.isatty()):
        print(f"{len(repositories)} repositories have uncommitted files. Run `just release review` "
              "in an interactive terminal to inspect and commit them. Nothing was committed.")
        for repository in repositories:
            print(f"  {repository.path} ({len(repository.files)} changed)")
        return "needs_tty"
    if os.environ.get("TERM", "dumb") == "dumb":
        print("This terminal does not advertise arrow-key support (TERM=dumb). "
              "Open `just release review` in a normal terminal. Nothing was committed.")
        return "needs_tty"
    import curses

    return curses.wrapper(_session, catalog, workspace, train_id)
