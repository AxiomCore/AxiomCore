#!/usr/bin/env bash
# Container packaging without the runtime source, from a fresh public checkout, on Linux.
#
# Packages the public hello-api example for this machine's architecture from a fresh public
# clone, in a throwaway HOME, with no --runtime-source, and checks the context: input/ and a
# Dockerfile FROM the digest-pinned builder image, no source. Then it runs the image:
#   - with Docker (AXIOM_DOCKER=1): `docker buildx build` of the context, then the container
#     must serve /_axiom/ready and GET /hello. Needs a published builder image.
#   - without Docker: the Dockerfile's seal and runtime steps natively, with the
#     axiom-server-host binary the builder image holds (AXIOM_BUILDER_BIN, for example
#     extracted from the image, or built from the same reviewed source): `package-input`
#     seals the input, `verify-bundle` passes and `run-bundle` serves the same routes.
#
# Usage: AXIOM_BIN=/path/to/axiom [AXIOM_BUILDER_BIN=/path/to/axiom-server-host] \
#        [AXIOM_BUILDER_IMAGE=registry/repository@sha256:...] [AXIOM_HIDE="/dir ..."] \
#        [AXIOMCORE_REF=main] [AXIOM_DOCKER=1] validation/packaging-without-runtime-source.sh
#
# AXIOM_BUILDER_IMAGE: passed as --builder-image; by default the image the CLI release pins
# (packaging fails with AXPACKAGE_BUILDER_UNPINNED if it pins none).
# AXIOM_HIDE: directories hidden (empty tmpfs) while the CLI packages, so private checkouts
# on the machine cannot be read. Needs unprivileged user namespaces.
set -euo pipefail
: "${AXIOM_BIN:?the axiom CLI binary under test}"
case "$(uname -m)" in
  x86_64) target=container-amd64 platform=linux/amd64 ;;
  aarch64|arm64) target=container-arm64 platform=linux/arm64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 2 ;;
