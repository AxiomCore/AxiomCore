#!/usr/bin/env python3
"""Compare the commands an Axiom CLI binary offers with the commands the docs show.

The script walks `axiom --help` recursively (hidden commands and `help` are
skipped), then reads every page under `content/docs` and collects each
`axiom ...` invocation written in a fenced code block or an inline code span.
A command path counts as documented when some invocation reaches it, so
`axiom auth users list` documents `auth`, `auth users` and `auth users list`.
Options between command words are skipped, for example
`axiom auth --environment production users list`.

Usage (from the docs directory):

    python3 tools/docs-vs-binary.py --axiom /path/to/axiom
    python3 tools/docs-vs-binary.py --axiom axiom --json

Exit status: 0 when every binary command is documented, 1 when some are not,
2 on a usage or binary error. Docs invocations whose first word is not a
command of the binary are listed for review but do not change the exit status
(hidden and aliased commands are not visible in `--help`).
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import pathlib
import re
import shlex
import subprocess
import sys

DOCS_DEFAULT = pathlib.Path(__file__).resolve().parent.parent / "content" / "docs"
COMMANDS_HEADER = re.compile(r"^(Commands|Subcommands):\s*$")
COMMAND_LINE = re.compile(r"^  ([a-z0-9][a-z0-9-]*)(?:,\s*[a-z0-9-]+)*(?:\s{2,}.*)?$")
FENCE = re.compile(r"^(\s*)(`{3,}|~{3,})")
INLINE_CODE = re.compile(r"(`+)(.+?)\1", re.S)
AXIOM_WORD = re.compile(r"(?<![\w./-])axiom(?=\s)")


def parse_subcommands(help_text: str) -> list[str]:
    """Return the subcommand names listed under `Commands:` in clap help output."""
    names: list[str] = []
    in_section = False
    for line in help_text.splitlines():
        if COMMANDS_HEADER.match(line):
            in_section = True
            continue
        if in_section:
            if not line.strip():
                break
            match = COMMAND_LINE.match(line)
            if match:
                if match.group(1) != "help":
                    names.append(match.group(1))
            elif not line.startswith("    "):
                # A wrapped description is indented further; anything else ends the list.
                break
    return names


def run_help(axiom: str, path: tuple[str, ...]) -> str:
    result = subprocess.run(
        [axiom, *path, "--help"],
        capture_output=True,
        text=True,
        timeout=60,
        env={**os.environ, "NO_COLOR": "1", "TERM": "dumb"},
    )
    if result.returncode != 0:
        raise RuntimeError(f"`axiom {' '.join(path)} --help` exited {result.returncode}: {result.stderr.strip()[:200]}")
    return result.stdout


def walk_binary(axiom: str, jobs: int = 8) -> dict[tuple[str, ...], list[str]]:
    """Map every visible command path (including the root `()`) to its subcommands."""
    tree: dict[tuple[str, ...], list[str]] = {}
    frontier: list[tuple[str, ...]] = [()]
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        while frontier:
            results = list(pool.map(lambda p: (p, parse_subcommands(run_help(axiom, p))), frontier))
            frontier = []
            for path, children in results:
                tree[path] = children
                frontier.extend(path + (child,) for child in children)
    return tree


def code_fragments(text: str) -> list[str]:
    """Fenced code block bodies and inline code spans of one Markdown/MDX page."""
    fragments: list[str] = []
    prose: list[str] = []
    block: list[str] | None = None
    fence = ""
    for line in text.splitlines():
        match = FENCE.match(line)
        if block is None:
            if match:
                block, fence = [], match.group(2)
            else:
                prose.append(line)
        elif match and match.group(2).startswith(fence[0]) and len(match.group(2)) >= len(fence) and not line.strip()[len(match.group(2)):].strip():
            fragments.append("\n".join(block))
            block = None
        else:
            block.append(line)
    if block is not None:
        fragments.append("\n".join(block))
    for match in INLINE_CODE.finditer("\n".join(prose)):
        fragments.append(match.group(2).replace("\n", " "))
    return fragments


def invocations(fragment: str) -> list[list[str]]:
    """Token lists that follow each `axiom` word in a code fragment."""
    fragment = re.sub(r"\\\r?\n\s*", " ", fragment)  # shell line continuations
    out: list[list[str]] = []
    for line in fragment.splitlines():
        for match in AXIOM_WORD.finditer(line):
            rest = re.split(r"[|;&<>`)]|\$\(", line[match.end():], maxsplit=1)[0]
            try:
                tokens = shlex.split(rest, comments=True)
            except ValueError:
                tokens = rest.split()
            out.append(tokens)
    return out


def resolve(tree: dict[tuple[str, ...], list[str]], tokens: list[str]) -> tuple[tuple[str, ...], str | None]:
    """Follow tokens through the command tree, skipping options and their values.

    Returns the deepest command path reached and, when the first word after
    `axiom` is not a command, that word (for the review list).
    """
    path: tuple[str, ...] = ()
    after_option = False
    for token in tokens:
        children = tree.get(path, [])
        if token.startswith("-"):
            after_option = "=" not in token
            continue
        if token in children:
            path += (token,)
            after_option = False
            continue
        if after_option:
            after_option = False
            continue
        break
    unknown = None
    if not path and tokens and not tokens[0].startswith("-"):
        word = tokens[0]
        if re.fullmatch(r"[a-z][a-z0-9-]*", word):
            unknown = word
    return path, unknown


def scan_docs(docs: pathlib.Path, tree: dict[tuple[str, ...], list[str]]):
    documented: dict[tuple[str, ...], set[str]] = {}
    unknown: dict[str, set[str]] = {}
    for page in sorted(list(docs.rglob("*.mdx")) + list(docs.rglob("*.md"))):
        rel = page.relative_to(docs).as_posix()
        for fragment in code_fragments(page.read_text(encoding="utf-8")):
            for tokens in invocations(fragment):
                path, word = resolve(tree, tokens)
                for depth in range(1, len(path) + 1):
                    documented.setdefault(path[:depth], set()).add(rel)
                if word:
                    unknown.setdefault(word, set()).add(rel)
    return documented, unknown


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--axiom", default="axiom", help="Axiom CLI binary to inspect (default: axiom on PATH)")
    parser.add_argument("--docs", type=pathlib.Path, default=DOCS_DEFAULT, help="docs content directory (default: content/docs)")
    parser.add_argument("--json", action="store_true", help="print a JSON report")
    parser.add_argument("--jobs", type=int, default=8, help="parallel --help calls (default 8)")
    args = parser.parse_args(argv)

    try:
        version = subprocess.run([args.axiom, "--version"], capture_output=True, text=True, timeout=60).stdout.strip()
        tree = walk_binary(args.axiom, args.jobs)
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"docs-vs-binary: {error}", file=sys.stderr)
        return 2
    if not args.docs.is_dir():
        print(f"docs-vs-binary: {args.docs} is not a directory", file=sys.stderr)
        return 2

    documented, unknown = scan_docs(args.docs, tree)
    commands = sorted(path for path in tree if path)
    missing = [path for path in commands if path not in documented]
    top_level = set(tree[()]) | {"help"}
    unknown = {word: pages for word, pages in unknown.items() if word not in top_level}

    if args.json:
        print(json.dumps({
            "binary": version,
            "commands": len(commands),
            "documented": len(commands) - len(missing),
            "undocumented": [" ".join(p) for p in missing],
            "docs_words_not_in_binary": {w: sorted(p) for w, p in sorted(unknown.items())},
        }, indent=2))
    else:
        print(f"{version}: {len(commands)} commands, {len(commands) - len(missing)} documented, {len(missing)} undocumented")
        for path in missing:
            print(f"UNDOCUMENTED\taxiom {' '.join(path)}")
        for word, pages in sorted(unknown.items()):
            print(f"REVIEW\taxiom {word}\t(not a visible top-level command; {', '.join(sorted(pages))})")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
