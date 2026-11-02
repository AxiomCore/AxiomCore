#!/usr/bin/env bash
# W8.7 row runner (examples matrix section 4 "Procedure per row").
# Usage: run-row.sh <example dir name> [log name]
# Env: AXIOM_W8_TOOL_PATH (declared tool directories), AXIOM_W8_CLI_ARCHIVE (CLI release-candidate tar.gz), AXIOM_W8_EXAMPLES_ZIP_DIR (packaged
#      archives, default docs/public/examples), AXIOM_W8_CANARIES (0700 canary dir), AXIOM_W8_UI_HOST_MANIFEST (a signed
#      web UI Host candidate's host-manifest.json, installed into the clean HOME first; examples G1, O6),
#      AXIOM_W8_LOG_PREFIX (default W8D2), AXIOM_W8_SESSION_HOME (a HOME where `axiom login` already ran: its 0600 CLI
#      session files are copied into the clean HOME for production rows, copied back after check.sh, and kept out of the CLI scan), and the row's own
#      AXIOM_W8_* variables. Never pass a secret in argv.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); repo=$(cd "$here/../.." && pwd)
row=${1:?example directory name}; log=${2:-$row}
W=/Volumes/ExternalSSD/axiom-w1a
: "${AXIOM_W8_CLI_ARCHIVE:?path to the CLI release-candidate archive}"
zips=${AXIOM_W8_EXAMPLES_ZIP_DIR:-$repo/docs/public/examples}
run=$W/tmp/w8x-run/$row; logfile=$W/logs/${AXIOM_W8_LOG_PREFIX:-W8D2}-examples-$log.log
rm -rf "$run"; mkdir -p "$run/prefix/bin" "$run/home" "$W/logs"
exec > >(tee "$logfile") 2>&1
echo "row $row $(date -u +%FT%TZ)"
echo "cli archive sha256 $(shasum -a 256 "$AXIOM_W8_CLI_ARCHIVE" | cut -d' ' -f1)"
tar -xzf "$AXIOM_W8_CLI_ARCHIVE" -C "$run/prefix/bin"
echo "example archive sha256 $(shasum -a 256 "$zips/$row.zip" | cut -d' ' -f1)"
(cd "$run" && unzip -q "$zips/$row.zip")
export HOME=$run/home TMPDIR=$W/tmp CI=1 AXIOM_UI_WEB_NO_OPEN=1
export PATH=$run/prefix/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin
# Tools a row declares (Python 3.10 or later, Node 24 or later) come first: AXIOM_W8_TOOL_PATH.
[ -z "${AXIOM_W8_TOOL_PATH:-}" ] || export PATH=$AXIOM_W8_TOOL_PATH:$PATH
# Interpreter caches never land in the scanned HOME.
export PYTHONDONTWRITEBYTECODE=1 PIP_CACHE_DIR=$run/pip-cache npm_config_cache=${npm_config_cache:-$run/npm-cache}
export AXIOM_W8_SCAN="python3 -B $here/canary_scan.py scan ${AXIOM_W8_CANARIES:-/nonexistent}"
[ "${AXIOM_W8_APPLY:-}" = yes ] || export AXIOM_CLOUD_URL=http://127.0.0.1:9   # local steps reach nothing
# The CLI's session files (auth.json, access.json, both 0600) under its config directory.
session_files="auth.json access.json"
if [ -n "${AXIOM_W8_SESSION_HOME:-}" ]; then
  for rel in "Library/Application Support/axiom" ".config/axiom"; do
    for f in $session_files; do
      [ -f "$AXIOM_W8_SESSION_HOME/$rel/$f" ] || continue
      mkdir -p -m 700 "$HOME/$rel"; (umask 077; cp -p "$AXIOM_W8_SESSION_HOME/$rel/$f" "$HOME/$rel/$f"); chmod 600 "$HOME/$rel/$f"
      echo "session file copied: $rel/$f"
    done
  done
fi
if [ -n "${AXIOM_W8_UI_HOST_MANIFEST:-}" ]; then
  echo "ui host manifest sha256 $(shasum -a 256 "$AXIOM_W8_UI_HOST_MANIFEST" | cut -d' ' -f1)"
  axiom ui host install --target web --release-manifest "$AXIOM_W8_UI_HOST_MANIFEST" --non-interactive
fi
status=0
bash "$run/$row/check.sh" || status=$?
# The CLI may have rotated its refresh token during the row: hand the session back so the next row starts valid.
if [ -n "${AXIOM_W8_SESSION_HOME:-}" ]; then
  for rel in "Library/Application Support/axiom" ".config/axiom"; do
    for f in $session_files; do
      [ -f "$HOME/$rel/$f" ] && [ -d "$AXIOM_W8_SESSION_HOME/$rel" ] || continue
      (umask 077; cp -p "$HOME/$rel/$f" "$AXIOM_W8_SESSION_HOME/$rel/$f.new") && mv -f "$AXIOM_W8_SESSION_HOME/$rel/$f.new" "$AXIOM_W8_SESSION_HOME/$rel/$f"
    done
  done
fi
if [ -n "${AXIOM_W8_CANARIES:-}" ]; then
  python3 -B "$here/canary_scan.py" control "$AXIOM_W8_CANARIES" || status=1
  # CLI surface: every file the CLI wrote under the clean HOME except its own 0600 session files.
  excludes=()
  for f in $session_files; do
    find "$run/home" -type f -perm 0600 -name "$f" -print | sed "s#^$run/home/#excluded session file: #"
    excludes+=(--exclude "$f")
  done
  rsync -a "${excludes[@]}" "$run/home/" "$run/scan-home/"
  python3 -B "$here/canary_scan.py" scan "$AXIOM_W8_CANARIES" cli "$run/scan-home" || status=1
  python3 -B "$here/canary_scan.py" scan "$AXIOM_W8_CANARIES" compiler "$run/$row" || status=1
fi
echo "row $row exit $status; log $logfile"
rm -rf "$run"
exit $status
