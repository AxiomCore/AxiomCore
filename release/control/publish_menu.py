#!/usr/bin/env python3
"""Explicit multi-select terminal picker for staged release publications."""

from __future__ import annotations

import sys
import os
from pathlib import Path

import ctl


HOST_IDS = {"ui-host-web", "ui-host-android", "ui-host-ios"}


def entries(catalog: dict, root: Path, train_id: str) -> list[tuple[str, bool]]:
    directory = root / "trains" / train_id / "components"
    ordered = ["ui-host", *(item["id"] for item in catalog["components"]
                            if item["id"] not in HOST_IDS)]
    result = []
    for component in ordered:
        candidate = directory / component
        ready = all((candidate / name).is_file() and not (candidate / name).is_symlink()
                    for name in ("intent.json", "plan.json", "staged.json", "gate.json", "notes.md"))
        result.append((component, ready))
    return result


def choose(catalog: dict, root: Path, train_id: str) -> list[str]:
    options = entries(catalog, root, train_id)
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise ctl.ReleaseError("interactive publish selection requires a terminal; use `just release publish COMPONENT [TRAIN]` in automation")
    if os.environ.get("TERM", "dumb") == "dumb":
        raise ctl.ReleaseError("publish selection needs an arrow-key terminal (TERM is dumb); specify COMPONENT in automation")
    if not any(ready for _, ready in options):
        print(f"No staged candidates to publish for train {train_id}. Build a candidate first.")
        return []
    import curses

    def session(screen) -> list[str]:
        try:
            curses.curs_set(0)
        except curses.error:
            pass
        screen.keypad(True)
        position = 0
        selected: set[str] = set()
        notice = ""
        while True:
            height, width = screen.getmaxyx()
            screen.erase()
            screen.addnstr(0, 0, f"Publish train {train_id} · select staged candidates", width - 1)
            screen.addnstr(1, 0, "↑/↓ move · Space select · Enter review · q cancel", width - 1)
            visible = max(1, height - 5)
            start = max(0, min(position - visible + 1, len(options) - visible))
            for row, (name, ready) in enumerate(options[start:start + visible], 2):
                index = start + row - 2
                label = f"[{'x' if name in selected else ' '}] {name:24} {'staged' if ready else 'not staged'}"
                try:
                    screen.addnstr(row, 0, label, width - 1,
                                   curses.A_REVERSE if index == position else
                                   curses.A_DIM if not ready else 0)
                except curses.error:
                    pass
            screen.addnstr(height - 2, 0, notice or f"Selected: {len(selected)}", width - 1)
            screen.refresh()
            key = screen.getch()
            if key in (curses.KEY_DOWN, ord("j")):
                position = min(len(options) - 1, position + 1)
            elif key in (curses.KEY_UP, ord("k")):
                position = max(0, position - 1)
            elif key == ord(" "):
                name, ready = options[position]
                if ready:
                    selected.remove(name) if name in selected else selected.add(name)
                    notice = ""
                else:
                    notice = f"{name} has no complete staged candidate."
            elif key in (10, 13):
                if not selected:
                    notice = "Select at least one staged candidate first."
                    continue
                chosen = [name for name, _ in options if name in selected]
                screen.erase()
                screen.addnstr(0, 0, "Publish these production destinations?", width - 1)
                for row, name in enumerate(chosen[:max(0, height - 4)], 2):
                    screen.addnstr(row, 0, f"  {name}", width - 1)
                screen.addnstr(height - 2, 0, "Type p to start, or any other key to return.", width - 1)
                screen.refresh()
                if screen.getch() == ord("p"):
                    return chosen
            elif key in (ord("q"), 27):
                return []

    return curses.wrapper(session)
