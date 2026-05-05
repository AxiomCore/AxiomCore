Project Root: /Users/yashmakan/AxiomCore/AxiomCore/release
Project Structure:
```
.
|-- .release_todo
|-- atmx-cli
    |-- package-lock.json
    |-- package.json
    |-- src
        |-- generators
            |-- model-generator.ts
            |-- sdk-generator.ts
            |-- utils.ts
        |-- index.ts
        |-- templates
        |-- types.ts
    |-- tsconfig.json
|-- cli
    |-- .gitignore
    |-- justfile
    |-- scripts
        |-- build.sh
        |-- publish.sh
        |-- release.sh
|-- extractors
    |-- scripts
        |-- publish.sh
|-- homebrew-tap
    |-- Formula
        |-- axiom.rb
|-- justfile
|-- scripts
    |-- build_apple.sh
    |-- publish_apple.sh
    |-- publish_sdk.sh
    |-- wasm.sh

```

---
## File: cli/justfile

```
default:
    @just --list

build:
    ./scripts/build.sh

release:
    ./scripts/release.sh

publish version:
    ./scripts/publish.sh {{version}}

all version:
    ./scripts/build.sh {{version}}
    ./scripts/release.sh
    ./scripts/publish.sh {{version}}

```
---
## File: cli/scripts/build.sh

```sh
#!/bin/bash
set -e

CLI_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$CLI_ROOT/dist"
AXIOM_RUST_PATH="$CLI_ROOT/../../cli" # Points to AxiomCore/cli

mkdir -p "$DIST_DIR"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🛠️  Starting Axiom CLI Build Process"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

VERSION_ARG=$1
if [ ! -z "$VERSION_ARG" ]; then
    CLEAN_VERSION="${VERSION_ARG#v}"
    CARGO_FILE="$AXIOM_RUST_PATH/Cargo.toml"

    if [ -f "$CARGO_FILE" ]; then
        echo "🔖 Updating Cargo.toml version to $CLEAN_VERSION"
        sed -i '' "s/^version = \".*\"/version = \"$CLEAN_VERSION\"/" "$CARGO_FILE"
    else
        echo "❌ Cargo.toml not found at $CARGO_FILE"
        exit 1
    fi
fi

echo "🦀 Building Axiom (Rust) in $AXIOM_RUST_PATH..."
cd "$AXIOM_RUST_PATH"
cargo build --release

# Copy the binary to our orchestration dist folder
cp "target/release/axiom-cli" "$DIST_DIR/axiom"

echo "✅ Axiom built successfully."
ls -lh "$DIST_DIR/axiom"

```
---
## File: cli/scripts/publish.sh

```sh

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

```
---
## File: cli/scripts/release.sh

```sh
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

```
---
## File: extractors/scripts/publish.sh

```sh
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

```
---
## File: homebrew-tap/Formula/axiom.rb

```rb
class Axiom < Formula
  desc "Axiom CLI - Unified Configuration and API SDK Generator"
  homepage "https://github.com/AxiomCore/AxiomCore"
  url "https://github.com/AxiomCore/AxiomCore/releases/download/v0.62.0/axiom-macos-arm64.tar.gz"
  sha256 "8c0db512ae65d84e138dc6fe8aeffc086a22b8d41bd52944fd57fe458f23201f"
  version "0.62.0"

  def install
    bin.install "axiom"
  end

  test do
    assert_match "Usage", shell_output("#{bin}/axiom --help")
  end
end

```
---
## File: justfile

