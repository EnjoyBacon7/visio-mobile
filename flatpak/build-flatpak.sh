#!/usr/bin/env bash
# Build a Flatpak bundle from source inside the GNOME SDK.
#
# Prerequisites (CI installs these):
#   - flatpak + flatpak-builder
#   - org.gnome.Platform//47 + org.gnome.Sdk//47
#   - org.freedesktop.Sdk.Extension.llvm18//24.08
#   - Frontend already built (crates/visio-desktop/frontend/dist/)
#   - cargo vendor cargo-vendor already run (cargo-vendor/ directory exists)
#   - lk-webrtc/ pre-downloaded
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

# 1. Vendor cargo deps if not already done
if [ ! -d "cargo-vendor" ]; then
    echo "==> Vendoring cargo dependencies"
    cargo vendor cargo-vendor > /dev/null
fi

# 2. Build frontend if not already done
if [ ! -d "crates/visio-desktop/frontend/dist" ]; then
    echo "==> Building frontend"
    cd crates/visio-desktop/frontend
    npm ci
    npm run build
    cd "$PROJECT_ROOT"
fi

# 3. Force-add untracked build artifacts so flatpak-builder copies them
#    (flatpak-builder uses `git ls-files` in git repos, skipping untracked files)
echo "==> Staging untracked build artifacts for flatpak-builder"
git add -f crates/visio-desktop/frontend/dist/ cargo-vendor/ lk-webrtc/ 2>/dev/null || true

# 4. Build Flatpak
echo "==> Building Flatpak (from source in GNOME SDK sandbox)"
flatpak-builder --force-clean \
    --repo="$SCRIPT_DIR/repo" \
    "$SCRIPT_DIR/build" \
    "$SCRIPT_DIR/io.visio.desktop.yml"

# 5. Unstage the force-added files
git reset HEAD -- crates/visio-desktop/frontend/dist/ cargo-vendor/ lk-webrtc/ 2>/dev/null || true

# 6. Create distributable bundle
mkdir -p "$PROJECT_ROOT/target/release/bundle/flatpak"
BUNDLE="$PROJECT_ROOT/target/release/bundle/flatpak/visio-mobile.flatpak"
flatpak build-bundle \
    "$SCRIPT_DIR/repo" \
    "$BUNDLE" \
    io.visio.desktop

echo "==> Done: $BUNDLE"
echo ""
echo "Install with:  flatpak install visio-mobile.flatpak"
echo "Run with:      flatpak run io.visio.desktop"
