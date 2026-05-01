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