```
default:
    @just --list

build-runtime:
    @echo "🛠 Building Runtimes (WASM + iOS/macOS)..."
    ./scripts/wasm.sh
    ./scripts/build_apple.sh

build-axiom version:
    ./cli/scripts/build.sh {{version}}

package-axiom:
    ./cli/scripts/release.sh

publish-axiom version:
    ./cli/scripts/publish.sh {{version}}

release-axiom version: (build-axiom version) package-axiom (publish-axiom version)

build-apple version:
    ./scripts/build_apple.sh

publish-apple version:
    ./scripts/publish_apple.sh {{version}}

release-apple version: (build-apple version) (publish-apple version)

publish-sdk sdk version:
    ./scripts/publish_sdk.sh {{sdk}} {{version}}

release-atmx version:
    #!/usr/bin/env bash
    set -e
    CLEAN_VERSION=$(echo "{{version}}" | sed 's/^v//')
    JUSTFILE_DIR="{{justfile_directory()}}"

    echo "📦 Preparing atmx-web core v$CLEAN_VERSION..."
    cd "$JUSTFILE_DIR/../../axiom-sdk/web/atmx"
    npm version $CLEAN_VERSION --no-git-tag-version

    echo "📝 Updating ATMX_VERSION constant in src/index.ts..."
    sed -i '' "s/export const ATMX_VERSION = \".*\";/export const ATMX_VERSION = \"$CLEAN_VERSION\";/" src/index.ts

    echo "🛠 Building atmx-web..."
    npm install
    npm run build

    echo "🚀 Publishing atmx-web to NPM..."
    npm publish --access public || echo "⚠️ NPM Publish failed (maybe already exists?)"

    echo "☁️ Uploading atmx-web to Cloudflare R2..."
    chmod +x scripts/upload.sh
    ./scripts/upload.sh

    # -----------------------------------------------------

    echo "📦 Building and Publishing atmx-react v$CLEAN_VERSION to NPM..."
    cd "$JUSTFILE_DIR/../../axiom-sdk/web/atmx-react"
    npm version $CLEAN_VERSION --no-git-tag-version

    echo "📝 Updating atmx-web dependency version in package.json..."
    sed -i '' "s/\"atmx-web\": \".*\"/\"atmx-web\": \"^$CLEAN_VERSION\"/" package.json

    npm install
    npm run build
    npm publish --access public || echo "⚠️ NPM Publish failed (maybe already exists?)"

    # -----------------------------------------------------

    echo "📦 Building and Publishing atmx-cli v$CLEAN_VERSION to NPM..."
    cd "$JUSTFILE_DIR/atmx-cli"
    npm version $CLEAN_VERSION --no-git-tag-version
    npm install
    npm run build
    npm publish --access public || echo "⚠️ NPM Publish failed (maybe already exists?)"

    echo "✅ ATMX Ecosystem published successfully!"

release-extractor name version:
    ./extractors/scripts/publish.sh {{name}} {{version}}

release-all version:
    @echo "🚀 Starting Full AxiomCore Release for v{{version}}..."

    # 1. Update SDK Versions in source code (pubspecs & Rust utils.rs) before compiling!
    ./scripts/publish_sdk.sh flutter {{version}} --skip-publish

    # 2. Build Runtimes (Wasm + Apple Framework)
    just build-runtime

    # 3. Build & Release Axiom CLI (Now has updated SDK versions compiled in)
    just release-axiom {{version}}

    # 4. Publish Apple Framework + Flutter SDKs to pub.dev
    @echo "📦 Publishing Apple SDK..."
    @if just publish-apple {{version}}; then \
        echo "✅ Apple SDK published successfully"; \
    else \
        echo "⚠️ Apple publish failed (likely rate limit). Skipping for now."; \
        echo "publish-apple {{version}}" >> .release_todo; \
    fi

    # 5. Release Web SDK
    just release-atmx {{version}}

    # 6. Release Extractors
    @echo "📦 Releasing Extractors..."
    just release-extractor axiom-fastapi {{version}}
    just release-extractor axiom-go-extractor {{version}}

    @echo "================================================="
    @echo "🎉 All AxiomCore components released successfully for v{{version}}!"
    @echo "⚠️ NOTE: Don't forget to commit the version bumps in axiom-sdk!"
    @echo "   cd ../../axiom-sdk && git add . && git commit -m 'chore: bump sdk to {{version}}' && git push"

```
---
## File: scripts/build_apple.sh

