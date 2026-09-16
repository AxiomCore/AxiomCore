#!/bin/bash
set -e

export MACOSX_DEPLOYMENT_TARGET=11.0
export IPHONEOS_DEPLOYMENT_TARGET=13.0

# --- Absolute Path Resolution ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RUNTIME_DIR="$REPO_ROOT/../axiom-runtime"
LIB_NAME="libaxiom_runtime.a"
FRAMEWORK_NAME="AxiomRuntime"

INCLUDE_DIR="$RUNTIME_DIR/include"
DIST_DIR="$RUNTIME_DIR/dist"
TARGET_DIR="$RUNTIME_DIR/target"

echo "🚀 Starting Universal Apple Build Process..."
cd "$RUNTIME_DIR"

# 1. Generate the C ABI header from Rust. This is deliberately not a hand
# maintained copy: a release must package the exact ABI it links.
echo "📝 Generating C ABI header..."
mkdir -p "$INCLUDE_DIR"
command -v cbindgen >/dev/null || { echo "cbindgen is required to package AxiomRuntime" >&2; exit 1; }
"$RUNTIME_DIR/scripts/generate-ffi-header.sh"

cat <<EOF > "$INCLUDE_DIR/module.modulemap"
module AxiomRuntime {
    header "axiom.h"
    export *
}
EOF

# 2. Build Rust Targets
echo "🛠 Building Rust targets (iOS + macOS)..."
rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim \
                  aarch64-apple-darwin x86_64-apple-darwin

cargo build --release --target aarch64-apple-ios
cargo build --release --target x86_64-apple-ios
cargo build --release --target aarch64-apple-ios-sim

cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# 3. Create Universal Binaries (Lipo)
echo "🔗 Creating universal binaries..."

mkdir -p "$TARGET_DIR/ios-sim-universal"
lipo -create \
    "$TARGET_DIR/x86_64-apple-ios/release/$LIB_NAME" \
    "$TARGET_DIR/aarch64-apple-ios-sim/release/$LIB_NAME" \
    -output "$TARGET_DIR/ios-sim-universal/$LIB_NAME"

mkdir -p "$TARGET_DIR/macos-universal"
lipo -create \
    "$TARGET_DIR/x86_64-apple-darwin/release/$LIB_NAME" \
    "$TARGET_DIR/aarch64-apple-darwin/release/$LIB_NAME" \
    -output "$TARGET_DIR/macos-universal/$LIB_NAME"

# 4. Create XCFramework
echo "📦 Packaging Universal XCFramework..."
rm -rf "$DIST_DIR/$FRAMEWORK_NAME.xcframework"
mkdir -p "$DIST_DIR"

xcodebuild -create-xcframework \
    -library "$TARGET_DIR/aarch64-apple-ios/release/$LIB_NAME" \
    -headers "$INCLUDE_DIR" \
    -library "$TARGET_DIR/ios-sim-universal/$LIB_NAME" \
    -headers "$INCLUDE_DIR" \
    -library "$TARGET_DIR/macos-universal/$LIB_NAME" \
    -headers "$INCLUDE_DIR" \
    -output "$DIST_DIR/$FRAMEWORK_NAME.xcframework"

# 5. COMPRESS FOR DISTRIBUTION
echo "🗜 Zipping XCFramework for remote distribution..."
cd "$DIST_DIR"
# -y preserves symlinks which Apple Frameworks require
zip -ryq "$FRAMEWORK_NAME.xcframework.zip" "$FRAMEWORK_NAME.xcframework"
cd - > /dev/null

echo "-------------------------------------------"
echo "✅ Universal Framework Zipped at $DIST_DIR/$FRAMEWORK_NAME.xcframework.zip"
