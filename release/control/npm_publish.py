#!/usr/bin/env python3
"""Run npm publish with a short-lived user config outside the release SSD."""

from __future__ import annotations

import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile


def main() -> int:
    token = os.environ.get("NPM_TOKEN")
    if not token or "\n" in token or "\r" in token:
        print("release: NPM_TOKEN is missing or malformed", file=sys.stderr)
        return 2
    temporary = Path("/private/tmp" if platform.system() == "Darwin" else "/tmp")
    descriptor, filename = tempfile.mkstemp(prefix="axiom-npm-auth-", dir=temporary)
    config = Path(filename)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w") as output:
            output.write("//registry.npmjs.org/:_authToken=${NPM_TOKEN}\n")
        env = os.environ.copy()
        env["NPM_CONFIG_USERCONFIG"] = str(config)
        return subprocess.run(sys.argv[1:], env=env, check=False).returncode
    finally:
        config.unlink(missing_ok=True)


if __name__ == "__main__":
    raise SystemExit(main())