```sh
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

# 1. Generate Static Headers
echo "📝 Writing static C headers..."
mkdir -p "$INCLUDE_DIR"
cat <<EOF > "$INCLUDE_DIR/axiom.h"
#ifndef AXIOM_RUNTIME_H
#define AXIOM_RUNTIME_H
#include <stdint.h>
#include <stdbool.h>

typedef struct { const uint8_t* ptr; uint64_t len; } AxiomString;
typedef struct { uint8_t* ptr; uint64_t len; } AxiomBuffer;

typedef enum {
    Success = 0,
    UnknownError = 1,
    RequestParsingFailed = 2,
    NetworkError = 3,
    ResponseDeserializationFailed = 4,
    UnknownEndpoint = 5,
    InvalidContract = 10,
    RuntimeTooOld = 11,
    ContractNotLoaded = 12
} FfiError;

typedef struct {
    uint64_t request_id;
    int32_t error_code;
    AxiomBuffer data;
    AxiomBuffer error_message; // Added for updated FFI signature
} AxiomResponseBuffer;

typedef void (*AxiomCallback)(AxiomResponseBuffer* response);
typedef void (*AxiomAuthCallback)(uint64_t request_id);

void axiom_initialize(AxiomString base_url);
int32_t axiom_load_contract(AxiomString namespace, AxiomString base_url, AxiomBuffer contract_buf, AxiomString signature, AxiomString public_key);
void axiom_register_callback(AxiomCallback callback);
void axiom_register_auth_provider(AxiomAuthCallback callback);
void axiom_provide_auth_token(uint64_t request_id, AxiomString token);
void axiom_free_buffer(AxiomBuffer buf);
void axiom_process_responses();
void axiom_call(uint64_t request_id, AxiomString namespace, uint32_t endpoint_id, AxiomString method, AxiomString path, AxiomString traceparent, AxiomString headers_json, AxiomBuffer input_buf);
void axiom_set_auth_token(AxiomString namespace, AxiomString method_name, AxiomString token);
void axiom_clear_auth_token(AxiomString namespace, AxiomString method_name);
void axiom_send_stream_message(uint64_t request_id, AxiomBuffer payload_buf);

#endif
EOF

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

```
---
## File: scripts/publish_apple.sh

```sh
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

```
---
## File: scripts/publish_sdk.sh

```sh
#!/bin/bash
set -e

SDK=$1
VERSION=$2
if [ -z "$VERSION" ]; then
    echo "❌ Usage: ./publish_sdk.sh <sdk> <v0.1.0>"
    exit 1
fi
CLEAN_VERSION="${VERSION#v}"

RELEASE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$RELEASE_ROOT/../.."

if [ "$SDK" == "flutter" ]; then
    echo "📦 Preparing Flutter SDK v$CLEAN_VERSION..."

    # 1. Update Rust CLI Template Constants so the next `cargo build` uses the new version
    UTILS_RS="$REPO_ROOT/axiom-build/src/core/utils.rs"
    if [ -f "$UTILS_RS" ]; then
        echo "📝 Updating Flutter SDK version in axiom-build/src/core/utils.rs..."
        sed -i '' "s/const AXIOM_FLUTTER_VERSION: &str = \".*\";/const AXIOM_FLUTTER_VERSION: \&str = \"^${CLEAN_VERSION}\";/" "$UTILS_RS"
        sed -i '' "s/const AXIOM_FLUTTER_GENERATOR_VERSION: &str = \".*\";/const AXIOM_FLUTTER_GENERATOR_VERSION: \&str = \"^${CLEAN_VERSION}\";/" "$UTILS_RS"
    fi

    # 2. Publish Generator
    echo "📝 Updating axiom_flutter_generator pubspec.yaml..."
    cd "$REPO_ROOT/axiom-sdk/flutter/axiom_flutter_generator"
    sed -i '' "s/^version: .*/version: ${CLEAN_VERSION}/" pubspec.yaml

    # Skip actual publish if --skip-publish flag is passed (used for prepping files before build)
    if [ "$3" != "--skip-publish" ]; then
        echo "🚀 Publishing axiom_flutter_generator to pub.dev..."
        fvm dart pub publish --force
    fi

    # 3. Publish Main SDK
    echo "📝 Updating axiom_flutter pubspec.yaml..."
    cd "$REPO_ROOT/axiom-sdk/flutter/axiom_flutter"
    sed -i '' "s/^version: .*/version: ${CLEAN_VERSION}/" pubspec.yaml

    if [ "$3" != "--skip-publish" ]; then
        echo "🚀 Publishing axiom_flutter to pub.dev..."
        fvm dart pub publish --force
    fi

    echo "✅ Flutter SDK updated/published successfully!"
else
    echo "❌ Unknown SDK: $SDK"
    exit 1
fi

```
---
## File: scripts/wasm.sh

