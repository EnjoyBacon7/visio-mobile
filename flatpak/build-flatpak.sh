#!/usr/bin/env bash
# Build a Flatpak bundle from source inside the GNOME SDK.
#
# Prerequisites (CI installs these):
#   - flatpak + flatpak-builder
#   - org.gnome.Platform//46 + org.gnome.Sdk//46
#   - org.freedesktop.Sdk.Extension.rust-stable//24.08
#   - Frontend already built (crates/visio-desktop/frontend/dist/)
#   - cargo vendor already run (vendor/ directory exists)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

# 1. Vendor cargo deps if not already done
if [ ! -d "vendor" ]; then
    echo "==> Vendoring cargo dependencies"
    cargo vendor > /dev/null
fi

# 2. Build frontend if not already done
if [ ! -d "crates/visio-desktop/frontend/dist" ]; then
    echo "==> Building frontend"
    cd crates/visio-desktop/frontend
    npm ci
    npm run build
    cd "$PROJECT_ROOT"
fi

# 3. Build Flatpak
echo "==> Building Flatpak (from source in GNOME SDK sandbox)"
flatpak-builder --force-clean \
    --repo="$SCRIPT_DIR/repo" \
    "$SCRIPT_DIR/build" \
    "$SCRIPT_DIR/io.visio.desktop.yml"

# 4. Create distributable bundle
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
