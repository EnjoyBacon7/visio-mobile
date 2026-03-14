#!/usr/bin/env bash
# Run Maestro E2E tests against the installed Android app.
# Requires: Maestro CLI, Android device/emulator with app installed.
# Optional: LiveKit dev server + visio-bot for call tests.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"

# Check prerequisites
command -v maestro >/dev/null 2>&1 || { echo "Maestro CLI required: curl -Ls 'https://get.maestro.mobile.dev' | bash"; exit 1; }

echo "Running Maestro E2E tests..."
maestro test "$ROOT_DIR/e2e/maestro/"
