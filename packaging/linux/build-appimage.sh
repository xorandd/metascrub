#!/usr/bin/env bash
# Builds dist/metascrub-<arch>.AppImage from an already-compiled metascrub (desktop app).
#
#   cargo build --release -p metascrub
#   packaging/linux/build-appimage.sh
#
# Optional: pass the binary path as the first argument. Downloads appimagetool
# on first use if it isn't installed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${1:-$ROOT/target/release/metascrub}"
ARCH="${ARCH:-$(uname -m)}"
VERSION="${VERSION:-$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)}"
OUT="$ROOT/dist"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

[ -x "$BIN" ] || { echo "error: $BIN not found; run: cargo build --release -p metascrub" >&2; exit 1; }

# 1. Lay out the AppDir.
APPDIR="$WORK/metascrub.AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
install -m755 "$BIN" "$APPDIR/usr/bin/metascrub"
install -m644 "$ROOT/packaging/linux/metascrub.desktop" "$APPDIR/usr/share/applications/metascrub.desktop"
install -m644 "$ROOT/assets/icon.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/metascrub.png"
cp "$APPDIR/usr/share/applications/metascrub.desktop" "$APPDIR/metascrub.desktop"
cp "$ROOT/assets/icon.png" "$APPDIR/metascrub.png"
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/metascrub" "$@"
EOF
chmod 755 "$APPDIR/AppRun"

# 2. Get appimagetool if needed.
TOOL="$(command -v appimagetool || true)"
if [ -z "$TOOL" ]; then
    TOOL="$WORK/appimagetool"
    curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
    chmod 755 "$TOOL"
fi

# 3. Pack it. APPIMAGE_EXTRACT_AND_RUN avoids needing FUSE (e.g. on CI runners).
mkdir -p "$OUT"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" VERSION="$VERSION" "$TOOL" --no-appstream "$APPDIR" "$OUT/metascrub-$ARCH.AppImage"
echo "built $OUT/metascrub-$ARCH.AppImage"
