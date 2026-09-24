#!/usr/bin/env python3
"""Publish a staged Dart package with a scoped env-var token reference."""

from __future__ import annotations

import os
import subprocess
import sys


def main() -> int:
    if not os.environ.get("PUB_TOKEN"):
        print("release: PUB_TOKEN is required for the scoped pub.dev publisher", file=sys.stderr)
        return 2
    for command in (("dart", "pub", "token", "add", "https://pub.dev", "--env-var", "PUB_TOKEN"),
                    ("dart", "pub", "publish", "--dry-run"),
                    ("dart", "pub", "publish", "--force")):
        result = subprocess.run(command, check=False)
        if result.returncode:
            return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
