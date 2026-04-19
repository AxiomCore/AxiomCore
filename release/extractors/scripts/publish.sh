#!/bin/bash
set -e

EXTRACTOR_NAME=$1
VERSION=$2

if [ -z "$EXTRACTOR_NAME" ] || [ -z "$VERSION" ]; then
    echo "❌ Usage: ./publish.sh <extractor-name> <version-tag>"
    echo "Example: ./publish.sh axiom-fastapi v1.0.0"
    exit 1
fi

# Determine source directory (e.g., axiom-fastapi -> AxiomCore/axiom-extractor/extractors/python/frameworks/axiom_fastapi)
# Note: uses snake_case for directory names
DIR_NAME="${EXTRACTOR_NAME//-/_}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXTRACTOR_DIR="${SCRIPT_DIR}/axiom-extractor/extractors/python/frameworks/${EXTRACTOR_NAME//-/_}"

if [ ! -d "$EXTRACTOR_DIR" ]; then
    echo "❌ Extractor directory not found: $EXTRACTOR_DIR"
    exit 1
fi

# Detect platform for unique asset naming
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
esac
PLATFORM="${OS}-${ARCH}"

echo "🐍 Building Python Extractor '$EXTRACTOR_NAME' for $PLATFORM..."
cd "$EXTRACTOR_DIR"

# 1. Install dependencies via Poetry
poetry install

# 2. Build using your specific PyInstaller flags
# We point to build_entrypoint.py as your main entry
poetry run pyinstaller --onefile \
    --name "$EXTRACTOR_NAME" \
    --paths src/ \
    --collect-all fastapi \
    --collect-all pydantic \
    --collect-all pydantic_core \
    --collect-all starlette \
    build_entrypoint.py

BINARY_PATH="dist/$EXTRACTOR_NAME"
if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ PyInstaller build failed. Binary not found at $BINARY_PATH"
    exit 1
fi

# 3. Create a unique asset name so they don't overwrite each other in the release
# Result: axiom-fastapi-macos-arm64
ASSET_NAME="${EXTRACTOR_NAME}-${PLATFORM}"
mv "$BINARY_PATH" "$ASSET_NAME"

echo "🚀 Uploading $ASSET_NAME to GitHub Release $VERSION..."

# Ensure the release exists (it won't error if it already exists because of || true)
gh release create "$VERSION" --title "$VERSION" --notes "Unified Axiom Extractor Release" || true

# Upload the unique asset. --clobber ensures if you re-run the build for the SAME
# platform, it replaces the old binary, but leaves binaries for OTHER platforms alone.
gh release upload "$VERSION" "$ASSET_NAME" --clobber

echo "✅ Published: $ASSET_NAME to release $VERSION"
