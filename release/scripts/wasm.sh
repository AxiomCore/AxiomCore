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
