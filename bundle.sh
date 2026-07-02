#!/usr/bin/env bash
# Builds "FIP.app" from the release binary.
set -euo pipefail
cd "$(dirname "$0")"

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

if ! command -v cargo >/dev/null 2>&1; then
  echo "▸ cargo introuvable, installation de Rust via rustup…"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  . "$HOME/.cargo/env"
fi

APP="FIP.app"
BIN="fip-menubar-light"

echo "▸ cargo build --release"
cargo build --release

echo "▸ assemblage de $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp Info.plist "$APP/Contents/Info.plist"
cp FIP.icns "$APP/Contents/Resources/FIP.icns"
cp "target/release/$BIN" "$APP/Contents/MacOS/$BIN"
chmod +x "$APP/Contents/MacOS/$BIN"

# Ad-hoc re-signing: copying the binary and injecting the Info.plist invalidates the
# ad-hoc signature applied by the linker. We re-sign the complete bundle (free,
# no developer account required). This does NOT notarize: on another Mac, the
# recipient will have to remove the quarantine (see message below).
echo "▸ signature ad-hoc du bundle"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP" && echo "✓ signature ad-hoc valide"

echo "✓ $APP prêt."
echo "  Lancer :  open '$APP'"
echo "  Installer : cp -r '$APP' /Applications/"
echo
echo "  Sur un AUTRE Mac (app non notarisée), lever la quarantaine une fois :"
echo "    xattr -dr com.apple.quarantine '/Applications/$APP'"
