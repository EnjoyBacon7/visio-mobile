#!/usr/bin/env bash
# Cross-platform E2E test: Desktop ↔ Android with real video/audio.
#
# Orchestrates:
#   1. LiveKit Docker server
#   2. visio-bot publishing real video + screen share (acts as desktop)
#   3. Android app via Maestro (joins same room, verifies tracks)
#
# Prerequisites:
#   - Docker running
#   - Android device/emulator connected (adb devices)
#   - APK installed (./scripts/build-android.sh && adb install ...)
#   - Maestro CLI (curl -Ls 'https://get.maestro.mobile.dev' | bash)
#   - ffmpeg (brew install ffmpeg)
#   - Test video downloaded (./scripts/download-test-media.sh)
#
# Usage:
#   ./scripts/run-cross-platform-e2e.sh [--room ROOM] [--duration SECS]
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

ROOM="${ROOM:-cross-e2e-$(date +%s)}"
DURATION="${DURATION:-30}"
LIVEKIT_URL="ws://localhost:7880"
MEDIA_FILE="$ROOT_DIR/test-assets/test-video.mp4"
BOT_LOG="$ROOT_DIR/test-assets/bot-output.log"

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --room) ROOM="$2"; shift 2 ;;
        --duration) DURATION="$2"; shift 2 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { echo -e "${BLUE}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

cleanup() {
    info "Cleaning up..."
    kill $BOT_PID 2>/dev/null || true
    docker stop livekit-cross-e2e 2>/dev/null || true
    rm -f "$BOT_LOG"
}
trap cleanup EXIT

# =========================================================================
# Step 0: Prerequisites
# =========================================================================
info "Checking prerequisites..."

command -v docker >/dev/null 2>&1 || { fail "Docker required"; exit 1; }
command -v ffmpeg >/dev/null 2>&1 || { fail "ffmpeg required: brew install ffmpeg"; exit 1; }
command -v cargo  >/dev/null 2>&1 || { fail "Rust/Cargo required"; exit 1; }

if [ ! -f "$MEDIA_FILE" ]; then
    warn "Test video not found. Downloading..."
    "$SCRIPT_DIR/download-test-media.sh"
fi

# Build bot
info "Building visio-bot..."
cd "$ROOT_DIR"
cargo build -p visio-bot --quiet 2>&1 || { fail "Build failed"; exit 1; }
ok "visio-bot built"

# =========================================================================
# Step 1: Start LiveKit
# =========================================================================
info "Starting LiveKit server..."
docker stop livekit-cross-e2e 2>/dev/null || true
docker run -d --rm --name livekit-cross-e2e \
    -p 7880:7880 -p 7881:7881 -p 7882:7882/udp \
    livekit/livekit-server --dev --bind 0.0.0.0 \
    >/dev/null 2>&1
sleep 3
ok "LiveKit running on port 7880"

# =========================================================================
# Step 2: Start bot (desktop participant with real video + screen share)
# =========================================================================
info "Starting bot in room '$ROOM' with real video + screen share..."
BOT_PID=""
cargo run -p visio-bot --quiet -- \
    --url "$LIVEKIT_URL" \
    --room "$ROOM" \
    --identity "desktop-bot" \
    --name "Desktop (Bot)" \
    --media-file "$MEDIA_FILE" \
    --loop-media \
    --screen-share \
    --monitor-audio \
    --expect-participants 1 \
    --duration "$DURATION" \
    --chat-message "Hello from Desktop!" \
    --raise-hand \
    2>&1 | tee "$BOT_LOG" &
BOT_PID=$!
sleep 5
ok "Bot running (PID $BOT_PID)"

# =========================================================================
# Step 3: Instructions for connecting
# =========================================================================
echo ""
echo "============================================================"
echo -e "${GREEN}Cross-Platform E2E Test Ready${NC}"
echo "============================================================"
echo ""
echo "Room:     $ROOM"
echo "LiveKit:  $LIVEKIT_URL"
echo "Duration: ${DURATION}s"
echo ""
echo "The bot is publishing:"
echo "  - Audio (from test video)"
echo "  - Video (from test video)"
echo "  - Screen share (from test video)"
echo "  - Chat message: 'Hello from Desktop!'"
echo "  - Hand raised"
echo ""
echo "Connect from your platforms:"
echo ""
echo -e "${YELLOW}Android (Maestro):${NC}"
echo "  maestro test .maestro/09_full_call_flow.yaml"
echo ""
echo -e "${YELLOW}Android (Manual):${NC}"
echo "  Open Visio, enter room: $ROOM"
echo "  Verify: video tile, screen share, chat message, hand icon"
echo ""
echo -e "${YELLOW}Desktop (cargo tauri dev):${NC}"
echo "  cd crates/visio-desktop && cargo tauri dev"
echo "  Enter room: $ROOM"
echo ""
echo "============================================================"
echo ""

# =========================================================================
# Step 4: Wait for bot to finish
# =========================================================================
info "Waiting for bot to complete (${DURATION}s)..."
wait $BOT_PID 2>/dev/null || true
BOT_EXIT=$?

echo ""
echo "============================================================"
echo -e "${BLUE}Bot Results${NC}"
echo "============================================================"

# Extract summary from log
if [ -f "$BOT_LOG" ]; then
    echo ""
    grep -E "\[SUMMARY\]|\[AUDIO QUALITY\]|\[EVENT\] (ParticipantJoined|TrackSubscribed|TrackUnsubscribed)" "$BOT_LOG" 2>/dev/null || true
    echo ""

    # Check results
    SUBS="$(grep -c "TrackSubscribed" "$BOT_LOG" 2>/dev/null)" || SUBS=0
    JOINS="$(grep -c "ParticipantJoined" "$BOT_LOG" 2>/dev/null)" || JOINS=0

    if [ "$JOINS" -gt 0 ]; then
        ok "Remote participant(s) joined: $JOINS"
    else
        warn "No remote participants joined during the test"
    fi

    if [ "$SUBS" -gt 0 ]; then
        ok "Tracks received from remote: $SUBS"
    else
        warn "No tracks received from remote participants"
    fi

    if grep -q "Audio quality OK" "$BOT_LOG" 2>/dev/null; then
        ok "Audio quality: OK"
    elif grep -q "NO audio frames received" "$BOT_LOG" 2>/dev/null; then
        fail "Audio quality: NO frames received"
    fi
fi

echo ""
if [ "$BOT_EXIT" -eq 0 ]; then
    ok "Cross-platform E2E test completed"
else
    fail "Bot exited with code $BOT_EXIT"
fi
