# End-to-End Testing

## Why

Visio is a cross-platform video conferencing app (Android, iOS, desktop) built on a shared Rust core with LiveKit. The E2E test suite verifies that all platforms can:

- Connect to the same LiveKit room
- Exchange audio, video, and screen share tracks
- Send and receive chat messages
- Survive mic/camera toggles without crashing or losing tracks
- Maintain acceptable audio/video quality (no long gaps, no freezes)

This cannot be tested with unit tests or per-platform integration tests alone — the critical bugs live at the intersection of platforms (codec negotiation, track subscription timing, audio echo between participants).

## Architecture

```
e2e/
├── scripts/
│   ├── run-cross-platform-e2e.sh   # Main orchestrator
│   ├── run-maestro-tests.sh        # Android UI tests
│   ├── run-playwright-tests.sh     # Desktop UI tests
│   └── download-test-media.sh      # Download test video for bot
├── visio-bot/                      # Headless Rust bot (LiveKit participant)
│   ├── Cargo.toml
│   └── src/main.rs
├── maestro/                        # Maestro YAML flows for Android
│   ├── config.yaml
│   ├── 01_app_launch.yaml
│   ├── ...
│   └── run_all.yaml
└── test-assets/                    # Downloaded media + logs (gitignored)
    ├── test-video.mp4
    ├── bot-output.log
    └── desktop-output.log
```

Platform-specific test files remain colocated with their platform code:
- `crates/visio-desktop/frontend/e2e/` — Playwright specs for desktop frontend
- `ios/VisioMobileUITests/` — XCUITest for iOS
- `android/app/src/androidTest/` — Compose instrumentation tests for Android
- `crates/visio-core/tests/` — Rust integration tests (require LiveKit Docker)

## The Cross-Platform E2E Test

### What it does

The main test (`e2e/scripts/run-cross-platform-e2e.sh`) spins up a local LiveKit server in Docker, then connects 4 participants to the same room:

| Participant | Platform | How it connects | What it does |
|---|---|---|---|
| **visio-bot** | Rust binary | Direct LiveKit SDK | Publishes synthetic/real audio+video+screen share, monitors quality |
| **Desktop** | Tauri app | CLI args `--livekit-url --token` | Auto-connects, toggles mic/camera/screen share |
| **Android** | Kotlin app | Deep link `visio-test://connect?...` via adb | Auto-connects, toggles mic/camera |
| **iOS** | Swift app | Deep link `visio-test://connect?...` via simctl | Auto-connects, toggles mic/camera (synthetic audio on simulator) |

### Turn-based speaking

To measure each participant's audio independently, the test uses a turn-based schedule:

| Time | Who speaks | Others |
|---|---|---|
| 0–5s | All (warmup) | — |
| 5–25s | Bot | Desktop, Android, iOS muted |
| 25–50s | Desktop | Bot, Android, iOS muted |
| 50–75s | Android | Bot, Desktop, iOS muted |
| 75–100s | iOS | Bot, Desktop, Android muted |
| 100–120s | All | Everyone unmuted |

This schedule is hardcoded in each platform's E2E auto-connect code. The bot resets its gap timers on mute/unmute events so intentional silence doesn't count as quality degradation.

### Quality gates

The bot monitors incoming audio and video frames per participant and reports:
- **Frame count**: did we receive any frames at all?
- **Max gap**: longest gap between consecutive frames (audio threshold: 200ms, video: 2000ms)
- **FPS**: average video framerate
- **Silent frames**: percentage of near-zero audio frames

The orchestration script parses the bot's log output and exits non-zero if any quality gate fails.

### Orientation rotation

During Android's and iOS's speaking turns, the script rotates the device orientation (portrait → landscape → portrait) to verify the app survives orientation changes without dropping the call.

## Running the tests

### Prerequisites

- Docker (for LiveKit server)
- Rust toolchain (builds visio-bot)
- ffmpeg (for bot video playback)
- For Android: device/emulator with APK installed + adb
- For iOS: simulator with app installed + xcrun simctl
- For Desktop: Tauri dev environment

### Quick start

```sh
# Download test video (one-time)
./e2e/scripts/download-test-media.sh

# Run full cross-platform test (120s, all platforms)
./e2e/scripts/run-cross-platform-e2e.sh

# Desktop + bot only
./e2e/scripts/run-cross-platform-e2e.sh --no-android --no-ios

# Bot + Android only
./e2e/scripts/run-cross-platform-e2e.sh --no-desktop --no-ios

# Custom duration
DURATION=60 ./e2e/scripts/run-cross-platform-e2e.sh
```

### Other test runners

```sh
# Android Maestro UI tests (no LiveKit needed for basic flows)
./e2e/scripts/run-maestro-tests.sh

# Desktop Playwright tests (frontend only, mocked Tauri API)
./e2e/scripts/run-playwright-tests.sh

# Rust integration tests (requires LiveKit Docker)
docker run -d --rm --name livekit-e2e -p 7880:7880 livekit/livekit-server --dev
cargo test -p visio-core --test integration_livekit
```

## The visio-bot

`e2e/visio-bot/` is a headless LiveKit participant used as a deterministic test partner. It:

- Publishes synthetic media (440Hz sine wave + colored video frames) or real media from an mp4 file via ffmpeg
- Sends chat messages, emoji reactions, and hand raises on a schedule
- Subscribes to all remote tracks and monitors frame quality per participant
- Logs all LiveKit events for post-test analysis
- Implements the turn-based speaking schedule (mute/unmute at specific times)

```sh
# Standalone usage
cargo run -p visio-bot -- \
  --url ws://localhost:7880 \
  --room my-test-room \
  --duration 60 \
  --monitor-audio \
  --expect-participants 1
```

## Auto-connect hooks

Each platform has a test-only auto-connect mechanism that allows the E2E script to inject connection parameters without user interaction:

- **Desktop**: Tauri CLI args `--livekit-url <url> --token <jwt>` → emits `"auto-connect"` event to frontend
- **Android**: Deep link `visio-test://connect?livekit_url=<url>&token=<jwt>` handled in `MainActivity.kt`
- **iOS**: Deep link `visio-test://connect?livekit_url=<url>&token=<jwt>` handled in `VisioManager.swift`

These hooks are embedded in production code but only triggered by the test-specific URL scheme (`visio-test://`), not the production scheme (`visio://`).

## Synthetic audio

On emulators and simulators that lack a real microphone, synthetic audio capture generates a 440Hz sine wave at 48kHz/mono and pushes it through the same FFI path as real microphone audio:

- **Android**: `AudioCapture.startSynthetic()` in `AudioCapture.kt`
- **iOS**: `SyntheticAudioCapture.swift` → `visio_push_ios_audio_frame()` C FFI
- **Bot**: Built-in `generate_sine_frame()` in `main.rs`

This ensures the audio pipeline is tested end-to-end even without physical hardware.
