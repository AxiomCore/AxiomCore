#!/bin/bash
set -e

VERSION=$1
if [ -z "$VERSION" ]; then
    echo "❌ Error: No version provided. Usage: ./publish_apple.sh v0.1.0"
    exit 1
fi
# Strip the "v" prefix for places that just need the raw number
CLEAN_VERSION="${VERSION#v}"
TAGGED_VERSION="v${CLEAN_VERSION}"

RELEASE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZIP_PATH="$RELEASE_ROOT/dist/AxiomRuntime.xcframework.zip"

if [ ! -f "$ZIP_PATH" ]; then
    echo "❌ Error: Zip file not found at $ZIP_PATH. Run 'just build-apple' first."
    exit 1
fi

echo "🧮 Calculating SHA256 checksum..."
ZIP_SHA=$(shasum -a 256 "$ZIP_PATH" | awk '{print $1}')
echo "   SHA256: $ZIP_SHA"

# ==========================================
# 1. UPDATE FLUTTER PODSPECS
# ==========================================
echo "📝 Updating Flutter Podspecs to version $CLEAN_VERSION..."
PODSPEC_IOS="$RELEASE_ROOT/../../axiom-sdk/flutter/axiom_flutter/ios/axiom_flutter.podspec"
PODSPEC_MACOS="$RELEASE_ROOT/../../axiom-sdk/flutter/axiom_flutter/macos/axiom_flutter.podspec"

# macOS sed syntax to replace `s.version = '...'`
sed -i '' "s/s\.version[[:space:]]*=[[:space:]]*'.*'/s.version          = '${CLEAN_VERSION}'/g" "$PODSPEC_IOS"
sed -i '' "s/s\.version[[:space:]]*=[[:space:]]*'.*'/s.version          = '${CLEAN_VERSION}'/g" "$PODSPEC_MACOS"

# ==========================================
# 2. UPDATE SWIFT PACKAGE
# ==========================================
echo "📝 Updating Swift Package.swift to version $CLEAN_VERSION..."
PACKAGE_SWIFT="$RELEASE_ROOT/../../axiom-sdk/swift/Package.swift"

# Replace the URL to point to the new version tag
sed -i '' "s|url: \"https://github.com/AxiomCore/AxiomCore/releases/download/.*/AxiomRuntime.xcframework.zip\"|url: \"https://github.com/AxiomCore/AxiomCore/releases/download/v${CLEAN_VERSION}/AxiomRuntime.xcframework.zip\"|g" "$PACKAGE_SWIFT"

# Replace the checksum with the actual calculated SHA256
sed -i '' "s/checksum: \".*\"/checksum: \"${ZIP_SHA}\"/g" "$PACKAGE_SWIFT"

echo "🚀 Creating GitHub Release $TAGGED_VERSION if it doesn't exist..."
gh release create "$TAGGED_VERSION" \
  --repo AxiomCore/AxiomCore \
  --title "$TAGGED_VERSION" \
  --notes "Release $TAGGED_VERSION" \
  2>/dev/null || echo "ℹ️  Release $TAGGED_VERSION already exists, skipping create..."

echo "🚀 Uploading $ZIP_PATH to GitHub Release $TAGGED_VERSION..."
gh release upload "$TAGGED_VERSION" "$ZIP_PATH" --repo AxiomCore/AxiomCore --clobber

echo "✅ Apple framework published and SDK definitions updated!"


# ==========================================
# 4. PUBLISH SDK
# ==========================================
echo "🚀 Triggering Flutter SDK Publish..."
"$RELEASE_ROOT/scripts/publish_sdk.sh" flutter "$VERSION"

echo "✅ Apple framework published and SDK definitions updated!"
