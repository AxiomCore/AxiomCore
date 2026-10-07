#!/usr/bin/env python3
"""Reject internal/generated files anywhere in reachable public Git history."""
from pathlib import PurePosixPath
import subprocess
import sys

INTERNAL_DOCUMENTS = {
    'cli/INSPECTOR_JEV.md', 'docs/CLOUDFLARE_PAGES.md', 'docs/CONTENT_EVIDENCE.md',
    'docs/DEPLOYMENT.md', 'docs/DOCUMENTATION_CONTRACT.md', 'docs/LAUNCH_CHECKLIST.md',
    'docs/MAINTENANCE.md', 'docs/MIGRATION.md', 'docs/RELEASE_READINESS.md',
}

RETIRED_IMPLEMENTATION_BLOBS = {'acb0bebc55180a6fff866f95f1935c0f206b81d3', '178c500b1ca6c467d49ac0023e57826bddc5ed1e', '48fbee9c9639b78636a1841bcbafcb450a2085e7'}

def prohibited(path):
    return ('.summary_files' in PurePosixPath(path).parts
            or path.startswith('release/control/') or path in INTERNAL_DOCUMENTS)

def main():
    try:
        output = subprocess.check_output([
            'git', 'log', '--all', '--format=', '--name-only', '--no-renames'
        ], stderr=subprocess.PIPE).decode('utf-8')
    except (subprocess.CalledProcessError, UnicodeError):
        print('Public source check could not inspect complete Git history.', file=sys.stderr)
        return 2
    try:
        objects = subprocess.check_output(['git', 'rev-list', '--objects', '--all'],
                                          stderr=subprocess.PIPE).decode('utf-8')
    except (subprocess.CalledProcessError, UnicodeError):
        print('Public source check could not inspect reachable objects.', file=sys.stderr)
        return 2
    retired = {line.split(' ', 1)[0] for line in objects.splitlines()} & RETIRED_IMPLEMENTATION_BLOBS
    if retired:
        print(f'Public source check rejected {len(retired)} retired implementation objects. '
              'Use a fresh sanitized clone and contact the repository maintainer.', file=sys.stderr)
        return 1
    findings = {p for p in output.splitlines() if prohibited(p)}
    if findings:
        print(f'Public source check rejected {len(findings)} internal/generated paths. '
              'Use a fresh sanitized clone and contact the repository maintainer.', file=sys.stderr)
        return 1
    print('Reachable public Git history passes the file-boundary policy.')
    return 0

if __name__ == '__main__':
    sys.exit(main())
