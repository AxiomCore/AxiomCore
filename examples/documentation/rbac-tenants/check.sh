#!/usr/bin/env bash
# E5 rbac-tenants: roles, permissions, organizations, tenant selection, Permission/Role/Ownership guards, auth/policy.json with axiom auth policy check in CI, narrowing refused by the runtime
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
step "backend build" sh -c "cd backend && axiom build application.acore >/dev/null"
if axiom auth policy --help >/dev/null 2>&1; then
  set +e; axiom auth policy check --base auth/policy.base.json --policy auth/policy.json --json > work-impact.json; rc=$?; set -e
  step "policy check exits 6 for an unacknowledged widening" test "$rc" -eq 6
  step "approve the widening items" python3 -B approve.py work-impact.json work-approvals.json
  step "policy check exits 0 with the approval" axiom auth policy check --base auth/policy.base.json --policy auth/policy.json --approvals work-approvals.json --json
  set +e; axiom auth policy check --base auth/policy.base.json --policy auth/policy.narrow.json --json >/dev/null; rc=$?; set -e
  step "a narrowing without acknowledgement exits 7" test "$rc" -eq 7
else lead "axiom auth policy is not in this CLI (lane D2a)"; fi

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-rbac (libSQL) / axq-w8x-pg (PostgreSQL), wave 2 / P1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-rbac project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
step "pull the current policy" axiom auth policy pull --environment "$AXIOM_W8_ENV" --write work-policy.json
step "release" axiom auth policy release --policy auth/policy.json
lead "axiom auth policy plan --environment $AXIOM_W8_ENV --release <sha>; apply with --acknowledge-from work-approvals.json"
lead "with authz_invalidation on: apply auth/policy.narrow.json, call GET /orders with the old token within 65 s -> 401 authorization_changed; the web client refreshes once and the retry succeeds"
lead "Ownership guard: user B reading user A's order -> 403; Role guard: member calling DELETE /orders/{id} -> 403; admin -> 200"
finish
