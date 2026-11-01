#!/usr/bin/env bash
# E12 local-auth: local auth emulation (no network, no hosted database), mail viewer, mock OIDC, then config promotion to Cloud
# Examples matrix (frozen rev 4) sections 3, 4 and 7. Local steps run anywhere; production steps are the lead's.
# Run from the unpacked example archive with the installed release-candidate CLI on PATH and a
# clean HOME (tools/w8D2/run-row.sh does both). Production steps run only with AXIOM_W8_APPLY=yes.
set -euo pipefail
cd "$(dirname "$0")"
pass=0 fail=0
step() { local label=$1; shift; echo "## $label"; if "$@"; then echo "PASS $label"; pass=$((pass+1)); else echo "FAIL $label"; fail=$((fail+1)); fi; }
lead() { echo "LEAD $*"; }
finish() { echo "summary: $pass passed, $fail failed"; [ "$fail" -eq 0 ]; }
step "installed CLI" axiom --version

# Local, offline. No `axiom login`: the emulator needs no Cloud account and allocates nothing.
rm -rf work && mkdir -m 700 work
EMU= SRV=
cleanup() {
  [ -z "$SRV" ] || kill "$SRV" 2>/dev/null || true
  [ -z "$EMU" ] || { kill "$EMU" 2>/dev/null || true; wait "$EMU" 2>/dev/null || true; }
  EMU= SRV=
}
trap cleanup EXIT
state=.axiom/auth-local/state.json
rm -f "$state"
axiom auth emulate --identity jo@example.com > work/emulate.log 2>&1 &
EMU=$!
for _ in $(seq 1 120); do [ -s "$state" ] && break; kill -0 "$EMU" 2>/dev/null || break; sleep 0.5; done
step "emulator started on loopback with the auth stream on a local SQLite file" python3 -B -c '
import json, sys
s = json.load(open(sys.argv[1]))
for k in ("issuer", "mail", "oidc"):
    assert s[k].startswith("http://127.0.0.1:"), k
assert s["issuer"].endswith("/local/local-auth") and s["store"]["stream"] == "axiom-auth" and s["store"]["version"] == "3.0.0"
assert s["store"]["file"].endswith(".axiom/auth-local/store.sqlite")
print("issuer", s["issuer"].split("/local/")[0] + "/local/<project>", "store axiom-auth", s["store"]["version"])
' "$state"
step "state only in a private .axiom/auth-local (mode 700)" sh -c 'test "$(stat -f %Lp .axiom/auth-local 2>/dev/null || stat -c %a .axiom/auth-local)" = 700'
step "sign-up code in the mail viewer, password and email-code sign-in, refresh rotation and reuse" python3 -B flow.py password "$state" work
step "mock OIDC sign-in of a synthetic identity" python3 -B flow.py oidc "$state" work

# The local runtime verifies the emulator's tokens (jwt provider, EdDSA, key by reference).
port=$(python3 -B -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
step "backend build" sh -c "cd backend && axiom build application.acore >/dev/null"
step "deployment points at the emulator" sh -c "axiom server deployment backend/.axiom/contracts/backend.axiom backend/.axiom/contracts/backend.server.json -o work/deployment.json --static-key >/dev/null 2>&1 && axiom auth emulate deployment work/deployment.json >/dev/null && grep -q '\"algorithm\": \"EdDSA\"' work/deployment.json"
AXIOM_AUTH_LOCAL_PUBLIC_KEY="$(cat .axiom/auth-local/public.pem)" axiom server run backend/.axiom/contracts/backend.axiom backend/.axiom/contracts/backend.server.json --deployment work/deployment.json --listen "127.0.0.1:$port" > work/server.log 2>&1 &
SRV=$!
for _ in $(seq 1 60); do curl -s -o /dev/null "http://127.0.0.1:$port/me" && break; sleep 0.5; done
step "guarded endpoint: 401 without a token" test "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/me")" = 401
(umask 077; printf 'Authorization: Bearer %s\n' "$(cat work/access-token)" > work/auth-header; printf 'Authorization: Bearer %sx\n' "$(cat work/access-token)" > work/forged-header)
step "guarded endpoint: 200 with the emulator's access token" sh -c "curl -sf -H @work/auth-header 'http://127.0.0.1:$port/me' | python3 -B -c 'import json,sys; v=json.load(sys.stdin); assert v[\"subject\"] and v[\"id\"]; print(\"subject and session from the verified token\")'"
step "a forged token is refused" test "$(curl -s -o /dev/null -w '%{http_code}' -H @work/forged-header "http://127.0.0.1:$port/me")" = 401

# Promotion source: public settings and secret references only.
step "export holds secret references only" sh -c "axiom auth config export --from local --output work/config.json >/dev/null && python3 -B check_export.py work/config.json"
step "no secret in emulator output or the export" sh -c "! grep -qF \"\$(cat work/password)\" work/emulate.log work/config.json work/server.log"

# Production (lead)
if [ "${AXIOM_W8_APPLY:-}" != yes ]; then
  echo "Stopped before production steps: set AXIOM_W8_APPLY=yes (lead; project local, then axq-w8x-auth development, wave 1)."
  cleanup; trap - EXIT
  step "reset deletes local users, keys and mail" sh -c "axiom auth emulate reset >/dev/null && test ! -e .axiom/auth-local"
  finish; exit $?
fi
: "${AXIOM_W8_PROJECT:?axq-w8x-auth project slug}" "${AXIOM_W8_ENV:=development}"
H=(--project "$AXIOM_W8_PROJECT" --environment "$AXIOM_W8_ENV")
step "diff names environment, security impact and runtime target" sh -c "axiom auth config diff --from local --to env:$AXIOM_W8_ENV ${H[*]} --json > work/diff.json && python3 -B check_export.py --diff work/diff.json"
set +e; axiom auth config push --from local --to "env:$AXIOM_W8_ENV" "${H[@]}"; rc=$?; set -e
step "push refuses unapproved widening (exit 6)" test "$rc" -eq 6
lead "write auth/config.approvals.json for the widening items, push again: exit 0 (the google provider item needs AUTH_PROVIDER_GOOGLE_CLIENT_SECRET in Secrets; until the backend resolves references, push refuses it with the missing_secret guidance)"
cleanup; trap - EXIT
step "reset deletes local users, keys and mail" sh -c "axiom auth emulate reset >/dev/null && test ! -e .axiom/auth-local"
finish
