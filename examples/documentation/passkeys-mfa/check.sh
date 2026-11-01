#!/usr/bin/env bash
# E10 passkeys-mfa: TOTP and recovery codes, passkeys through Chrome's virtual authenticator, reauthentication, an assurance-guarded endpoint
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
step "web client build" sh -c "cd web && npm install --no-audit --no-fund --silent \"${AXIOM_AUTH_WEB_TGZ:?}\" && node --disable-warning=ExperimentalWarning build.mjs >/dev/null"

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-auth, wave 1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-auth project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
lead "settings: mfa_policy optional, mfa_factors [totp, passkey], webauthn_rp_id localhost, webauthn_origins and allowed_origins http://localhost:5177"
lead "browser (tools/w8D2, Playwright, channel chrome, WebAuthn.addVirtualAuthenticator ctap2 internal uv): sign up, enrollTotp, confirm with a code computed from the canary seed, keep the recovery codes in secrets-w8x/"
lead "GET /admin with the password-only token -> 401 step_up_required (acr_values=aal2); stepUp totp -> 200"
lead "registerPasskey; sign out; signInWithPasskey -> acr phr; a recovery code works once, the second use -> invalid_code"
finish
