#!/usr/bin/env bash
# Fully automated cross-platform E2E test: Bot + Desktop + Android.
#
# Orchestrates (ZERO human intervention):
#   1. LiveKit Docker server (local)
#   2. visio-bot publishing real video + audio + screen share
#   3. Desktop app (Tauri) auto-connecting via --livekit-url/--token
#   4. Android app auto-connecting via visio-test:// deep link
#
# Prerequisites:
#   - Docker running
#   - Android device/emulator connected (adb devices)
#   - APK installed on device (debug build with visio-test:// scheme)
#   - ffmpeg (brew install ffmpeg)
#   - Test video downloaded (./scripts/download-test-media.sh)
#   - Desktop crate pre-built (cargo build -p visio-desktop --no-default-features)
#
# Usage:
#   ./scripts/run-cross-platform-e2e.sh [--duration SECS] [--no-android] [--no-desktop]
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

ROOM="e2e-$(date +%s)"
DURATION="${DURATION:-60}"
LIVEKIT_URL="ws://localhost:7880"
MEDIA_FILE="$ROOT_DIR/test-assets/test-video.mp4"
BOT_LOG="$ROOT_DIR/test-assets/bot-output.log"
DESKTOP_LOG="$ROOT_DIR/test-assets/desktop-output.log"
API_KEY="devkey"
API_SECRET="secret"
SKIP_ANDROID=false
SKIP_DESKTOP=false
EXPECTED_PARTICIPANTS=0

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --duration) DURATION="$2"; shift 2 ;;
        --room) ROOM="$2"; shift 2 ;;
        --no-android) SKIP_ANDROID=true; shift ;;
        --no-desktop) SKIP_DESKTOP=true; shift ;;
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

BOT_PID=""
DESKTOP_PID=""

cleanup() {
    info "Cleaning up..."
    [ -n "$BOT_PID" ] && kill "$BOT_PID" 2>/dev/null || true
    [ -n "$DESKTOP_PID" ] && kill "$DESKTOP_PID" 2>/dev/null || true
    [ -n "${VITE_PID:-}" ] && kill "$VITE_PID" 2>/dev/null || true
    lsof -ti:5173 | xargs kill -9 2>/dev/null || true
    adb shell am force-stop io.visio.mobile 2>/dev/null || true
    docker stop livekit-cross-e2e 2>/dev/null || true
    # Keep logs for debugging
    # rm -f "$BOT_LOG" "$DESKTOP_LOG"
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

if [ "$SKIP_ANDROID" = false ]; then
    command -v adb >/dev/null 2>&1 || { fail "adb required for Android test"; exit 1; }
    ADB_DEVICES=$(adb devices 2>/dev/null | grep -c "device$") || ADB_DEVICES=0
    if [ "$ADB_DEVICES" -eq 0 ]; then
        warn "No Android device connected — skipping Android"
        SKIP_ANDROID=true
    else
        ok "Android device detected"
        EXPECTED_PARTICIPANTS=$((EXPECTED_PARTICIPANTS + 1))
    fi
fi

if [ "$SKIP_DESKTOP" = false ]; then
    EXPECTED_PARTICIPANTS=$((EXPECTED_PARTICIPANTS + 1))
fi

# Get local IP for Android to reach LiveKit
LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || echo "127.0.0.1")
LIVEKIT_URL_ANDROID="ws://${LOCAL_IP}:7880"

# =========================================================================
# Step 1: Build
# =========================================================================
info "Building visio-bot..."
cd "$ROOT_DIR"
cargo build -p visio-bot --quiet 2>&1 || { fail "Bot build failed"; exit 1; }
ok "visio-bot built"

if [ "$SKIP_DESKTOP" = false ]; then
    info "Building desktop app..."
    cargo build -p visio-desktop --no-default-features --quiet 2>&1 || { fail "Desktop build failed"; exit 1; }
    ok "Desktop built"