```sh
#!/bin/bash
set -e

# Make sure wasm-pack is installed
if ! command -v wasm-pack &> /dev/null
then
    echo "📦 wasm-pack not found. Installing..."
    cargo install wasm-pack
fi

# --- Absolute Path Resolution ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RUNTIME_DIR="$REPO_ROOT/../axiom-runtime"
DIST_DIR="$RUNTIME_DIR/dist/wasm"

# Flutter Plugin Paths
FLUTTER_PLUGIN_ROOT="$REPO_ROOT/../axiom-sdk/flutter/axiom_flutter"
FLUTTER_WEB_ASSETS="$FLUTTER_PLUGIN_ROOT/lib/assets/wasm"

# ATMX JS Library Paths
ATMX_ROOT="$REPO_ROOT/../axiom-sdk/web/atmx"
ATMX_VENDOR_DIR="$ATMX_ROOT/src/core/vendor"

echo "🚀 Starting WebAssembly Build Process..."
cd "$RUNTIME_DIR"

# 1. Build using wasm-pack targeting no-modules
echo "🛠 Compiling Rust to WebAssembly (no-modules)..."
wasm-pack build --target no-modules --out-dir "$DIST_DIR" --release

# 2. Optimize Wasm (Optional)
if command -v wasm-opt &> /dev/null
then
    echo "⚡ Optimizing Wasm bundle..."
    wasm-opt -Oz \
      --enable-bulk-memory \
      --enable-nontrapping-float-to-int \
      --enable-sign-ext \
      --enable-mutable-globals \
      --strip-debug \
      --strip-producers \
      --dce \
      --vacuum \
      "$DIST_DIR/axiom_runtime_bg.wasm" -o "$DIST_DIR/axiom_runtime_bg.wasm"
else
    echo "⚠️ wasm-opt not found. Skipping optimization. (Install binaryen for smaller output)"
fi

# 3. Auto-Copy to Flutter Plugin assets folder
echo "🚚 Syncing Wasm to Flutter plugin..."
mkdir -p "$FLUTTER_WEB_ASSETS"
rm -rf "$FLUTTER_WEB_ASSETS"/*
cp "$DIST_DIR/axiom_runtime_bg.wasm" "$FLUTTER_WEB_ASSETS/axiom_runtime_bg.wasm"
cp "$DIST_DIR/axiom_runtime.js" "$FLUTTER_WEB_ASSETS/axiom_runtime.js"

# 4. Auto-Copy to ATMX
echo "🚚 Syncing Wasm to ATMX library..."
mkdir -p "$ATMX_VENDOR_DIR"
mkdir -p "$ATMX_ROOT/public"

# Keep JS glue code in vendor so Vite bundles it
rm -rf "$ATMX_VENDOR_DIR"/*
cp "$DIST_DIR/axiom_runtime.js" "$ATMX_VENDOR_DIR/axiom_runtime.js"

# Move WASM to public so Vite outputs it separately
cp "$DIST_DIR/axiom_runtime_bg.wasm" "$ATMX_ROOT/public/axiom_runtime.wasm"

echo "-------------------------------------------"
echo "✅ WebAssembly Sync Complete!"

```
---
