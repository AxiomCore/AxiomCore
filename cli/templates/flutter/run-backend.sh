#!/usr/bin/env bash
# Check, build and run the backend on http://localhost:{{port}}.
set -euo pipefail
cd "$(dirname "$0")"
AXIOM="${AXIOM:-axiom}"
"$AXIOM" check main.acore
"$AXIOM" build main.acore --out backend.axiom
"$AXIOM" server deployment backend.axiom backend.server.json --output deployment.json
exec "$AXIOM" run main.acore --deployment deployment.json --port "${PORT:-{{port}}}"
