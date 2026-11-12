#!/usr/bin/env bash
# E14 linux-backend: Acore backend container on Linux (Cloud Run) with axiomAuth and runtime-data credentials injected by Secrets only; lease expiry and revocation
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
step "plan and axiomAuth deployment scaffold" sh -c "cd backend && rm -f work-*.json && axiom server deployment .axiom/contracts/backend.axiom .axiom/contracts/backend.server.json -o work-deployment.json --project-id ${AXIOM_W8_PROJECT_ID:-11111111-1111-4111-8111-111111111111} --environment-id ${AXIOM_W8_ENVIRONMENT_ID:-22222222-2222-4222-8222-222222222222} >/dev/null && python3 -B ../check_scaffold.py work-deployment.json"
step "native target accepts the contract" sh -c "cd backend && axiom server validate .axiom/contracts/backend.axiom .axiom/contracts/backend.server.json --deployment work-deployment.json --target native-server >/dev/null"

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-hosted / axq-w8x-pg, wave 1 / P1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-hosted project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
if [ -n "${AXIOM_W8_PROJECT_ID:-}" ] && [ -n "${AXIOM_W8_ENVIRONMENT_ID:-}" ]; then
  step "deployment: axiomAuth for this environment, no credential" sh -c "cd backend && rm -f deployment.json && axiom server deployment .axiom/contracts/backend.axiom .axiom/contracts/backend.server.json -o deployment.json --project-id $AXIOM_W8_PROJECT_ID --environment-id $AXIOM_W8_ENVIRONMENT_ID >/dev/null && python3 -B ../check_scaffold.py deployment.json"
  if [ -n "${AXIOM_W8_RUNTIME_SOURCE:-}" ] && [ -n "${AXIOM_W8_CONTEXT_OUT:-}" ]; then
    step "container-amd64 build context" sh -c "cd backend && axiom server package .axiom/contracts/backend.axiom .axiom/contracts/backend.server.json --deployment deployment.json --target container-amd64 --runtime-source $AXIOM_W8_RUNTIME_SOURCE --output $AXIOM_W8_CONTEXT_OUT >/dev/null"
  fi
fi
lead "build the container-amd64 context on Cloud Build (docker buildx --platform linux/amd64; no local Docker on the 8 GB machine); a backend with a hosted database adds --environment <env> --secret-root (hosted-persistence)"
lead "deploy to Cloud Run as axq-w8x-e14; the deployment JSON and image hold no credential (scan the image tarball and the service JSON)"
lead "a token of another environment -> 401; after the lease TTL a copied lease -> 401; after revocation -> 401"
finish
