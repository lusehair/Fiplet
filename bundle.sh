#!/usr/bin/env bash
# Builds "FIP.app" from the release binary.
#
# Set UNIVERSAL=1 to also build for Intel (requires the x86_64 target:
# `rustup target add x86_64-apple-darwin`) and merge both with lipo.
set -euo pipefail
cd "$(dirname "$0")"

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

if ! command -v cargo >/dev/null 2>&1; then
  echo "✗ cargo not found. Install Rust (https://rustup.rs) and run this script again." >&2
  exit 1
fi

APP="FIP.app"
BIN="fip-menubar-light"

if [ "${UNIVERSAL:-0}" = "1" ]; then
  echo "▸ cargo build --release (arm64 + x86_64)"
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output "target/universal/$BIN" \
    "target/aarch64-apple-darwin/release/$BIN" \
    "target/x86_64-apple-darwin/release/$BIN"
  BUILT="target/universal/$BIN"
else
  echo "▸ cargo build --release"
  cargo build --release
  BUILT="target/release/$BIN"
fi

echo "▸ assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp Info.plist "$APP/Contents/Info.plist"
cp FIP.icns "$APP/Contents/Resources/FIP.icns"
cp "$BUILT" "$APP/Contents/MacOS/$BIN"
chmod +x "$APP/Contents/MacOS/$BIN"

# Ad-hoc re-signing: copying the binary and injecting the Info.plist invalidates the
# ad-hoc signature applied by the linker. We re-sign the complete bundle (free,
# no developer account required). This does NOT notarize: on another Mac, the
# recipient will have to remove the quarantine (see message below).
echo "▸ ad-hoc signing the bundle"
codesign --force --sign - "$APP"
codesign --verify --strict "$APP" && echo "✓ ad-hoc signature valid"

echo "✓ $APP ready."
echo "  Run:      open '$APP'"
echo "  Install:  cp -r '$APP' /Applications/"
echo
echo "  On ANOTHER Mac (app not notarized), clear the quarantine flag once:"
echo "    xattr -dr com.apple.quarantine '/Applications/$APP'"
