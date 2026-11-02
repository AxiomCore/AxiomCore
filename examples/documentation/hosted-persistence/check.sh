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
step "backend build (hosted PostgreSQL storage, E4)" sh -c "cd backend && axiom build hosted.postgres.acore >/dev/null && rm -rf .axiom"
step "backend build (hosted libSQL storage)" sh -c "cd backend && axiom build hosted.acore >/dev/null"

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-hosted (E3) / axq-w8x-pg (E4), wave 1 / P1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-hosted project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
ENGINE=${AXIOM_W8_ENGINE:-sqlite}   # postgresql for E4
BACKEND=hosted.acore PROVIDER=libsql
if [ "$ENGINE" = postgresql ]; then
  cp schema.postgres.acore schema.acore; BACKEND=hosted.postgres.acore PROVIDER=postgres
  (cd backend && rm -rf .axiom && axiom build hosted.postgres.acore >/dev/null)
fi
step "status" axiom database status schema.acore "${H[@]}"
step "enroll this machine's release signing key" axiom project signers add --project "$AXIOM_W8_PROJECT" --schema schema.acore
step "migrate" axiom database migrate schema.acore "${H[@]}" --non-interactive
step "history" axiom database history schema.acore "${H[@]}"
# Linux container (E14 recipe): the notes credential is leased by the service account through
# the data gateway. The scaffold gives postgres a local signed-binding configuration; a hosted
# database binding has none (the lease supplies endpoint, login and head), so it is cleared.
# the data gateway; AxiomSecrets.toml and the deployment hold references only.
if [ -n "${AXIOM_W8_PROJECT_ID:-}" ] && [ -n "${AXIOM_W8_ENVIRONMENT_ID:-}" ] && [ -n "${AXIOM_W8_RESOURCE_ID:-}" ]; then
  cat > backend/AxiomSecrets.toml <<TOML
format = "axiom-secret-bindings/v1"

[environments.$AXIOM_W8_ENV.cloud]
identity = "google-metadata"
project_id = "$AXIOM_W8_PROJECT_ID"
environment_id = "$AXIOM_W8_ENVIRONMENT_ID"
data_gateway = "${AXIOM_W8_DATA_GATEWAY:-https://axiom-data-gateway-617539083629.asia-southeast1.run.app}"

[environments.$AXIOM_W8_ENV.secrets.notesDb]
source = "axiom-cloud-database"
resource = "$AXIOM_W8_RESOURCE_ID"
TOML
  step "deployment: axiomAuth for this environment, $PROVIDER notes binding, explicit grants" sh -c "cd backend && rm -f deployment.json && axiom server deployment .axiom/contracts/backend.axiom .axiom/contracts/backend.server.json -o deployment.json --project-id $AXIOM_W8_PROJECT_ID --environment-id $AXIOM_W8_ENVIRONMENT_ID >/dev/null && python3 -B -c 'import json; p=\"deployment.json\"; d=json.load(open(p)); b=d[\"bindings\"][\"notes\"]; assert b[\"provider\"]==\"$PROVIDER\" and b.get(\"secrets\")=={\"notesDb\": \"secret:notesDb\"} and d[\"authentication\"][\"kind\"]==\"axiomAuth\"; b[\"configuration\"]={}; d[\"grants\"]=[\"notes.create\",\"notes.list\"]; json.dump(d, open(p,\"w\"), indent=2)'"
  if [ -n "${AXIOM_W8_RUNTIME_SOURCE:-}" ] && [ -n "${AXIOM_W8_CONTEXT_OUT:-}" ]; then
    step "container-amd64 build context (value-free sealed secrets)" sh -c "cd backend && axiom server package .axiom/contracts/backend.axiom .axiom/contracts/backend.server.json --deployment deployment.json --target container-amd64 --environment $AXIOM_W8_ENV --secret-root . --runtime-source $AXIOM_W8_RUNTIME_SOURCE --output $AXIOM_W8_CONTEXT_OUT >/dev/null"
  else lead "package: axiom server package ... --target container-amd64 --environment $AXIOM_W8_ENV --secret-root backend --runtime-source <workspace> --output <context>"; fi
fi
lead "build the context on Cloud Build (docker buildx --platform linux/amd64), deploy to Cloud Run with its own service account; register that account: axiom secrets register-workload, axiom secrets grant (requirement notesDb, consumer server)"
lead "  curl POST /notes without a token -> 401; user A -> 201, GET /notes -> the row; user B GET /notes -> no row; user B writing an id user A already used -> 201 (ids are per user, schema 1.1.0 owned_notes keyed on (owner, id)); the same id twice for one user -> 409 duplicate"
lead "  revoke the grant (axiom secrets revoke-grant): the next lease is refused and /notes answers 503; a copied lease after its TTL -> 401"
lead "  scan: canary_scan.py scan <canaries> cli \"$HOME\" .axiom backend/.axiom <context>/input"
finish
