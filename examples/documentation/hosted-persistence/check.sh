#!/usr/bin/env bash
# E3/E4 hosted-persistence: one Acore web plus backend app on an Axiom-managed database: hosted migrate, runtime-data through the gateway with a Secrets lease, guarded writes
# Examples matrix (frozen rev 4) sections 3 and 4. Lead row: needs production resources.
# Run from the unpacked example archive with the installed release-candidate CLI on PATH and a
# clean HOME (tools/w8D2/run-row.sh does both). Production steps run only with AXIOM_W8_APPLY=yes.
set -euo pipefail
cd "$(dirname "$0")"
pass=0 fail=0
step() { local label=$1; shift; echo "## $label"; if "$@"; then echo "PASS $label"; pass=$((pass+1)); else echo "FAIL $label"; fail=$((fail+1)); fi; }
lead() { echo "LEAD $*"; }
finish() { echo "summary: $pass passed, $fail failed"; [ "$fail" -eq 0 ]; }
step "installed CLI" axiom --version

# Local, offline
step "schema check (libSQL)" axiom database check schema.acore
step "backend build" sh -c "cd backend && axiom build application.acore >/dev/null"

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-hosted (E3) / axq-w8x-pg (E4), wave 1 / P1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-hosted project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
ENGINE=${AXIOM_W8_ENGINE:-sqlite}   # postgresql for E4
[ "$ENGINE" = postgresql ] && cp schema.postgres.acore schema.acore
step "status" axiom database status schema.acore "${H[@]}"
step "migrate" axiom database migrate schema.acore "${H[@]}" --non-interactive
step "history" axiom database history schema.acore "${H[@]}"
lead "deploy backend/ with the E14 recipe (runtime-data credential by Secrets only), then:"
lead "  curl POST /notes without a token -> 401; with a signed-in token -> 201; GET /notes -> the row"
lead "  copy the gateway lease out of the container env, wait for expiry and after revocation: 401 (AXRESOURCE lease refused)"
lead "  scan: canary_scan.py scan <canaries> cli \"$HOME\" .axiom backend/.axiom"
finish