esac
if [ "${AXIOM_DOCKER:-}" != 1 ]; then : "${AXIOM_BUILDER_BIN:?the axiom-server-host binary of the builder image (or AXIOM_DOCKER=1)}"; fi
ref=${AXIOMCORE_REF:-main}
work=$(mktemp -d)
server='' container=''
cleanup() {
  [ -n "$server" ] && kill "$server" 2>/dev/null || true
  [ -n "$container" ] && docker rm -f "$container" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
mkdir -p "$work/home" "$work/bin"
install -m 0755 "$AXIOM_BIN" "$work/bin/axiom"
clean_env=(env -i "HOME=$work/home" "PATH=$work/bin:/usr/bin:/bin" "TMPDIR=$work" "GIT_CONFIG_GLOBAL=/dev/null" "GIT_TERMINAL_PROMPT=0" "LANG=C.UTF-8"
  "AXIOM_HIDE=${AXIOM_HIDE:-}" "AXIOM_BUILDER_IMAGE=${AXIOM_BUILDER_IMAGE:-}" "TARGET=$target")
pass=0
ok() { echo "PASS $1"; pass=$((pass+1)); }

"${clean_env[@]}" git clone -q --depth 1 --branch "$ref" https://github.com/AxiomCore/AxiomCore.git "$work/checkout"
app="$work/checkout/examples/documentation/hello-api"
ok "fresh public clone ($ref), throwaway HOME"

# Runs in the clean environment; every input comes from the environment, never spliced code.
package='set -e
for d in $AXIOM_HIDE; do mount -t tmpfs none "$d"; [ -z "$(ls -A "$d")" ]; done
builder=()
[ -n "$AXIOM_BUILDER_IMAGE" ] && builder=(--builder-image "$AXIOM_BUILDER_IMAGE")
cd "$1"
axiom build main.acore --out backend.axiom >/dev/null
axiom server deployment backend.axiom backend.server.json --output deployment.json >/dev/null
axiom server package backend.axiom backend.server.json --deployment deployment.json --target "$TARGET" "${builder[@]}" --output context > package.json
axiom server package backend.axiom backend.server.json --deployment deployment.json --target "$TARGET" --output-input input > input.json'
if [ -n "${AXIOM_HIDE:-}" ]; then
  "${clean_env[@]}" unshare -U -r -m bash -c "$package" package "$app"
  ok "packaged $target with ${AXIOM_HIDE} hidden"
else
  "${clean_env[@]}" bash -c "$package" package "$app"
  ok "packaged $target"
fi

python3 -I - "$app" "$platform" <<'EOF'
import hashlib, json, os, sys
app, platform = sys.argv[1], sys.argv[2]
out = json.load(open(os.path.join(app, "package.json")))
context = os.path.join(app, "context")
names = sorted(os.path.relpath(os.path.join(r, f), context) for r, _, fs in os.walk(context) for f in fs)
outside = [n for n in names if not n.startswith("input/")]
assert outside == [".dockerignore", "Dockerfile", "compose.yaml", "container.json"], outside
assert not any(n.endswith(".rs") or n.startswith("source") for n in names), names
dockerfile = open(os.path.join(context, "Dockerfile")).read()
froms = [l for l in dockerfile.splitlines() if l.startswith("FROM ")]
image = out["builder"]["image"]
assert froms[0] == f"FROM --platform={platform} {image} AS build", froms
assert "@sha256:" in image and len(image.rsplit(":", 1)[1]) == 64, image
assert "COPY source" not in dockerfile and "cargo" not in dockerfile
meta = json.load(open(os.path.join(context, "container.json")))
assert meta["runtimeBinary"] == "prebuilt-builder-image" and meta["builderImage"] == image
assert meta["dockerfileSha256"] == hashlib.sha256(dockerfile.encode()).hexdigest()
only = json.load(open(os.path.join(app, "input.json")))
assert only["metadata"]["inputInventorySha256"] == meta["inputInventorySha256"]
mine = sorted(os.path.relpath(os.path.join(r, f), os.path.join(app, "input")) for r, _, fs in os.walk(os.path.join(app, "input")) for f in fs)
assert mine == [n[len("input/"):] for n in names if n.startswith("input/")], mine
for n in mine:
    assert open(os.path.join(app, "input", n), "rb").read() == open(os.path.join(context, "input", n), "rb").read(), n
print("context:", len(names), "files; builder", image, f"({out['builder']['pinnedBy']})")
EOF
ok "context holds input/ and a Dockerfile FROM the digest-pinned builder, no source; --output-input matches"

port=$((20000 + RANDOM % 20000))
if [ "${AXIOM_DOCKER:-}" = 1 ]; then
  docker buildx build --platform "$platform" --load --tag "axiom-packaging-check:$port" "$app/context" >/dev/null
  ok "docker buildx build"
  container=$(docker run -d -p "127.0.0.1:$port:8080" --memory 512m --cpus 1 "axiom-packaging-check:$port")
else
  install -m 0755 "$AXIOM_BUILDER_BIN" "$work/bin/axiom-server-host"
  "$work/bin/axiom-server-host" server package-input "$app/context/input" "$work/release" >/dev/null
  ok "seal: package-input"
  "$work/release/bin/axiom" server verify-bundle "$work/release" >/dev/null
  ok "verify-bundle"
  env -i "PATH=/usr/bin:/bin" "TMPDIR=$work" "AXIOM_LISTEN=127.0.0.1:$port" K_SERVICE=packaging-check \
    AXIOM_CONTAINER_MEMORY_BYTES=536870912 AXIOM_CONTAINER_CPU_MILLIS=1000 \
    "$work/release/bin/axiom" server run-bundle "$work/release" >"$work/run.log" 2>&1 &
  server=$!
fi
for _ in $(seq 1 150); do curl -fsS "http://127.0.0.1:$port/_axiom/ready" >/dev/null 2>&1 && break; sleep 0.2; done
ready=$(curl -fsS "http://127.0.0.1:$port/_axiom/ready" || true)
[ "$ready" = '{"ready":true}' ] || { cat "$work/run.log" 2>/dev/null; [ -n "$container" ] && docker logs "$container"; exit 1; }
ok "run-bundle: /_axiom/ready"
hello=$(curl -fsS "http://127.0.0.1:$port/hello")
case "$hello" in *'Hello from Acore'*) ok "run-bundle: GET /hello" ;; *) echo "unexpected /hello: $hello" >&2; exit 1 ;; esac
echo "summary: $pass passed, 0 failed"
