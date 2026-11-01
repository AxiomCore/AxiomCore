#!/usr/bin/env bash
# E12 local-auth: local auth emulation (no network, no hosted database), mail viewer, mock OIDC, then config promotion to Cloud
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
if axiom auth emulate --help >/dev/null 2>&1; then
  lead "axiom auth emulate (loopback, issuer http://127.0.0.1:<port>/local/<project>); network blocked by the run harness"
  lead "sign up with the canary password; /local/mail shows the code; /local/oidc signs synthetic identities (--identity jo@example.com)"
  step "export holds secret references only" sh -c "axiom auth config export --from local --output work-config.json && python3 -B check_export.py work-config.json"
else lead "axiom auth emulate is not in this CLI (lane D2a)"; fi

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project local, then axq-w8x-auth development, wave 1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-auth project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
step "diff names environment, security impact and runtime target" sh -c "axiom auth config diff --from work-config.json --to env:$AXIOM_W8_ENV --json > work-diff.json && python3 -B check_export.py --diff work-diff.json"
set +e; axiom auth config push --from work-config.json --to "env:$AXIOM_W8_ENV"; rc=$?; set -e
step "push refuses unapproved widening (exit 6)" test "$rc" -eq 6
lead "write auth/config.approvals.json for the widening items, push again: exit 0"
finish
