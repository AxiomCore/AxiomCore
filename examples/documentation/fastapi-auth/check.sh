#!/usr/bin/env bash
# E13 fastapi-auth checks (examples matrix 4).
#   Local (no network beyond package installs): verifier and API tests, frontend build from the
#   installed @axiomcore/auth-web tarball, bundle scan.
#   Lead row (AXIOM_W8_APPLY=yes): real sign-in against a Free libSQL environment; see README.
# Inputs: AXIOM_AUTH_WEB_TGZ (path to the packed tarball), optional AXIOM_W8_SCAN (scanner command,
# called as `$AXIOM_W8_SCAN <surface> <path>...`), optional PYTHON (default python3).
set -euo pipefail
cd "$(dirname "$0")"
: "${AXIOM_AUTH_WEB_TGZ:?path to axiomcore-auth-web-<version>.tgz}"
PY=${PYTHON:-python3}
pass=0 fail=0
step() { local label=$1; shift; if "$@"; then echo "PASS $label"; pass=$((pass+1)); else echo "FAIL $label"; fail=$((fail+1)); fi; }
rm -rf work && mkdir -p work

if ! "$PY" -c 'import fastapi, cryptography, httpx' 2>/dev/null; then
  "$PY" -m venv work/venv && work/venv/bin/pip install -q -r backend/requirements.txt httpx
  PY=$PWD/work/venv/bin/python
fi
step "verifier refuses wrong iss, aud, expired, alg none, HS256, OAuth tokens; API answers 401 and 200"   sh -c "cd backend && "$PY" -B -m unittest -q test_verify"

cp -R frontend work/frontend
step "install @axiomcore/auth-web from the tarball" sh -c "cd work/frontend && npm install --no-audit --no-fund --silent '$AXIOM_AUTH_WEB_TGZ'"
step "build the web client" sh -c "cd work/frontend && AXIOM_ISSUER=https://auth.axiomcore.dev/x/y AXIOM_AUDIENCE=https://auth.axiomcore.dev/x/y/api npm run -s build >/dev/null"
step "bundle holds the installed client verbatim" cmp -s work/frontend/dist/auth-web.js work/frontend/node_modules/@axiomcore/auth-web/dist/index.js
if [ -n "${AXIOM_W8_SCAN:-}" ]; then
  step "bundle canary scan" $AXIOM_W8_SCAN web-bundle work/frontend/dist
fi
echo "summary: $pass passed, $fail failed"
rm -rf work
[ "$fail" -eq 0 ]
