#!/usr/bin/env bash
# Builds a universal (Apple Silicon + Intel) metascrub.app inside
# dist/metascrub-macos-universal.dmg, plus a universal command-line binary.
# Must run on macOS (uses lipo, codesign and hdiutil).
#
#   packaging/macos/build-dmg.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="${VERSION:-$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)}"
OUT="$ROOT/dist"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$ROOT"

# 1. Build for both Mac CPU types.
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
rustup target add "${TARGETS[@]}"
for t in "${TARGETS[@]}"; do
    cargo build --release -p metascrub -p metascrub-cli --target "$t"
done

# 2. Assemble the .app bundle with one universal binary.
APP="$WORK/metascrub.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/metascrub" \
    "target/aarch64-apple-darwin/release/metascrub" "target/x86_64-apple-darwin/release/metascrub"
cp assets/icon.icns "$APP/Contents/Resources/metascrub.icns"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>metascrub</string>
    <key>CFBundleDisplayName</key><string>metascrub</string>
    <key>CFBundleIdentifier</key><string>io.github.metascrub</string>
    <key>CFBundleExecutable</key><string>metascrub</string>
    <key>CFBundleIconFile</key><string>metascrub</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
plutil -lint "$APP/Contents/Info.plist"

# Ad-hoc signature: required for the app to launch on Apple Silicon at all.
# (Not a paid Apple Developer signature, so users still see a first-run warning.)
codesign --force --deep --sign - "$APP"

# 3. Disk image with a shortcut to /Applications for drag-to-install.
mkdir -p "$WORK/dmg" "$OUT"
cp -R "$APP" "$WORK/dmg/"
ln -s /Applications "$WORK/dmg/Applications"
hdiutil create -volname metascrub -srcfolder "$WORK/dmg" -ov -format UDZO "$OUT/metascrub-macos-universal.dmg"

# 4. Universal command-line tool.
lipo -create -output "$WORK/metascrub-cli" \
    "target/aarch64-apple-darwin/release/metascrub-cli" "target/x86_64-apple-darwin/release/metascrub-cli"
codesign --force --sign - "$WORK/metascrub-cli"
tar -czf "$OUT/metascrub-cli-macos-universal.tar.gz" -C "$WORK" metascrub-cli

echo "built $OUT/metascrub-macos-universal.dmg and $OUT/metascrub-cli-macos-universal.tar.gz"
