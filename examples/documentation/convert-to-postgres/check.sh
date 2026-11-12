#!/usr/bin/env bash
# E9 convert-to-postgres: explicit libSQL to PostgreSQL conversion with report, receipt and cutover; Pro only (OQ-11)
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
step "PostgreSQL schema check" axiom database check schema.postgres.acore

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-upgrade, wave 3)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-upgrade project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
step "enroll this machine's release signing key" axiom project signers add --project "$AXIOM_W8_PROJECT" --schema schema.acore
step "migrate the libSQL contract" axiom database migrate schema.acore "${H[@]}" --non-interactive
step "draft a PostgreSQL proposal from the hosted head" axiom database convert draft "${H[@]}" --stream w8x.convert --out work-draft
step "plan with the reviewed PostgreSQL contract (compatibility report, signed receipt; nothing uploaded)" axiom database convert plan schema.postgres.acore "${H[@]}" --out work-conversion
lead "Pro only (conversion allowlist): axiom database convert submit --project $AXIOM_W8_PROJECT --dir work-conversion --wait; then axiom project resources request --action convert; wait for the receipt and the fence"
lead "the SQLite build's next lease is refused AXRESOURCE_REDEPLOY_REQUIRED"
lead "deploy and grant the PostgreSQL build (placement CV-3), then app reads and writes succeed; existing auth users sign in"
finish
