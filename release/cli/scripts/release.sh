#!/bin/bash
set -e

CLI_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$CLI_ROOT/dist"
RELEASE_DIR="$CLI_ROOT/release"

mkdir -p "$RELEASE_DIR"

# Detect platform for naming the tarball
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

# FIX: Map 'darwin' kernel name to 'macos' for consistent naming
if [ "$OS" == "darwin" ]; then
    OS="macos"
fi

case "$ARCH" in
    x86_64)       ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
    *)            ARCH="arm64" ;;
esac

PLATFORM="${OS}-${ARCH}"

echo "📦 Packaging Axiom CLI for $PLATFORM..."
cd "$DIST_DIR"

if [ ! -f "axiom" ]; then
    echo "❌ Error: axiom binary not found in $DIST_DIR."
    exit 1
fi

tar -czf "$RELEASE_DIR/axiom-${PLATFORM}.tar.gz" axiom
cd "$CLI_ROOT"

echo "✅ Checksum generated:"
shasum -a 256 "$RELEASE_DIR/axiom-${PLATFORM}.tar.gz"
