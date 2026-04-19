
#!/bin/bash
set -e

VERSION=$1
if [ -z "$VERSION" ]; then
    echo "❌ Error: No version provided. Usage: ./publish.sh v0.1.0"
    exit 1
fi
PLAIN_VERSION="${VERSION#v}"

CLI_REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_DIR="$CLI_REPO_DIR/release"
TAP_REPO_DIR="$(cd "$CLI_REPO_DIR/../homebrew-tap" && pwd)"

OS="macos"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
esac
PLATFORM="${OS}-${ARCH}"

AXIOM_TAR="$RELEASE_DIR/axiom-${PLATFORM}.tar.gz"
if [[ ! -f "$AXIOM_TAR" ]]; then
    echo "❌ Error: Tarball not found at $AXIOM_TAR"
    exit 1
fi

AXIOM_SHA=$(shasum -a 256 "$AXIOM_TAR" | awk '{print $1}')

echo "🚀 Creating GitHub Release $VERSION..."
# Assumes you are running this from the main AxiomCore repo
git add .
git commit -m "Release $VERSION" || echo "Nothing to commit"
git push origin main

gh release create "$VERSION" "$AXIOM_TAR" --title "$VERSION" --notes "Automated Release" || echo "Release might already exist, uploading asset..."
gh release upload "$VERSION" "$AXIOM_TAR" --clobber

echo "🍺 Updating Homebrew Tap..."
FORMULA_FILE="$TAP_REPO_DIR/Formula/axiom.rb"

sed -i '' "s|releases/download/.*/axiom-macos-arm64.tar.gz|releases/download/${VERSION}/axiom-macos-arm64.tar.gz|g" "$FORMULA_FILE"
sed -i '' "s|sha256 \".*\"|sha256 \"${AXIOM_SHA}\"|g" "$FORMULA_FILE"
sed -i '' "s|version \".*\"|version \"${PLAIN_VERSION}\"|g" "$FORMULA_FILE"

cd "$TAP_REPO_DIR"
git add Formula/axiom.rb
git commit -m "Update Axiom to $VERSION" || true
git push origin main

echo "✅ Successfully published Axiom CLI $VERSION!"
