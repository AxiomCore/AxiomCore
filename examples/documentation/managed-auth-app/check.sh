#!/usr/bin/env bash
# E1 and E2 managed-auth-app: every email method, hosted and custom UI, refresh, revocation,
# reload, cache clearing (E1); Google and GitHub through both UIs (E2).
# Examples matrix (frozen rev 4) sections 3 and 4. Lead row: needs production resources.
# Run with the installed release-candidate CLI on PATH and a clean HOME (tools/w8D2/run-row.sh).
set -euo pipefail
cd "$(dirname "$0")"
pass=0 fail=0
step() { local label=$1; shift; echo "## $label"; if "$@"; then echo "PASS $label"; pass=$((pass+1)); else echo "FAIL $label"; fail=$((fail+1)); fi; }
lead() { echo "LEAD $*"; }
finish() { echo "summary: $pass passed, $fail failed"; [ "$fail" -eq 0 ]; }
step "installed CLI" axiom --version

# Local, offline
step "backend build" sh -c "cd backend && axiom build application.acore >/dev/null"
step "guarded endpoint declares auth(...)" grep -q 'auth(required: true' backend/application.acore
step "frontend contract lock" sh -c "cd frontend && axiom contract resolve >/dev/null"
step "frontend check (web)" sh -c "cd frontend && axiom check main.acore --target web >/dev/null"

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-auth, wave 1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-auth project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
step "issuer into the manifest" axiom auth config pull "${H[@]}" --manifest frontend/AxiomDeps.toml --write
step "manifest, settings and backend agree" axiom auth check "${H[@]}" --manifest frontend/AxiomDeps.toml --backend backend/application.acore
lead "automated methods on a branch with captured mail (branches-v1 Outbound, admin GET mail): password, email code, magic link, signup verification, reset; hosted page and custom UI"
lead "G6 once per RC on the non-branch environment: every email method with real Resend delivery to the owner inbox (owner reads codes)"
lead "refresh: the old refresh token is refused and reuse ends the family; axiom auth users revoke <id> and ban <id> end the session at the next refresh"
lead "storage memory: reload signs out; storage session: reload keeps the session; sign-out empties storage and the cached /me (cacheIdentity viewer:me)"
lead "pre-hydration HTML from the host holds no session, user or token; GET /me without a token -> 401; record: no SSR in the Acore web target"
lead "E2: Google and GitHub through hosted and custom UI; account_link_required refused safely; provider secrets in no output"
lead "scans: canary_scan.py scan <canaries> web-bundle <host dist>; browser-storage <dumps>; cli \"\$HOME\" (except the 0600 session file); compiler .axiom backend/.axiom"
finish
