#!/bin/bash
set -e

EXTRACTOR_NAME=$1
VERSION=$2

if [ -z "$EXTRACTOR_NAME" ] || [ -z "$VERSION" ]; then
    echo "❌ Usage: ./publish.sh <extractor-name> <version-tag>"
    echo "Example: ./publish.sh axiom-fastapi v0.1.0"
    echo "Example: ./publish.sh axiom-go-extractor v0.1.0"
    exit 1
fi

# --- Resolve paths safely (independent of where script is run) ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

echo "📁 SCRIPT_DIR: $SCRIPT_DIR"
echo "📁 REPO_ROOT: $REPO_ROOT"

# --- Platform detection ---
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
esac

PLATFORM="${OS}-${ARCH}"
ASSET_NAME="${EXTRACTOR_NAME}-${PLATFORM}"
BINARY_PATH=""

# ============================================================
# 🐹 GO EXTRACTOR
# ============================================================
if [ "$EXTRACTOR_NAME" == "axiom-go-extractor" ]; then
    GO_DIR="$REPO_ROOT/axiom-extractor/extractors/go"

    echo "🐹 Building Go Extractor '$EXTRACTOR_NAME' for $PLATFORM..."
    echo "📂 GO_DIR: $GO_DIR"

    if [[ ! -d "$GO_DIR" ]]; then
        echo "❌ Go extractor directory not found: $GO_DIR"
        exit 1
    fi

    cd "$GO_DIR"

    go mod tidy

    mkdir -p dist
    go build -o "dist/$EXTRACTOR_NAME" ./cmd/axiom-go-extractor

    BINARY_PATH="$GO_DIR/dist/$EXTRACTOR_NAME"

# ============================================================
# 🐍 PYTHON EXTRACTORS
# ============================================================
elif [[ "$EXTRACTOR_NAME" == axiom-* ]]; then
    SAFE_NAME="${EXTRACTOR_NAME//-/_}"

    EXTRACTOR_DIR="$REPO_ROOT/axiom-extractor/extractors/python/frameworks/$SAFE_NAME"

    echo "🐍 Building Python Extractor '$EXTRACTOR_NAME' for $PLATFORM..."
    echo "📂 EXTRACTOR_DIR: $EXTRACTOR_DIR"

    if [[ ! -d "$EXTRACTOR_DIR" ]]; then
        echo "❌ Extractor directory not found: $EXTRACTOR_DIR"
        exit 1
    fi

    cd "$EXTRACTOR_DIR"

    poetry install

    poetry run pyinstaller --onefile \
        --name "$EXTRACTOR_NAME" \
        --paths src/ \
        --collect-all fastapi \
        --collect-all pydantic \
        --collect-all pydantic_core \
        --collect-all starlette \
        build_entrypoint.py

    BINARY_PATH="$EXTRACTOR_DIR/dist/$EXTRACTOR_NAME"

else
    echo "❌ Unknown extractor type: $EXTRACTOR_NAME"
    exit 1
fi

# ============================================================
# ✅ VALIDATE BUILD
# ============================================================
echo "🔍 Checking binary at: $BINARY_PATH"

if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ Build failed. Binary not found at $BINARY_PATH"
    exit 1
fi

# Move binary to repo root for upload
cd "$REPO_ROOT"
mv "$BINARY_PATH" "$ASSET_NAME"

TARGET_REPO="AxiomCore/AxiomCore"

echo "🚀 Uploading $ASSET_NAME to GitHub Release $VERSION on $TARGET_REPO..."

# Create release if it doesn't exist
gh release create "$VERSION" \
    --repo "$TARGET_REPO" \
    --title "$VERSION" \
    --notes "Unified Axiom Extractor Release" 2>/dev/null || true

# Upload asset
gh release upload "$VERSION" "$ASSET_NAME" \
    --repo "$TARGET_REPO" \
    --clobber

echo "✅ Published: $ASSET_NAME to $TARGET_REPO release $VERSION"
