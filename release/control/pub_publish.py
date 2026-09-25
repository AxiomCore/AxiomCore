#!/usr/bin/env python3
"""Publish a staged Dart package with a short-lived pub.dev identity token."""

from __future__ import annotations

import os
import re
import subprocess
import sys


def authorized_environment() -> dict[str, str] | None:
    environment = dict(os.environ)
    service_account = environment.get("AXIOM_PUB_SERVICE_ACCOUNT", "")
    if service_account:
        if not re.fullmatch(r"[a-z][a-z0-9-]*@[a-z][a-z0-9-]*\.iam\.gserviceaccount\.com",
                            service_account):
            print("release: AXIOM_PUB_SERVICE_ACCOUNT is not a Google service account email",
                  file=sys.stderr)
            return None
        result = subprocess.run(
            ("gcloud", "auth", "print-identity-token",
             f"--impersonate-service-account={service_account}",
             "--audiences=https://pub.dev", "--include-email"),
            capture_output=True, text=True, check=False)
        if result.returncode or not result.stdout.strip():
            print("release: could not obtain pub.dev identity token; check service-account "
                  "impersonation and gcloud authentication", file=sys.stderr)
            return None
        environment["PUB_TOKEN"] = result.stdout.strip()
    elif not environment.get("PUB_TOKEN"):
        print("release: pub.dev publishing is not configured; set AXIOM_PUB_SERVICE_ACCOUNT "
              "in Infisical prod and authorize it in pub.dev package administration", file=sys.stderr)
        return None
    return environment


def run_commands(commands: tuple[tuple[str, ...], ...]) -> int:
    environment = authorized_environment()
    if environment is None:
        return 2
    for command in (("fvm", "dart", "pub", "token", "add", "https://pub.dev", "--env-var", "PUB_TOKEN"),
                    *commands):
        result = subprocess.run(command, env=environment, check=False)
        if result.returncode:
            return result.returncode
    return 0


def main() -> int:
    return run_commands((("fvm", "dart", "pub", "publish", "--dry-run"),
                         ("fvm", "dart", "pub", "publish", "--force")))


def build_main(tool: str) -> int:
    if tool not in {"dart", "flutter"}:
        print("release: unsupported Flutter build tool", file=sys.stderr)
        return 2
    return run_commands((("fvm", tool, "pub", "get"),
                         ("fvm", tool, "pub", "publish", "--dry-run")))


if __name__ == "__main__":
    arguments = sys.argv[1:]
    if not arguments:
        raise SystemExit(main())
    if len(arguments) == 2 and arguments[0] == "build":
        raise SystemExit(build_main(arguments[1]))
    print("release: unsupported pub helper command", file=sys.stderr)
    raise SystemExit(2)
