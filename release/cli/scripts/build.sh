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