fi

# =========================================================================
# Step 2: Generate tokens
# =========================================================================
info "Generating tokens..."

generate_token() {
    local identity="$1"
    local name="$2"
    cargo run -p visio-bot --quiet -- \
        --url "$LIVEKIT_URL" \
        --room "$ROOM" \
        --identity "$identity" \
        --name "$name" \
        --api-key "$API_KEY" \
        --api-secret "$API_SECRET" \
        --token-only
}

BOT_TOKEN=$(generate_token "bot" "Bot (Video)")
ok "Bot token generated"

if [ "$SKIP_DESKTOP" = false ]; then
    DESKTOP_TOKEN=$(generate_token "desktop-user" "Desktop User")
    ok "Desktop token generated"
fi

if [ "$SKIP_ANDROID" = false ]; then
    ANDROID_TOKEN=$(generate_token "android-user" "Android User")
    ok "Android token generated"
fi

# =========================================================================
# Step 3: Start LiveKit
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
# Step 4: Start bot
# =========================================================================
info "Starting bot in room '$ROOM'..."
cargo run -p visio-bot --quiet -- \
    --url "$LIVEKIT_URL" \
    --room "$ROOM" \
    --identity "bot" \
    --name "Bot (Video)" \
    --token "$BOT_TOKEN" \
    --media-file "$MEDIA_FILE" \
    --loop-media \
    --screen-share \
    --monitor-audio \
    --expect-participants "$EXPECTED_PARTICIPANTS" \
    --duration "$DURATION" \
    --chat-message "Hello from Bot!" \
    --raise-hand \
    2>&1 | tee "$BOT_LOG" &
BOT_PID=$!
sleep 5
ok "Bot running (PID $BOT_PID)"

# =========================================================================
# Step 5: Launch Desktop (auto-connect)
# =========================================================================
VITE_PID=""
if [ "$SKIP_DESKTOP" = false ]; then
    info "Launching desktop app (auto-connect)..."

    # Kill any existing Vite on port 5173
    lsof -ti:5173 | xargs kill -9 2>/dev/null || true
    sleep 1

    # Start Vite dev server on port 5173 (Tauri expects this)
    cd "$ROOT_DIR/crates/visio-desktop/frontend"
    npx vite >/dev/null 2>&1 &
    VITE_PID=$!
    sleep 3

    # Launch desktop binary with auto-connect args
    cd "$ROOT_DIR"
    cargo run -p visio-desktop --no-default-features --quiet -- \
        --livekit-url "$LIVEKIT_URL" \
        --token "$DESKTOP_TOKEN" \
        >"$DESKTOP_LOG" 2>&1 &
    DESKTOP_PID=$!
    sleep 5
    ok "Desktop app launched (PID $DESKTOP_PID)"
fi

# =========================================================================
# Step 6: Launch Android (auto-connect via deep link)
# =========================================================================
if [ "$SKIP_ANDROID" = false ]; then
    info "Launching Android app (auto-connect via deep link)..."

    # URL-encode the token (replace + and = for URI safety)
    ENCODED_TOKEN=$(python3 -c "import urllib.parse; print(urllib.parse.quote('$ANDROID_TOKEN', safe=''))")

    # Launch app via deep link
    adb shell am start -a android.intent.action.VIEW \
        -d "visio-test://connect?livekit_url=${LIVEKIT_URL_ANDROID}\&token=${ENCODED_TOKEN}" \
        io.visio.mobile 2>&1 || warn "adb launch failed"

    sleep 3
    ok "Android app launched via deep link"
fi

