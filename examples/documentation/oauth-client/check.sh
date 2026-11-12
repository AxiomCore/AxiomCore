#!/usr/bin/env bash
# E11 oauth-client: an OAuth application signing in through the Axiom OIDC issuer: authorization code with PKCE, ID token verified, consent and logout
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
step "client unit test (PKCE, ID token signature and claim checks)" sh -c "cd client && python3 -B -m unittest -q test_rp"

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project axq-w8x-auth, wave 1)."
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-auth project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
step "OIDC discovery and JWKS" axiom auth oidc check "${H[@]}"
step "register the client" axiom auth oauth-apps create --name "W8 example" --type public --redirect-uri http://127.0.0.1:5178/callback --scope openid --scope email "${H[@]}" --json
lead "python3 -B client/rp.py --issuer <issuer> --client-id <axcl_...> [--out <private dir>]: sign in and allow in the browser; rp.py checks state, iss, the code exchange with the PKCE verifier, the ID token signature (JWKS) and iss, aud, nonce, exp, a tampered token refused, userinfo, code replay refused and the access token revoked after it"
lead "GET /v1/consents lists the client; revoke; the next authorize shows consent again"
finish
