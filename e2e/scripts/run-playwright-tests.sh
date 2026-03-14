#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")"
FRONTEND_DIR="$ROOT_DIR/crates/visio-desktop/frontend"

cd "$FRONTEND_DIR"

# Install deps if needed
if [ ! -d "node_modules/@playwright" ]; then
    echo "Installing Playwright..."
    npm install -D @playwright/test
    npx playwright install chromium
fi

echo "Running Playwright E2E tests..."
npx playwright test "$@"