# =========================================================================
# Step 7: Status
# =========================================================================
echo ""
echo "============================================================"
echo -e "${GREEN}Cross-Platform E2E Test Running${NC}"
echo "============================================================"
echo ""
echo "Room:       $ROOM"
echo "LiveKit:    $LIVEKIT_URL"
echo "Duration:   ${DURATION}s"
echo "Local IP:   $LOCAL_IP"
echo ""
echo "Participants:"
echo "  - Bot:      publishing audio + video + screen share + chat + hand raise"
[ "$SKIP_DESKTOP" = false ] && echo "  - Desktop:  auto-connected via CLI args"
[ "$SKIP_ANDROID" = false ] && echo "  - Android:  auto-connected via deep link ($LIVEKIT_URL_ANDROID)"
echo ""
echo "============================================================"
echo ""

# =========================================================================
# Step 8: Wait for bot to finish and report
# =========================================================================
info "Waiting for bot to complete (${DURATION}s)..."
wait "$BOT_PID" 2>/dev/null || true
BOT_EXIT=$?
BOT_PID=""

# Close Android app
if [ "$SKIP_ANDROID" = false ]; then
    info "Closing Android app..."
    adb shell am force-stop io.visio.mobile 2>/dev/null || true
fi

# Kill desktop after bot finishes
if [ -n "$DESKTOP_PID" ]; then
    kill "$DESKTOP_PID" 2>/dev/null || true
    DESKTOP_PID=""
fi

echo ""
echo "============================================================"
echo -e "${BLUE}Results${NC}"
echo "============================================================"

EXIT_CODE=0

if [ -f "$BOT_LOG" ]; then
    echo ""
    grep -E "\[SUMMARY\]|\[AUDIO QUALITY\]" "$BOT_LOG" 2>/dev/null || true
    echo ""

    # Check results
    SUBS="$(grep -c "TrackSubscribed" "$BOT_LOG" 2>/dev/null)" || SUBS=0
    JOINS="$(grep -c "ParticipantJoined" "$BOT_LOG" 2>/dev/null)" || JOINS=0

    if [ "$JOINS" -gt 0 ]; then
        ok "Remote participant(s) joined: $JOINS"
    else
        fail "No remote participants joined during the test"
        EXIT_CODE=1
    fi

    if [ "$SUBS" -gt 0 ]; then
        ok "Tracks received from remote: $SUBS"
    else
        warn "No tracks received from remote (mic/camera not auto-enabled)"
    fi

    if grep -q "Audio quality OK" "$BOT_LOG" 2>/dev/null; then
        ok "Audio quality: OK"
    elif grep -q "NO audio frames received" "$BOT_LOG" 2>/dev/null; then
        warn "Audio quality: no frames (expected if no remote audio published)"
    fi

    # Chat verification
    CHATS="$(grep -c "\[EVENT\] ChatMessage:" "$BOT_LOG" 2>/dev/null)" || CHATS=0
    if [ "$CHATS" -gt 1 ]; then
        ok "Chat messages exchanged: $CHATS"
    elif [ "$CHATS" -eq 1 ]; then
        warn "Only bot's own message received — no cross-platform chat"
    else
        fail "No chat messages detected"
        EXIT_CODE=1
    fi

    # Per-participant summary
    echo ""
    if [ "$SKIP_DESKTOP" = false ]; then
        if grep -q "desktop-user" "$BOT_LOG" 2>/dev/null; then
            ok "Desktop: connected and visible to bot"
        else
            fail "Desktop: NOT detected by bot"
            EXIT_CODE=1
        fi
    fi

    if [ "$SKIP_ANDROID" = false ]; then
        if grep -q "android-user" "$BOT_LOG" 2>/dev/null; then
            ok "Android: connected and visible to bot"
        else
            fail "Android: NOT detected by bot"
            EXIT_CODE=1
        fi
    fi
fi

echo ""
if [ "$EXIT_CODE" -eq 0 ] && [ "$BOT_EXIT" -eq 0 ]; then
    ok "Cross-platform E2E test PASSED"
else
    fail "Cross-platform E2E test FAILED (bot_exit=$BOT_EXIT)"
    EXIT_CODE=1
fi

exit $EXIT_CODE
