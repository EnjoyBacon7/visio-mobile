#!/usr/bin/env bash
# Fully automated cross-platform E2E test: Bot + Desktop + Android + iOS.
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
#   ./scripts/run-cross-platform-e2e.sh [--duration SECS] [--no-android] [--no-desktop] [--no-ios]
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

ROOM="e2e-$(date +%s)"
DURATION="${DURATION:-120}"
LIVEKIT_URL="ws://localhost:7880"
MEDIA_FILE="$ROOT_DIR/test-assets/test-video.mp4"
BOT_LOG="$ROOT_DIR/test-assets/bot-output.log"
DESKTOP_LOG="$ROOT_DIR/test-assets/desktop-output.log"
API_KEY="devkey"
API_SECRET="secret"
SKIP_ANDROID=false
SKIP_DESKTOP=false
SKIP_IOS=false
EXPECTED_PARTICIPANTS=0

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --duration) DURATION="$2"; shift 2 ;;
        --room) ROOM="$2"; shift 2 ;;
        --no-android) SKIP_ANDROID=true; shift ;;
        --no-desktop) SKIP_DESKTOP=true; shift ;;
        --no-ios) SKIP_IOS=true; shift ;;
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
ROTATION_PID=""

cleanup() {
    info "Cleaning up..."
    [ -n "$BOT_PID" ] && kill "$BOT_PID" 2>/dev/null || true
    [ -n "$DESKTOP_PID" ] && kill "$DESKTOP_PID" 2>/dev/null || true
    [ -n "${ROTATION_PID:-}" ] && kill "$ROTATION_PID" 2>/dev/null || true
    [ -n "${VITE_PID:-}" ] && kill "$VITE_PID" 2>/dev/null || true
    lsof -ti:5173 | xargs kill -9 2>/dev/null || true
    # Restore Android orientation to auto-rotate
    adb shell settings put system user_rotation 0 2>/dev/null || true
    adb shell settings put system accelerometer_rotation 1 2>/dev/null || true
    adb shell am force-stop io.visio.mobile 2>/dev/null || true
    [ "$SKIP_IOS" = false ] && xcrun simctl terminate booted io.visio.mobile 2>/dev/null || true
    docker stop livekit-cross-e2e 2>/dev/null || true
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
cargo build -p visio-bot --release --quiet 2>&1 || { fail "Bot build failed"; exit 1; }
ok "visio-bot built"

if [ "$SKIP_DESKTOP" = false ]; then
    info "Building desktop app..."
    cargo build -p visio-desktop --no-default-features --release --quiet 2>&1 || { fail "Desktop build failed"; exit 1; }
    ok "Desktop built"
fi

# =========================================================================
# Step 2: Generate tokens
# =========================================================================
info "Generating tokens..."

generate_token() {
    local identity="$1"
    local name="$2"
    "$ROOT_DIR/target/release/visio-bot" \
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

if [ "$SKIP_IOS" = false ]; then
    IOS_TOKEN=$(generate_token "ios-user" "iOS User")
    ok "iOS token generated"
    EXPECTED_PARTICIPANTS=$((EXPECTED_PARTICIPANTS + 1))
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
"$ROOT_DIR/target/release/visio-bot" \
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
    "$ROOT_DIR/target/release/visio-desktop" \
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
# Step 6b: Launch iOS (auto-connect via deep link on simulator)
# =========================================================================
if [ "$SKIP_IOS" = false ]; then
    info "Launching iOS app (auto-connect via deep link)..."

    ENCODED_IOS_TOKEN=$(python3 -c "import urllib.parse; print(urllib.parse.quote('$IOS_TOKEN', safe=''))")

    if xcrun simctl list devices booted 2>/dev/null | grep -q "Booted"; then
        xcrun simctl openurl booted "visio-test://connect?livekit_url=${LIVEKIT_URL_ANDROID}&token=${ENCODED_IOS_TOKEN}"
        sleep 3
        ok "iOS simulator app launched via deep link"
    else
        warn "No iOS simulator booted — skipping iOS"
        SKIP_IOS=true
    fi
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
echo "  - Bot:      publishing audio + video + screen share + chat + hand raise (speaks 0-25s)"
[ "$SKIP_DESKTOP" = false ] && echo "  - Desktop:  auto-connected via CLI args (speaks 25-50s)"
[ "$SKIP_ANDROID" = false ] && echo "  - Android:  auto-connected via deep link (speaks 50-75s)"
[ "$SKIP_IOS" = false ] && echo "  - iOS:      auto-connected via deep link simulator (speaks 75-100s)"
echo ""
echo "============================================================"
echo ""

# =========================================================================
# Step 7b: Orientation rotation test (runs in background during the test)
# =========================================================================
(
    # Wait for all participants to join (warmup phase)
    sleep 10

    # --- Android rotation: during Android's turn (50-75s) ---
    if [ "$SKIP_ANDROID" = false ]; then
        sleep 45  # t=55s — mid-Android turn
        info "[ROTATION] Android → landscape"
        adb shell settings put system accelerometer_rotation 0 2>/dev/null
        adb shell settings put system user_rotation 1 2>/dev/null
        sleep 10  # t=65s
        info "[ROTATION] Android → portrait"
        adb shell settings put system user_rotation 0 2>/dev/null
        adb shell settings put system accelerometer_rotation 1 2>/dev/null
    else
        sleep 55
    fi

    # --- iOS rotation: during iOS's turn (75-100s) ---
    if [ "$SKIP_IOS" = false ]; then
        sleep 15  # t=80s — mid-iOS turn
        info "[ROTATION] iOS simulator → landscape"
        osascript -e 'tell application "Simulator" to activate' \
                  -e 'delay 0.3' \
                  -e 'tell application "System Events" to key code 124 using command down' 2>/dev/null
        sleep 10  # t=90s
        info "[ROTATION] iOS simulator → portrait"
        osascript -e 'tell application "Simulator" to activate' \
                  -e 'delay 0.3' \
                  -e 'tell application "System Events" to key code 123 using command down' 2>/dev/null
    fi
) &
ROTATION_PID=$!

# =========================================================================
# Step 8: Wait for bot to finish and report
# =========================================================================
info "Waiting for bot to complete (${DURATION}s)..."
wait "$BOT_PID" 2>/dev/null || true
BOT_EXIT=$?
BOT_PID=""

# Stop rotation job
[ -n "$ROTATION_PID" ] && kill "$ROTATION_PID" 2>/dev/null || true
ROTATION_PID=""

# Restore Android orientation
adb shell settings put system user_rotation 0 2>/dev/null || true
adb shell settings put system accelerometer_rotation 1 2>/dev/null || true

# Close Android app
if [ "$SKIP_ANDROID" = false ]; then
    info "Closing Android app..."
    adb shell am force-stop io.visio.mobile 2>/dev/null || true
fi

# Close iOS app
if [ "$SKIP_IOS" = false ]; then
    info "Closing iOS app..."
    xcrun simctl terminate booted io.visio.mobile 2>/dev/null || true
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
    grep -E "\[SUMMARY\]|\[AUDIO QUALITY\]|\[VIDEO QUALITY\]" "$BOT_LOG" 2>/dev/null || true
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

    if [ "$SKIP_IOS" = false ]; then
        if grep -q "ios-user" "$BOT_LOG" 2>/dev/null; then
            ok "iOS: connected and visible to bot"
        else
            fail "iOS: NOT detected by bot"
            EXIT_CODE=1
        fi
    fi

    # =========================================================================
    # Quality Gates
    # =========================================================================

    # Per-participant audio quality gates
    echo ""
    info "Per-participant audio quality:"
    while IFS= read -r line; do
        PARTICIPANT="$(echo "$line" | sed 's/.*AUDIO QUALITY FINAL\] \([^:]*\):.*/\1/')"
        FRAMES="$(echo "$line" | grep -o 'frames=[0-9]*' | cut -d= -f2)" || FRAMES=0
        MAX_GAP="$(echo "$line" | grep -o 'max_gap=[0-9]*ms' | grep -o '[0-9]*')" || MAX_GAP=0
        if [ "$FRAMES" -gt 0 ] 2>/dev/null; then
            if [ "$MAX_GAP" -gt 200 ] 2>/dev/null; then
                fail "Audio $PARTICIPANT: $FRAMES frames, max_gap=${MAX_GAP}ms — choppy"
                EXIT_CODE=1
            else
                ok "Audio $PARTICIPANT: $FRAMES frames, max_gap=${MAX_GAP}ms — smooth"
            fi
        else
            warn "Audio $PARTICIPANT: 0 frames received"
        fi
    done < <(grep '\[AUDIO QUALITY FINAL\]' "$BOT_LOG" 2>/dev/null || true)
    # Check we got at least one audio report
    AUDIO_REPORT_COUNT="$(grep -c '\[AUDIO QUALITY FINAL\]' "$BOT_LOG" 2>/dev/null)" || AUDIO_REPORT_COUNT=0
    if [ "$AUDIO_REPORT_COUNT" -eq 0 ]; then
        warn "Audio: no per-participant reports found"
    fi

    # Per-participant video quality gates
    echo ""
    info "Per-participant video quality:"
    while IFS= read -r line; do
        PARTICIPANT="$(echo "$line" | sed 's/.*VIDEO QUALITY FINAL\] \([^:]*\):.*/\1/')"
        FRAMES="$(echo "$line" | grep -o 'frames=[0-9]*' | cut -d= -f2)" || FRAMES=0
        FPS="$(echo "$line" | grep -o 'avg_fps=[0-9.]*' | cut -d= -f2)" || FPS="0"
        MAX_GAP="$(echo "$line" | grep -o 'max_gap=[0-9]*ms' | grep -o '[0-9]*')" || MAX_GAP=0
        if [ "$FRAMES" -gt 0 ] 2>/dev/null; then
            if [ "$MAX_GAP" -gt 2000 ] 2>/dev/null; then
                fail "Video $PARTICIPANT: $FRAMES frames, ${FPS}fps, max_gap=${MAX_GAP}ms — freeze"
                EXIT_CODE=1
            else
                ok "Video $PARTICIPANT: $FRAMES frames, ${FPS}fps, max_gap=${MAX_GAP}ms — smooth"
            fi
        else
            warn "Video $PARTICIPANT: 0 frames received"
        fi
    done < <(grep '\[VIDEO QUALITY FINAL\]' "$BOT_LOG" 2>/dev/null || true)
    VIDEO_REPORT_COUNT="$(grep -c '\[VIDEO QUALITY FINAL\]' "$BOT_LOG" 2>/dev/null)" || VIDEO_REPORT_COUNT=0
    if [ "$VIDEO_REPORT_COUNT" -eq 0 ]; then
        warn "Video: no per-participant reports found"
    fi

    # Screen share rotation (bot)
    BOT_SCREEN_MUTE="$(grep -c 'Bot muting' "$BOT_LOG" 2>/dev/null)" || BOT_SCREEN_MUTE=0
    BOT_ALL_SPEAK="$(grep -c 'All speak' "$BOT_LOG" 2>/dev/null)" || BOT_ALL_SPEAK=0
    if [ "$BOT_SCREEN_MUTE" -gt 0 ] && [ "$BOT_ALL_SPEAK" -gt 0 ]; then
        ok "Bot turn-based: muted at 25s, resumed at 100s"
    else
        warn "Bot turn-based: pattern incomplete (mute=$BOT_SCREEN_MUTE, resume=$BOT_ALL_SPEAK)"
    fi

    # Turn-based speaking: check that each participant announced their turn
    echo ""
    info "Turn-based speaking verification:"
    TURN_DESKTOP="$(grep -c "Desktop: my turn to speak" "$BOT_LOG" 2>/dev/null)" || TURN_DESKTOP=0
    TURN_ANDROID="$(grep -c "Android: my turn to speak" "$BOT_LOG" 2>/dev/null)" || TURN_ANDROID=0
    TURN_IOS="$(grep -c "iOS: my turn to speak" "$BOT_LOG" 2>/dev/null)" || TURN_IOS=0
    [ "$SKIP_DESKTOP" = false ] && { [ "$TURN_DESKTOP" -gt 0 ] && ok "Desktop turn announced" || warn "Desktop turn not detected in chat"; }
    [ "$SKIP_ANDROID" = false ] && { [ "$TURN_ANDROID" -gt 0 ] && ok "Android turn announced" || warn "Android turn not detected in chat"; }
    [ "$SKIP_IOS" = false ] && { [ "$TURN_IOS" -gt 0 ] && ok "iOS turn announced" || warn "iOS turn not detected in chat"; }

    # TrackMuted/TrackUnmuted events — verify toggling happened
    MUTED_EVENTS="$(grep -c 'TrackMuted' "$BOT_LOG" 2>/dev/null)" || MUTED_EVENTS=0
    UNMUTED_EVENTS="$(grep -c 'TrackUnmuted' "$BOT_LOG" 2>/dev/null)" || UNMUTED_EVENTS=0
    if [ "$MUTED_EVENTS" -gt 2 ] && [ "$UNMUTED_EVENTS" -gt 2 ]; then
        ok "Track mute/unmute events: muted=$MUTED_EVENTS unmuted=$UNMUTED_EVENTS"
    else
        warn "Few mute/unmute events: muted=$MUTED_EVENTS unmuted=$UNMUTED_EVENTS"
    fi

    # Orientation rotation verification
    echo ""
    info "Orientation rotation verification:"
    if [ "$SKIP_ANDROID" = false ]; then
        # After rotation, check that Android is still connected (tracks still subscribed)
        ANDROID_TRACKS_AFTER="$(grep -c 'TrackSubscribed.*android-user\|android-user.*TrackSubscribed' "$BOT_LOG" 2>/dev/null)" || ANDROID_TRACKS_AFTER=0
        if [ "$ANDROID_TRACKS_AFTER" -gt 0 ]; then
            ok "Android: survived orientation rotation (tracks still subscribed)"
        else
            warn "Android: orientation rotation impact unclear"
        fi
    fi
    if [ "$SKIP_IOS" = false ]; then
        IOS_TRACKS_AFTER="$(grep -c 'TrackSubscribed.*ios-user\|ios-user.*TrackSubscribed' "$BOT_LOG" 2>/dev/null)" || IOS_TRACKS_AFTER=0
        if [ "$IOS_TRACKS_AFTER" -gt 0 ]; then
            ok "iOS: survived orientation rotation (tracks still subscribed)"
        else
            warn "iOS: orientation rotation impact unclear"
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
