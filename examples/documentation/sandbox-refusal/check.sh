#!/usr/bin/env bash
# E15 sandbox-refusal: a Workers (sandboxed) target is refused auth capability.
# Runs offline with the installed CLI (`axiom` on PATH); allocates nothing.
# Usage: bash check.sh   (from the unpacked example; writes only under ./work)
set -euo pipefail
cd "$(dirname "$0")"
pass=0 fail=0
ok() { echo "PASS $1"; pass=$((pass+1)); }
no() { echo "FAIL $1"; fail=$((fail+1)); }
# Synthetic IDs: the scaffold only writes the issuer URL, nothing is contacted.
PROJECT=11111111-1111-4111-8111-111111111111
ENVIRONMENT=22222222-2222-4222-8222-222222222222
rm -rf work && mkdir -p work

validate() { # <source dir> <name>: build, scaffold an axiomAuth deployment, validate for Workers
  local dir=$1 name=$2 out=work/$2
  cp -R "$dir" "$out"
  (cd "$out" && axiom build application.acore >/dev/null)
  local c=$out/.axiom/contracts/backend.axiom p=$out/.axiom/contracts/backend.server.json
  axiom server deployment "$c" "$p" -o "$out/deployment.json" --project-id "$PROJECT" --environment-id "$ENVIRONMENT" >/dev/null
  if axiom server validate "$c" "$p" --deployment "$out/deployment.json" --target cloudflare-workers-local >"$out/stdout.txt" 2>"$out/stderr.txt"; then
    echo "validate unexpectedly passed for $name"; return 1
  fi
  # The same contract is accepted for the native target: the refusal is the target's.
  axiom server validate "$c" "$p" --deployment "$out/deployment.json" --target native-server >/dev/null 2>"$out/native.txt"
}

# (1) axiomAuth on Workers v1: path deployment.authentication, requirement HS256 JWT.
if validate backend-status status \
  && grep -q 'deployment.authentication' work/status/stderr.txt && grep -q 'HS256 JWT' work/status/stderr.txt \
  && grep -q 'AXSERVER_TARGET' work/status/stderr.txt; then ok "axiomAuth refused on Workers (deployment.authentication, HS256 JWT)"; else no "axiomAuth refusal"; cat work/status/stderr.txt; fi
# Principal.* in a managed action is refused on Workers before any provider is considered.
if validate backend principal && grep -q 'Principal.\* is unavailable on Workers' work/principal/stderr.txt; then ok "Principal.* refused on Workers"; else no "Principal refusal"; cat work/principal/stderr.txt; fi
# (2) and (3), database bindings and sandbox extension bindings: see README (lead rows).
echo "summary: $pass passed, $fail failed"
rm -rf work
[ "$fail" -eq 0 ]
