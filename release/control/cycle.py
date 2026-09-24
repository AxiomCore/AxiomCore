#!/usr/bin/env python3
"""Interactive, non-publishing creation of the next release train."""

from __future__ import annotations

from datetime import datetime
import json
import os
from pathlib import Path
import stat
import sys
import tempfile

import ctl
import flow
import versions


HOST_IDS = ("ui-host-web", "ui-host-android", "ui-host-ios")
TYPES = ("feature", "fix", "security", "breaking", "internal")
INTENT = ctl.CONTROL_DIR / "intent.json"
DRAFT_FORMAT = "axiom-platform-release-cycle-draft/v1"


def draft_path(root: Path) -> Path:
    return root / "drafts" / "active-new-release.json"


def _checkpoint(path: Path, draft: dict) -> None:
    if path.is_symlink():
        raise ctl.ReleaseError(f"release draft is a symlink: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    _atomic_write(path, (json.dumps(draft, indent=2, ensure_ascii=False) + "\n").encode())


def read_draft(path: Path, catalog: dict, old_intent: bytes, old_ledger: bytes) -> dict:
    if path.is_symlink() or not path.is_file():
        raise ctl.ReleaseError(f"release draft is not an ordinary file: {path}")
    draft = json.loads(path.read_text())
    ids = {entry["id"] for entry in catalog["components"]}
    selected = draft.get("selected")
    confirmed = draft.get("selectionConfirmed", True)
    if (draft.get("format") != DRAFT_FORMAT or not isinstance(selected, list)
            or not isinstance(confirmed, bool) or (confirmed and not selected)
            or len(set(selected)) != len(selected) or not set(selected) <= ids
            or not isinstance(draft.get("answers"), dict)
            or not set(draft["answers"]) <= set(selected)
            or any(not isinstance(answer, dict) for answer in draft["answers"].values())
            or not flow.TRAIN_ID.fullmatch(str(draft.get("trainId", "")))):
        raise ctl.ReleaseError(f"release draft is malformed; inspect it before restarting: {path}")
    if (draft.get("intentSha256") != ctl.sha256(old_intent)
            or draft.get("versionsSha256") != ctl.sha256(old_ledger)
            or draft.get("catalogSha256") != catalog["sha256"]):
        raise ctl.ReleaseError("release sources changed since the saved draft; it is preserved at "
                               f"{path}. Review changes, or run `just release new restart` to archive it and start over")
    return draft


def archive_draft(path: Path, root: Path) -> Path | None:
    if not path.exists():
        return None
    if path.is_symlink() or not path.is_file():
        raise ctl.ReleaseError(f"release draft is not an ordinary file: {path}")
    if input("Archive the existing unfinished draft and start over? Type archive: ").strip() != "archive":
        print("Existing draft preserved; nothing changed.")
        return None
    archive = root / "drafts" / "archive"
    archive.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().astimezone().strftime("%Y%m%d-%H%M%S-%f")
    destination = archive / f"{stamp}.json"
    if destination.exists():
        raise ctl.ReleaseError(f"draft archive target exists: {destination}")
    os.replace(path, destination)
    print(f"Archived previous draft: {destination}")
    return destination


def finish_saved_draft(path: Path, root: Path, intent_bytes: bytes,
                       ledger_bytes: bytes) -> bool:
    """Recover an interruption after source files were saved, before draft archival."""
    if path.is_symlink() or not path.is_file():
        return False
    try:
        raw = json.loads(path.read_text())
    except (OSError, ValueError):
        return False
    train_id = raw.get("trainId")
    if not isinstance(train_id, str) or not flow.TRAIN_ID.fullmatch(train_id):
        return False
    backup = root / "trains" / train_id / "cycle-backup"
    expected_intent = backup / "intent-next.json"
    expected_ledger = backup / "versions-next.json"
    target = backup / "wizard-draft.json"
    if (target.exists() or not expected_intent.is_file() or not expected_ledger.is_file()
            or expected_intent.is_symlink() or expected_ledger.is_symlink()
            or expected_intent.read_bytes() != intent_bytes
            or expected_ledger.read_bytes() != ledger_bytes):
        return False
    os.replace(path, target)
    print(f"Release cycle {train_id} was already saved; archived its interrupted draft at {target}.")
    return True


def next_train_id(current: str, root: Path, today: str | None = None) -> str:
    day = today or datetime.now().astimezone().strftime("%Y.%m.%d")
    prefix = day + "."
    numbers = []
    if current.startswith(prefix) and current[len(prefix):].isdigit():
        numbers.append(int(current[len(prefix):]))
    directory = root / "trains"
    if directory.exists():
        for path in directory.iterdir():
            if path.name.startswith(prefix) and path.name[len(prefix):].isdigit():
                numbers.append(int(path.name[len(prefix):]))
    return prefix + str(max(numbers, default=0) + 1)


def published_evidence(root: Path, catalog: dict) -> dict[str, dict]:
    """Show only locally recorded remote-verification, never infer publication."""
    known = {entry["id"]: entry for entry in catalog["components"]}
    result: dict[str, dict] = {}
    directory = root / "trains"
    if not directory.is_dir():
        return result
    for train in directory.iterdir():
        if not train.is_dir() or train.is_symlink():
            continue
        components = train / "components"
        if not components.is_dir() or components.is_symlink():
            continue
        for candidate in components.iterdir():
            if not candidate.is_dir() or candidate.is_symlink():
                continue
            publication = candidate / "publication" / "published.json"
            staged = candidate / "staged.json"
            intent_path = candidate / "intent.json"
            if any(not path.is_file() or path.is_symlink() for path in (publication, staged, intent_path)):
                continue
            try:
                record = json.loads(publication.read_text())
                scoped = json.loads(intent_path.read_text())
                stage = json.loads(staged.read_text())
            except (OSError, ValueError):
                continue
            if (record.get("format") != "axiom-platform-component-publication/v1"
                    or record.get("status") != "remote-verified"
                    or record.get("trainId") != train.name
                    or scoped.get("trainId") != train.name
                    or stage.get("trainId") != train.name
                    or record.get("stageSha256") != ctl.sha256(ctl.canonical(stage))
                    or record.get("component") != candidate.name):
                continue
            for change in scoped.get("changes", []):
                component_id = change.get("component")
                if component_id not in known or (candidate.name != component_id and
                                                 not (candidate.name == "ui-host" and component_id in HOST_IDS)):
                    continue
                release_version = change.get("version") or record.get("version")
                if versions.versioned(known[component_id]) and (
                        not isinstance(release_version, str)
                        or not versions.STABLE_VERSION.fullmatch(release_version)):
                    continue
                value = {"trainId": train.name, "version": release_version,
                         "record": str(publication)}
                if component_id not in result or _train_rank(train.name) > _train_rank(result[component_id]["trainId"]):
                    result[component_id] = value
    return result


def _train_rank(train_id: str) -> tuple[int, int, int, int]:
    parts = train_id.split(".")
    if len(parts) == 4 and all(part.isdigit() for part in parts):
        return tuple(map(int, parts))
    return (0, 0, 0, 0)


def expand_selection(selected: set[str], catalog: dict) -> set[str]:
    by_id = {entry["id"]: entry for entry in catalog["components"]}
    if selected & set(HOST_IDS):
        selected.update(HOST_IDS)
    pending = list(selected)
    while pending:
        component_id = pending.pop()
        for dependency in by_id[component_id].get("depends_on", []):
            if dependency not in selected:
                selected.add(dependency)
                pending.append(dependency)
    return selected


def _put(screen, row: int, value: str, attr: int = 0) -> None:
    import curses

    height, width = screen.getmaxyx()
    if row < 0 or row >= height or width < 2:
        return
    try:
        screen.addnstr(row, 0, value, width - 1, attr)
    except curses.error:
        pass


def choose_components(catalog: dict, ledger: dict, root: Path,
                      published: dict[str, dict], *, initial: list[str] | None = None,
                      on_change=None) -> list[str]:
    if not sys.stdin.isatty() or not sys.stdout.isatty() or os.environ.get("TERM", "dumb") == "dumb":
        raise ctl.ReleaseError("new release selection needs an interactive arrow-key terminal")
    import curses

    entries = catalog["components"]
    labels = []
    for entry in entries:
        component_id = entry["id"]
        last = published.get(component_id, {})
        released = last.get("version") or ("digest @ " + last["trainId"] if last else "unknown")
        source = ctl.component_version(entry, catalog, ctl.WORKSPACE) or "—"
        candidate = ledger["components"][component_id]["candidateVersion"] or "digest"
        labels.append((component_id, released, source, candidate))

    def session(screen) -> list[str]:
        try:
            curses.curs_set(0)
        except curses.error:
            pass
        screen.keypad(True)
        position = 0
        selected: set[str] = set(initial or [])
        notice = ""
        while True:
            height, width = screen.getmaxyx()
            screen.erase()
            _put(screen, 0, "New release · choose components")
            _put(screen, 1, "↑/↓ move · Space select · Enter continue · q cancel")
            visible = max(1, height - 5)
            start = max(0, min(position - visible + 1, len(entries) - visible))
            for row, entry in enumerate(entries[start:start + visible], 2):
                index = start + row - 2
                component_id = entry["id"]
                name, released, source, candidate = labels[index]
                label = (f"[{'x' if component_id in selected else ' '}] {name:22} "
                         f"last {released[:18]:18} current {source[:12]:12}")
                if width >= 92:
                    label += f" next {candidate}"
                _put(screen, row, label, curses.A_REVERSE if index == position else 0)
            _put(screen, height - 2, notice or
                 f"{len(selected)} selected · next {labels[position][3]} · last is locally recorded remote verification")
            screen.refresh()
            key = screen.getch()
            if key in (curses.KEY_DOWN, ord("j")):
                position = min(len(entries) - 1, position + 1)
            elif key in (curses.KEY_UP, ord("k")):
                position = max(0, position - 1)
            elif key in (curses.KEY_NPAGE, curses.KEY_PPAGE):
                delta = visible if key == curses.KEY_NPAGE else -visible
                position = max(0, min(len(entries) - 1, position + delta))
            elif key == ord(" "):
                component_id = entries[position]["id"]
                targets = set(HOST_IDS) if component_id in HOST_IDS else {component_id}
                selected.difference_update(targets) if targets <= selected else selected.update(targets)
                if on_change is not None:
                    on_change([entry["id"] for entry in entries if entry["id"] in selected])
                notice = "UI Host web, Android, and iOS release together." if component_id in HOST_IDS else ""
            elif key in (10, 13):
                if not selected:
                    notice = "Select at least one component."
                    continue
                expanded = expand_selection(set(selected), catalog)
                extras = sorted(expanded - selected)
                screen.erase()
                _put(screen, 0, f"Create a new cycle for {len(expanded)} component(s)?")
                for row, component_id in enumerate(sorted(expanded)[:max(0, height - 5)], 2):
                    _put(screen, row, f"  {component_id}")
                _put(screen, height - 2, ("Required dependencies added: " + ", ".join(extras) if extras else
                                          "No additional dependencies.") + "  y confirm · other key back")
                screen.refresh()
                if screen.getch() == ord("y"):
                    return [entry["id"] for entry in entries if entry["id"] in expanded]
            elif key in (ord("q"), 27):
                return []

    return curses.wrapper(session)


def choose_type(component_id: str, current: str = "feature") -> str:
    import curses

    def session(screen) -> str:
        try:
            curses.curs_set(0)
        except curses.error:
            pass
        screen.keypad(True)
        position = TYPES.index(current) if current in TYPES else 0
        while True:
            screen.erase()
            _put(screen, 0, f"{component_id} · what kind of change?")
            _put(screen, 1, "↑/↓ move · Enter choose · Esc cancel")
            for row, change_type in enumerate(TYPES, 3):
                _put(screen, row, change_type, curses.A_REVERSE if row - 3 == position else 0)
            screen.refresh()
            key = screen.getch()
            if key in (curses.KEY_DOWN, ord("j")):
                position = min(len(TYPES) - 1, position + 1)
            elif key in (curses.KEY_UP, ord("k")):
                position = max(0, position - 1)
            elif key in (10, 13):
                return TYPES[position]
            elif key in (27, ord("q")):
                return ""

    return curses.wrapper(session)


def ask_line(label: str, default: str = "", *, required: bool = True) -> str:
    while True:
        suffix = f" [{default}]" if default else ""
        value = input(f"{label}{suffix}: ").strip() or default
        if value or not required:
            if "\n" in value or "\r" in value or len(value) > 500:
                print("Enter a single line of at most 500 characters.")
                continue
            return value
        print("A value is required. Ctrl-C cancels without saving.")


def component_summary_template(component_id: str, version: str | None) -> str:
    if version:
        return f"Release {component_id} {version} through the verified Axiom release control plane."
    return f"Release {component_id} as a verified digest-based deployment."


def overall_summary_template(changes: list[dict], ledger: dict) -> str:
    labels = [f"{change['component']} {ledger['components'][change['component']]['candidateVersion'] or '(digest)'}"
              for change in changes]
    result = "Release " + ", ".join(labels) + " through the Axiom release control plane."
    return result if len(result) <= 500 else f"Release {len(changes)} Axiom components through the verified release control plane."


def choose_summary_mode(label: str, template: str, existing: str | None = None) -> str:
    print(f"\n{label} summary template:\n  {template}")
    if existing is not None:
        print(f"Recovered previous answer:\n  {existing}")
    while True:
        prompt = ("[k]eep recovered, use [t]emplate, or [w]rite from empty? " if existing is not None
                  else "Use [t]emplate or [w]rite my own? ")
        choice = input(prompt).strip().lower()
        if existing is not None and choice in ("k", "keep"):
            return "keep"
        if choice in ("t", "template"):
            return "template"
        if choice in ("w", "write"):
            return "write"
        print("Enter k, t, or w." if existing is not None else
              "Enter t or w. Your custom summary starts empty.")


def suggested_version(candidate: str | None, source: str | None,
                      released: str | None, change_type: str) -> str:
    if not released:
        return candidate or source or ""
    if candidate and tuple(map(int, candidate.split("."))) > tuple(map(int, released.split("."))):
        return candidate
    major, minor, patch = map(int, released.split("."))
    if change_type == "breaking":
        return f"{major + 1}.0.0"
    if change_type == "feature":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def next_free_version(default: str, component_id: str, ledger: dict, catalog: dict) -> str:
    if not versions.STABLE_VERSION.fullmatch(default):
        return default
    proposed_version = default
    for _ in range(100):
        trial = json.loads(json.dumps(ledger))
        targets = HOST_IDS if component_id in HOST_IDS else (component_id,)
        for target in targets:
            trial["components"][target]["candidateVersion"] = proposed_version
        try:
            versions.validate_versions(trial, catalog)
            return proposed_version
        except ctl.ReleaseError as error:
            if "would both claim" not in str(error):
                raise
            major, minor, patch = map(int, proposed_version.split("."))
            proposed_version = f"{major}.{minor}.{patch + 1}"
    raise ctl.ReleaseError(f"no available nearby candidate version for {component_id}")


def collect_changes(selected: list[str], catalog: dict, ledger: dict,
                    old_intent: dict, published: dict[str, dict],
                    draft: dict, checkpoint: Path) -> tuple[list[dict], dict]:
    by_id = {entry["id"]: entry for entry in catalog["components"]}
    previous = {change["component"]: change for change in
                [*old_intent["changes"], *old_intent.get("queued", [])]}
    updated = json.loads(json.dumps(ledger))
    changes = []
    host_version = None
    for index, component_id in enumerate(selected, 1):
        entry = by_id[component_id]
        prior = previous.get(component_id, {})
        answer = draft["answers"].setdefault(component_id, {})
        source = ctl.component_version(entry, catalog, ctl.WORKSPACE)
        last = published.get(component_id, {})
        print(f"\n[{index}/{len(selected)}] {component_id}")
        print(f"  Last remotely verified here: {last.get('version') or last.get('trainId') or 'unknown'}"
              f" · current source: {source or 'n/a'}"
              f" · candidate: {ledger['components'][component_id]['candidateVersion'] or 'digest'}")
        if "type" not in answer:
            answer["type"] = choose_type(component_id, prior.get("type", "feature"))
            if not answer["type"]:
                raise KeyboardInterrupt
            _checkpoint(checkpoint, draft)
        change_type = answer["type"]
        if change_type not in TYPES:
            raise ctl.ReleaseError(f"saved change type is invalid for {component_id}")
        if versions.versioned(entry):
            default = host_version if component_id in HOST_IDS and host_version else (
                suggested_version(ledger["components"][component_id]["candidateVersion"],
                                  source, last.get("version"), change_type))
            if default and not (component_id in HOST_IDS and host_version):
                available = next_free_version(default, component_id, updated, catalog)
                if available != default:
                    print(f"  {default} is already claimed in its release channel; suggesting {available}.")
                    default = available
            if "version" in answer:
                value = answer["version"]
            elif component_id in HOST_IDS and host_version:
                value = host_version
                print(f"  Shared UI Host version: {value}")
                answer["version"] = value
                _checkpoint(checkpoint, draft)
            else:
                while True:
                    value = ask_line("Next candidate version (X.Y.Z)", default)
                    if not versions.STABLE_VERSION.fullmatch(value):
                        print("Use a stable X.Y.Z version.")
                        continue
                    if source and tuple(map(int, value.split("."))) < tuple(map(int, source.split("."))):
                        print(f"Candidate must be at least the current source version {source}.")
                        continue
                    released = last.get("version")
                    if released and tuple(map(int, value.split("."))) <= tuple(map(int, released.split("."))):
                        print(f"Candidate must be newer than the locally verified release {released}.")
                        continue
                    proposed = json.loads(json.dumps(updated))
                    if component_id in HOST_IDS:
                        for host_id in HOST_IDS:
                            proposed["components"][host_id]["candidateVersion"] = value
                    else:
                        proposed["components"][component_id]["candidateVersion"] = value
                    try:
                        versions.validate_versions(proposed, catalog)
                    except ctl.ReleaseError as error:
                        major, minor, patch = map(int, value.split("."))
                        default = f"{major}.{minor}.{patch + 1}"
                        print(f"Version conflict: {error}. Suggested next version: {default}")
                        continue
                    break
                answer["version"] = value
                _checkpoint(checkpoint, draft)
            if not isinstance(value, str) or not versions.STABLE_VERSION.fullmatch(value):
                raise ctl.ReleaseError(f"saved candidate version is invalid for {component_id}")
            if source and tuple(map(int, value.split("."))) < tuple(map(int, source.split("."))):
                raise ctl.ReleaseError(f"saved candidate version for {component_id} is older than source")
            released = last.get("version")
            if released and tuple(map(int, value.split("."))) <= tuple(map(int, released.split("."))):
                raise ctl.ReleaseError(f"saved candidate version for {component_id} is not newer than the last verified release")
            if component_id in HOST_IDS and host_version and value != host_version:
                raise ctl.ReleaseError("saved UI Host target versions disagree")
            if component_id in HOST_IDS:
                host_version = value
                for host_id in HOST_IDS:
                    updated["components"][host_id]["candidateVersion"] = value
            else:
                updated["components"][component_id]["candidateVersion"] = value
            versions.validate_versions(updated, catalog)
        version = answer.get("version") if versions.versioned(entry) else None
        template = component_summary_template(component_id, version)
        if answer.get("summaryMode") == "recovered":
            mode = choose_summary_mode(component_id, template, answer.get("summary"))
            if mode == "keep":
                answer["summaryMode"] = "write"
            elif mode == "template":
                answer["summaryMode"] = "template"
                answer["summary"] = template
            elif mode == "write":
                answer["summaryMode"] = "write"
                answer.pop("summary", None)
            _checkpoint(checkpoint, draft)
        if "summary" not in answer:
            if "summaryMode" not in answer:
                answer["summaryMode"] = choose_summary_mode(component_id, template)
                _checkpoint(checkpoint, draft)
            if answer["summaryMode"] == "template":
                answer["summary"] = template
            elif answer["summaryMode"] == "write":
                answer["summary"] = ask_line("What exactly changed?")
            else:
                raise ctl.ReleaseError(f"saved summary choice is invalid for {component_id}")
            _checkpoint(checkpoint, draft)
        summary = answer["summary"]
        if not isinstance(summary, str) or not summary.strip():
            raise ctl.ReleaseError(f"saved change summary is invalid for {component_id}")
        change = {"component": component_id, "type": change_type, "summary": summary}
        if change_type == "breaking":
            if "migration" not in answer:
                answer["migration"] = ask_line("Required migration guidance")
                _checkpoint(checkpoint, draft)
            change["migration"] = answer["migration"]
        if "runtimeVersion" in prior:
            change["runtimeVersion"] = prior["runtimeVersion"]
        changes.append(change)
    versions.validate_versions(updated, catalog)
    return changes, updated


def compose_intent(old: dict, train_id: str, changes: list[dict], summary: str,
                   published: dict[str, dict]) -> dict:
    selected = {change["component"] for change in changes}
    queued = []
    for change in [*old["changes"], *old.get("queued", [])]:
        if change["component"] in selected:
            continue
        if (change in old["changes"] and published.get(change["component"], {}).get("trainId")
                == old["trainId"]):
            continue
        queued.append({key: value for key, value in change.items() if key != "version"})
    return {"format": flow.INTENT_FORMAT, "trainId": train_id,
            "wave": "release", "summary": summary,
            "changes": changes, "queued": queued}


def _atomic_write(path: Path, data: bytes) -> None:
    with tempfile.NamedTemporaryFile("wb", dir=path.parent, prefix=".axiom-cycle-", delete=False) as output:
        output.write(data)
        temporary = Path(output.name)
    try:
        if path.is_symlink():
            raise ctl.ReleaseError(f"release file is a symlink: {path}")
        os.chmod(temporary, stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o600)
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def save_cycle(root: Path, train_id: str, intent_path: Path, ledger_path: Path,
               old_intent: bytes, old_ledger: bytes, intent: dict, ledger: dict) -> Path:
    if (not intent_path.is_file() or intent_path.is_symlink() or not ledger_path.is_file()
            or ledger_path.is_symlink()):
        raise ctl.ReleaseError("release intent and version ledger must be ordinary files")
    if intent_path.read_bytes() != old_intent or ledger_path.read_bytes() != old_ledger:
        raise ctl.ReleaseError("intent or versions changed while the wizard was open; nothing saved")
    directory = root / "trains" / train_id
    if directory.exists():
        raise ctl.ReleaseError(f"new train ID already has evidence; nothing saved: {directory}")
    directory.mkdir(parents=True)
    backup = directory / "cycle-backup"
    backup.mkdir()
    ctl.write_new_text(backup / "intent-before.json", old_intent.decode())
    ctl.write_new_text(backup / "versions-before.json", old_ledger.decode())
    next_intent = (json.dumps(intent, indent=2, ensure_ascii=False) + "\n").encode()
    next_ledger = (old_ledger if json.loads(old_ledger) == ledger else
                   (json.dumps(ledger, indent=2, ensure_ascii=False) + "\n").encode())
    ctl.write_new_text(backup / "intent-next.json", next_intent.decode())
    ctl.write_new_text(backup / "versions-next.json", next_ledger.decode())
    _atomic_write(intent_path, next_intent)
    if next_ledger != old_ledger:
        _atomic_write(ledger_path, next_ledger)
    return backup


def run(catalog: dict, ledger: dict, intent: dict,
        root: Path, intent_path: Path = INTENT, ledger_path: Path = versions.VERSIONS,
        *, restart: bool = False) -> bool:
    old_intent = intent_path.read_bytes()
    old_ledger = ledger_path.read_bytes()
    checkpoint = draft_path(root)
    if checkpoint.exists() and finish_saved_draft(checkpoint, root, old_intent, old_ledger):
        return True
    if restart and checkpoint.exists():
        if archive_draft(checkpoint, root) is None:
            return False
    published = published_evidence(root, catalog)
    if checkpoint.exists():
        draft = read_draft(checkpoint, catalog, old_intent, old_ledger)
        if (root / "trains" / draft["trainId"]).exists():
            draft["trainId"] = next_train_id(draft["trainId"], root)
            _checkpoint(checkpoint, draft)
        print(f"Resuming saved release draft {draft['trainId']} at {checkpoint}")
    else:
        draft = {"format": DRAFT_FORMAT, "trainId": next_train_id(intent["trainId"], root),
                 "intentSha256": ctl.sha256(old_intent), "versionsSha256": ctl.sha256(old_ledger),
                 "catalogSha256": catalog["sha256"], "selected": [], "selectionConfirmed": False,
                 "answers": {}, "summary": None}
        _checkpoint(checkpoint, draft)
        print(f"Saved release draft: {checkpoint}")
    if not draft.get("selectionConfirmed", True):
        def remember_selection(selected: list[str]) -> None:
            draft["selected"] = selected
            _checkpoint(checkpoint, draft)

        selected = choose_components(catalog, ledger, root, published,
                                     initial=draft["selected"], on_change=remember_selection)
        if not selected:
            print(f"Selection paused. Your draft remains at {checkpoint}; run `just release new` to resume.")
            return False
        draft["selected"] = selected
        draft["selectionConfirmed"] = True
        _checkpoint(checkpoint, draft)
    selected = draft["selected"]
    train_id = draft["trainId"]
    changes, updated_ledger = collect_changes(selected, catalog, ledger, intent, published,
                                               draft, checkpoint)
    if draft.get("summary") is None:
        template = overall_summary_template(changes, updated_ledger)
        if "overallSummaryMode" not in draft:
            draft["overallSummaryMode"] = choose_summary_mode("Overall release", template)
            _checkpoint(checkpoint, draft)
        if draft["overallSummaryMode"] == "template":
            draft["summary"] = template
        elif draft["overallSummaryMode"] == "write":
            draft["summary"] = ask_line("Overall release summary")
        else:
            raise ctl.ReleaseError("saved overall summary choice is invalid")
        _checkpoint(checkpoint, draft)
    summary = draft["summary"]
    drafted = compose_intent(intent, train_id, changes, summary, published)
    # Validation injects ledger versions into the supplied dict. Keep the
    # stored intent version-free so the ledger remains the sole version editor.
    flow.validate_intent(json.loads(json.dumps(drafted)), catalog, updated_ledger)
    print(f"\nNew cycle {train_id}: {', '.join(selected)}")
    print(f"Summary: {summary}")
    print(f"Other unfinished changes retained as queued: {len(drafted['queued'])}")
    print("No build or publication will start. The previous intent and version ledger will be backed up on the release SSD.")
    if ask_line("Type yes to save this cycle", required=False) != "yes":
        print(f"Cycle not created. Your answers remain saved at {checkpoint}; run `just release new` to resume.")
        return False
    backup = save_cycle(root, train_id, intent_path, ledger_path, old_intent, old_ledger,
                        drafted, updated_ledger)
    os.replace(checkpoint, backup / "wizard-draft.json")
    print(f"Created release cycle {train_id}. Previous files: {backup}")
    print("Next: `just release prepare` to create owned notes and review source changes.")
    return True
