#!/bin/bash
set -e

EXTRACTOR_NAME=$1
VERSION=$2

if [ -z "$EXTRACTOR_NAME" ] || [ -z "$VERSION" ]; then
    echo "❌ Usage: ./publish.sh <extractor-name> <version-tag>"
    echo "Example: ./publish.sh axiom-fastapi v1.0.0"
    echo "Example: ./publish.sh axiom-go-extractor v1.0.0"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MONOREPO_ROOT="${SCRIPT_DIR}/../../.."

# Detect platform for unique asset naming
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
esac
PLATFORM="${OS}-${ARCH}"
ASSET_NAME="${EXTRACTOR_NAME}-${PLATFORM}"
BINARY_PATH=""

if [ "$EXTRACTOR_NAME" == "axiom-go-extractor" ]; then
    echo "🐹 Building Go Extractor '$EXTRACTOR_NAME' for $PLATFORM..."
    cd "${MONOREPO_ROOT}/axiom-extractor/extractors/go"

    # Ensure dependencies
    go mod tidy

    # Build
    go build -o "dist/$EXTRACTOR_NAME" ./cmd/axiom-go-extractor
    BINARY_PATH="dist/$EXTRACTOR_NAME"

elif [[ "$EXTRACTOR_NAME" == axiom-fastapi* ]]; then
    DIR_NAME="${EXTRACTOR_NAME//-/_}"
    EXTRACTOR_DIR="${MONOREPO_ROOT}/axiom-extractor/extractors/python/frameworks/${DIR_NAME}"

    if [ ! -d "$EXTRACTOR_DIR" ]; then
        echo "❌ Extractor directory not found: $EXTRACTOR_DIR"
        exit 1
    fi

    echo "🐍 Building Python Extractor '$EXTRACTOR_NAME' for $PLATFORM..."
    cd "$EXTRACTOR_DIR"

    # Install dependencies via Poetry
    poetry install

    # Build using PyInstaller
    poetry run pyinstaller --onefile \
        --name "$EXTRACTOR_NAME" \
        --paths src/ \
        --collect-all fastapi \
        --collect-all pydantic \
        --collect-all pydantic_core \
        --collect-all starlette \
        build_entrypoint.py

    BINARY_PATH="dist/$EXTRACTOR_NAME"
else
    echo "❌ Unknown extractor type: $EXTRACTOR_NAME"
    exit 1
fi

if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ Build failed. Binary not found at $BINARY_PATH"
    exit 1
fi

mv "$BINARY_PATH" "$ASSET_NAME"
echo "🚀 Uploading $ASSET_NAME to GitHub Release $VERSION..."

# Ensure the release exists
gh release create "$VERSION" --title "$VERSION" --notes "Unified Axiom Extractor Release" || true

# Upload the unique asset
gh release upload "$VERSION" "$ASSET_NAME" --clobber

echo "✅ Published: $ASSET_NAME to release $VERSION"
