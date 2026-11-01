#!/usr/bin/env bash
# E16 preview-branch: schema and sanitized branches, test users, captured mail, reviewed promotion of a migration and a policy release
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
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-branch-free / axq-w8x-branch-pro, wave 2 / P2)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-branch-free project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
lead "branches-v1 P1: axiom branch create preview-1 --kind schema; migrate the new migration on the branch; review and promote"
lead "branches-v1 P5: --kind sanitized; masked columns hold no source values (scan the branch export with the canary scanner)"
lead "branches-v1 P7: test users and captured mail (GET mail), policy release reviewed and promoted; delete the branch, slug reusable"
finish
