#!/usr/bin/env python3
"""W8.7 canary scanner (examples matrix section 5). Counts and file names only.

  canary_scan.py new <dir>                      create synthetic canaries (dir 0700, files 0600)
  canary_scan.py add <dir> <name>               store a run value (access token, lease, ...) read from stdin
  canary_scan.py scan <dir> <surface> <path>... scan files; exit 1 on any hit
  canary_scan.py control <dir>                  positive control: every canary and pattern must be found

A canary is one file in <dir> holding one value. Values are matched exactly (also URL- and
JSON-escaped) on local files; no value, line or match is ever printed, only counts per
canary name or pattern and the relative file names. Patterns are those of w7-cr0.md:8.
"""
import json
import os
import re
import secrets
import stat
import string
import sys
import tempfile
import urllib.parse
from pathlib import Path

PATTERNS = {
    "jwt": rb"eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}",
    "lease_token": rb"ax[ds]l_[A-Za-z0-9_-]{20,}",
    "refresh_token_value": rb"axr1_[A-Za-z0-9_-]{8,}\.[0-9]+\.",
    "mfa_ticket": rb"axs1[._][A-Za-z0-9_-]{20,}",
    "credential_url": rb"[a-z][a-z0-9+.-]*://[^:/\s\"'@]+:[^@/\s\"']+@",
    "auth_token_param": rb"authToken=[^&\s\"']{8,}",
    # A JSON member or a form/query parameter with a value; code such as `refresh_token: x.y` is not one.
    "token_field": rb"(\"(refresh_token|access_token|id_token)\"\s*:\s*\"[A-Za-z0-9._~+/-]{16,}\"|\b(refresh_token|access_token|id_token)=[A-Za-z0-9._~+/-]{16,})",
    "stripe_key": rb"(sk|rk)_(live|test)_[A-Za-z0-9]{10,}",
    "resend_key": rb"re_[A-Za-z0-9]{8,}_[A-Za-z0-9]{8,}",
    "otpauth_secret": rb"otpauth://[^\s\"']*secret=[A-Z2-7]{16,}",
}
SAMPLES = {  # one synthetic match per pattern, for the positive control
    "jwt": b"eyJhbGciOiJFZERTQSJ9.eyJzdWIiOiJjb250cm9sIn0.c2ln",
    "lease_token": b"axdl_" + b"C" * 24,
    "refresh_token_value": b"axr1_controlcontrol.3.tail",
    "mfa_ticket": b"axs1." + b"c" * 24,
    "credential_url": b"postgres://control:control@127.0.0.1/db",
    "auth_token_param": b"?authToken=controlvalue1",
    "token_field": b'"refresh_token":"' + b"c" * 20 + b'"',
    "stripe_key": b"sk_test_" + b"c" * 12,
    "resend_key": b"re_controlxx_controlxx",
    "otpauth_secret": b"otpauth://totp/App:x?secret=" + b"A" * 16,
}
MAX_FILE = 64 << 20


def private_dir(d: Path) -> Path:
    st = d.stat()
    if not stat.S_ISDIR(st.st_mode) or st.st_mode & 0o077:
        sys.exit(f"canary dir must be a 0700 directory: {d}")
    return d


def canaries(d: Path) -> dict[str, list[bytes]]:
    out = {}
    for f in sorted(private_dir(d).iterdir()):
        if not f.is_file() or f.name.startswith("."):
            continue
        if f.stat().st_mode & 0o077:
            sys.exit(f"canary file must be 0600: {f.name}")
        value = f.read_text().strip()
        if len(value) < 6:
            sys.exit(f"canary too short to match safely: {f.name}")
        forms = {value, urllib.parse.quote(value, safe=""), json.dumps(value)[1:-1]}
        out[f.name] = sorted({v.encode() for v in forms})
    return out


def write_private(path: Path, value: str) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as fh:
        fh.write(value)


def new(d: Path) -> None:
    d.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(d, 0o700)
    alnum = string.ascii_letters + string.digits
    rand = lambda n, alphabet=alnum: "".join(secrets.choice(alphabet) for _ in range(n))
    crockford = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    values = {
        "password": "W8x-" + rand(20),
        "email_code_challenge": rand(32),
        "totp_seed": rand(32, "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"),
        "mock_client_secret": "mcs_" + rand(32),
        "mock_smtp_password": "smtp-" + rand(24),
    }
    for i in range(10):
        values[f"recovery_code_{i}"] = rand(5, crockford) + "-" + rand(5, crockford)
    for name, value in values.items():
        write_private(d / name, value)
    print(f"created {len(values)} canaries in {d}")


def scan_files(paths: list[str]):
    for root in paths:
        p = Path(root)
        files = [p] if p.is_file() else sorted(x for x in p.rglob("*") if x.is_file() and not x.is_symlink())
        for f in files:
            if f.stat().st_size <= MAX_FILE:
                yield (f.relative_to(p) if p.is_dir() else Path(f.name)), f.read_bytes()


def scan(d: Path, surface: str, paths: list[str]) -> dict:
    found = canaries(d)
    counts: dict[str, int] = {}
    files: dict[str, set] = {}
    n = 0
    for rel, data in scan_files(paths):
        n += 1
        for name, forms in found.items():
            c = sum(data.count(v) for v in forms)
            if c:
                counts["canary:" + name] = counts.get("canary:" + name, 0) + c
                files.setdefault("canary:" + name, set()).add(str(rel))
        for name, rx in PATTERNS.items():
            c = len(re.findall(rx, data))
            if c:
                counts["pattern:" + name] = counts.get("pattern:" + name, 0) + c
                files.setdefault("pattern:" + name, set()).add(str(rel))
    return {"surface": surface, "files_scanned": n, "hits": sum(counts.values()), "counts": counts,
            "files": {k: sorted(v) for k, v in files.items()}}


def control(d: Path) -> int:
    found = canaries(d)
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as tmp:
        planted = Path(tmp) / "planted.txt"
        fd = os.open(planted, os.O_WRONLY | os.O_CREAT, 0o600)
        with os.fdopen(fd, "wb") as fh:
            for forms in found.values():
                fh.write(forms[0] + b"\n")
            for sample in SAMPLES.values():
                fh.write(sample + b"\n")
        report = scan(d, "positive-control", [tmp])
    missing = [k for k in [*("canary:" + n for n in found), *("pattern:" + n for n in PATTERNS)] if k not in report["counts"]]
    print(json.dumps({"surface": "positive-control", "expected": len(found) + len(PATTERNS), "found": len(report["counts"]), "missing": missing}))
    return 0 if not missing else 2


def main(argv: list[str]) -> int:
    if len(argv) >= 2 and argv[0] == "new":
        new(Path(argv[1]))
        return 0
    if len(argv) == 3 and argv[0] == "add":
        name = argv[2]
        if not re.fullmatch(r"[a-z0-9_]{1,64}", name):
            sys.exit("name: [a-z0-9_]")
        write_private(private_dir(Path(argv[1])) / name, sys.stdin.read().strip())
        return 0
    if len(argv) >= 4 and argv[0] == "scan":
        report = scan(Path(argv[1]), argv[2], argv[3:])
        print(json.dumps(report, sort_keys=True))
        return 1 if report["hits"] else 0
    if len(argv) == 2 and argv[0] == "control":
        return control(Path(argv[1]))
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
