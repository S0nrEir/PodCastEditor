#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=11.0
targets=(aarch64-apple-darwin x86_64-apple-darwin)
case "${1:-}" in
    "") rustup target add "${targets[@]}"
        for arch in "${targets[@]}"; do
            cargo build --locked --release --target "$arch" --bin podcast-studio
        done ;;
    --prebuilt) ;;  # Package existing release builds from CARGO_TARGET_DIR.
    *) echo "usage: bash scripts/package-macos.sh [--prebuilt]" >&2; exit 2 ;;
esac

mkdir -p dist
stage=$(mktemp -d "$PWD/dist/macos.XXXXXX")
trap 'rm -rf "$stage"' EXIT
app="$stage/声剪.app"
exe="$app/Contents/MacOS/podcast-studio"
mkdir -p "$app/Contents/MacOS"
cp packaging/macos/Info.plist "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
lipo -create "${CARGO_TARGET_DIR:-target}/${targets[0]}/release/podcast-studio" \
             "${CARGO_TARGET_DIR:-target}/${targets[1]}/release/podcast-studio" -output "$exe"
chmod 755 "$exe"
lipo "$exe" -verify_arch arm64 x86_64
codesign --force --sign - "$app"
codesign --verify --strict "$app"
ditto -c -k --keepParent "$app" "$stage/声剪-macOS.zip"
mv "$stage/声剪-macOS.zip" dist/
