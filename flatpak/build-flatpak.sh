#!/usr/bin/env bash
# Build a Flatpak bundle from the pre-built Tauri binary.
# Run this AFTER `cargo tauri build` on Linux.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
STAGING="$SCRIPT_DIR/staging"
BUNDLE_DIR="$PROJECT_ROOT/target/release/bundle"
BINARY="$PROJECT_ROOT/target/release/visio-desktop"

echo "==> Preparing staging directory"
rm -rf "$STAGING"
mkdir -p "$STAGING"

# Binary
cp "$BINARY" "$STAGING/visio-desktop"

# Desktop + metadata
cp "$SCRIPT_DIR/io.visio.desktop.desktop" "$STAGING/"
cp "$SCRIPT_DIR/io.visio.desktop.metainfo.xml" "$STAGING/"

# Icons
cp "$PROJECT_ROOT/crates/visio-desktop/icons/32x32.png" "$STAGING/icon-32.png"
cp "$PROJECT_ROOT/crates/visio-desktop/icons/128x128.png" "$STAGING/icon-128.png"
cp "$PROJECT_ROOT/crates/visio-desktop/icons/icon.png" "$STAGING/icon-256.png"

# i18n
mkdir -p "$STAGING/i18n"
cp "$PROJECT_ROOT/i18n/"*.json "$STAGING/i18n/"

# Backgrounds
mkdir -p "$STAGING/backgrounds"
cp "$PROJECT_ROOT/assets/backgrounds/"*.jpg "$STAGING/backgrounds/"

# Models
mkdir -p "$STAGING/models"
cp "$PROJECT_ROOT/models/"*.onnx "$STAGING/models/" 2>/dev/null || true

# Frontend dist (bundled by Tauri)
mkdir -p "$STAGING/frontend-dist"
cp -r "$PROJECT_ROOT/crates/visio-desktop/frontend/dist/"* "$STAGING/frontend-dist/"

echo "==> Building Flatpak"
flatpak-builder --force-clean \
  --repo="$SCRIPT_DIR/repo" \
  "$SCRIPT_DIR/build" \
  "$SCRIPT_DIR/io.visio.desktop.yml"

echo "==> Creating Flatpak bundle"
mkdir -p "$PROJECT_ROOT/target/release/bundle/flatpak"
flatpak build-bundle \
  "$SCRIPT_DIR/repo" \
  "$PROJECT_ROOT/target/release/bundle/flatpak/visio-mobile.flatpak" \
  io.visio.desktop

echo "==> Done: target/release/bundle/flatpak/visio-mobile.flatpak"
