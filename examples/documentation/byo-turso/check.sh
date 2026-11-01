#!/usr/bin/env bash
# E7 byo-turso: application tables in the customer's own Turso database, Axiom-managed auth in a separate store, one migration history, disconnect
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
step "schema check" axiom database check schema.acore

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-byo, wave 2)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-byo project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
lead "connect: axiom project resources connect turso --database <url> --token-stdin < secrets-w8x/byo-turso.token (token never in argv)"
lead "ownership verification passes; GET .../resources shows provider byo-turso and billed_by turso"
step "enroll this machine's release signing key" axiom project signers add --project "$AXIOM_W8_PROJECT" --schema schema.acore
step "migrate into the BYO database" axiom database migrate schema.acore "${H[@]}" --non-interactive
step "one history" axiom database history schema.acore "${H[@]}"
lead "sign up a user through the hosted page: the user row is in the managed auth store, not in the BYO database"
lead "disconnect: axiom project resources disconnect <id>; sign-in still works (auth intact)"
finish
