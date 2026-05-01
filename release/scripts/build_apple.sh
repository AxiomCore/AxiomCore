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
