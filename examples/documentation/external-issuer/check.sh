#!/usr/bin/env bash
# E6 external-issuer: trusted external token exchange and the Supabase, Auth0, Clerk presets; Microsoft real, Apple mock
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

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-issuer, wave 1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-issuer project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
for preset in supabase auth0 clerk; do
  var=AXIOM_W8_${preset^^}_ISSUER; aud=AXIOM_W8_${preset^^}_AUDIENCE
  step "trusted issuer $preset" axiom auth trusted-issuers add --preset "$preset" --issuer "${!var:?}" --audience "${!aud:?}" "${H[@]}"
  lead "obtain a $preset test-app token into secrets-w8x/$preset.jwt (0600), then: axiom auth trusted-issuers test <id> --token-file secrets-w8x/$preset.jwt"
done
lead "Microsoft: axiom auth providers set microsoft --client-id ... --tenant ... --client-secret-stdin < secrets-w8x/microsoft.secret"
lead "Apple (mock issuer): axiom auth providers set apple --client-id ... --team-id ... --key-id ... --private-key-file secrets-w8x/apple.p8"
lead "disable one trusted issuer; the exchanged session is refused at the next refresh (invalid_refresh)"
finish
