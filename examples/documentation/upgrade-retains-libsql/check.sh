#!/usr/bin/env bash
# E8 upgrade-retains-libsql: Free to Pro upgrade keeps the libSQL resource and endpoint with the Free storage and request profile; same auth API
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
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-upgrade, wave 3)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-upgrade project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
step "Free resource" sh -c "axiom project resources --project $AXIOM_W8_PROJECT > work-before.json"
lead "record resource id, provider, endpoint host; sign in a test user and keep its refresh token in secrets-w8x/"
lead "upgrade to Pro with the Stripe test card: the dashboard, or axiom project resources --project $AXIOM_W8_PROJECT checkout (Stripe Checkout URL; test mode only for allowlisted projects)"
step "after upgrade" sh -c "axiom project resources --project $AXIOM_W8_PROJECT > work-after.json"
lead "same resource id, provider turso-libsql-v1, same endpoint; limits equal TierLimitsFor(pro, libsql); the refresh token still rotates"
finish
