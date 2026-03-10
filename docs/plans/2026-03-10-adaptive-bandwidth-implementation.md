# Adaptive Bandwidth & Android Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Preserve audio quality under degraded network by progressively disabling video, fix Android audio device initialization race condition, and fix Android screen share display.

**Architecture:** A `BandwidthController` in visio-core monitors `ConnectionQualityChanged` events and uses LiveKit's `RemoteTrackPublication` API (`set_enabled`, `set_video_quality`) to degrade/restore video tracks. Android audio device fix reorders `setPreferredDevice()` before `play()`/`startRecording()`. Android screen share fix adds a pending-surface mechanism in JNI.

**Tech Stack:** Rust (LiveKit SDK 0.7.32), Kotlin (Android AudioTrack/AudioRecord), Tauri (Desktop UI)

---

### Task 1: BandwidthController — core module with tests

**Files:**
- Create: `crates/visio-core/src/bandwidth.rs`
- Modify: `crates/visio-core/src/lib.rs`

**Step 1: Write the failing tests**

Add `crates/visio-core/src/bandwidth.rs` with tests first:

```rust
use std::time::{Duration, Instant};

use crate::events::ConnectionQuality;

/// Bandwidth degradation level, ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandwidthMode {
    /// All video at full quality.
    Full,
    /// Video downgraded to low quality (simulcast low layer).
    ReducedVideo,
    /// All video disabled, audio only.
    AudioOnly,
}

/// Tracks connection quality transitions and decides when to change
/// bandwidth mode, applying a 5-second hysteresis before upgrading.
pub struct BandwidthController {
    current_mode: BandwidthMode,
    last_quality: ConnectionQuality,
    /// When quality improved, the instant it happened.
    upgrade_pending_since: Option<Instant>,
    /// How long quality must remain improved before upgrading mode.
    hysteresis: Duration,
    /// After entering Poor, how long before cutting non-speaker video.
    poor_escalation: Duration,
    /// When quality first went to Poor in current streak.
    poor_since: Option<Instant>,
}

impl BandwidthController {
    pub fn new() -> Self {
        Self {
            current_mode: BandwidthMode::Full,
            last_quality: ConnectionQuality::Excellent,
            upgrade_pending_since: None,
            hysteresis: Duration::from_secs(5),
            poor_escalation: Duration::from_secs(3),
            poor_since: None,
        }
    }

    /// Feed a new connection quality reading. Returns the new mode if it changed.
    pub fn update(&mut self, quality: ConnectionQuality) -> Option<BandwidthMode> {
        self.update_with_time(quality, Instant::now())
    }

    /// Testable version that accepts a timestamp.
    pub fn update_with_time(
        &mut self,
        quality: ConnectionQuality,
        now: Instant,
    ) -> Option<BandwidthMode> {
        let previous_mode = self.current_mode;

        // Determine the target mode for this quality level
        let target_mode = match quality {
            ConnectionQuality::Lost => BandwidthMode::AudioOnly,
            ConnectionQuality::Poor => {
                // First time entering Poor: start escalation timer
                if self.poor_since.is_none() {
                    self.poor_since = Some(now);
                }
                // Phase 1: reduce video quality immediately
                // Phase 2: after poor_escalation, go audio-only
                if now.duration_since(self.poor_since.unwrap()) >= self.poor_escalation {
                    BandwidthMode::AudioOnly
                } else {
                    BandwidthMode::ReducedVideo
                }
            }
            ConnectionQuality::Good | ConnectionQuality::Excellent => {
                self.poor_since = None;
                BandwidthMode::Full
            }
        };

        // Downgrade: apply immediately
        if target_mode_rank(target_mode) > target_mode_rank(self.current_mode) {
            self.current_mode = target_mode;
            self.upgrade_pending_since = None;
            self.last_quality = quality;
            return if self.current_mode != previous_mode {
                Some(self.current_mode)
            } else {
                None
            };
        }

        // Upgrade: apply only after hysteresis
        if target_mode_rank(target_mode) < target_mode_rank(self.current_mode) {
            match self.upgrade_pending_since {
                None => {
                    self.upgrade_pending_since = Some(now);
                }
                Some(since) if now.duration_since(since) >= self.hysteresis => {
                    self.current_mode = target_mode;
                    self.upgrade_pending_since = None;
                }
                _ => {} // Still waiting for hysteresis
            }
        } else {
            // Same level — reset upgrade timer
            self.upgrade_pending_since = None;
        }

        self.last_quality = quality;
        if self.current_mode != previous_mode {
            Some(self.current_mode)
        } else {
            None
        }
    }

    pub fn current_mode(&self) -> BandwidthMode {
        self.current_mode
    }
}

/// Higher rank = worse quality.
fn target_mode_rank(mode: BandwidthMode) -> u8 {
    match mode {
        BandwidthMode::Full => 0,
        BandwidthMode::ReducedVideo => 1,
        BandwidthMode::AudioOnly => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_full_mode() {
        let ctrl = BandwidthController::new();
        assert_eq!(ctrl.current_mode(), BandwidthMode::Full);
    }

    #[test]
    fn poor_immediately_reduces_video() {
        let mut ctrl = BandwidthController::new();
        let now = Instant::now();
        let result = ctrl.update_with_time(ConnectionQuality::Poor, now);
        assert_eq!(result, Some(BandwidthMode::ReducedVideo));
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);
    }

    #[test]
    fn poor_escalates_to_audio_only_after_3s() {
        let mut ctrl = BandwidthController::new();
        let t0 = Instant::now();
        ctrl.update_with_time(ConnectionQuality::Poor, t0);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Still Poor after 2s — no escalation yet
        let t2 = t0 + Duration::from_secs(2);
        let result = ctrl.update_with_time(ConnectionQuality::Poor, t2);
        assert_eq!(result, None);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Still Poor after 3s — escalate
        let t3 = t0 + Duration::from_secs(3);
        let result = ctrl.update_with_time(ConnectionQuality::Poor, t3);
        assert_eq!(result, Some(BandwidthMode::AudioOnly));
    }

    #[test]
    fn lost_immediately_goes_audio_only() {
        let mut ctrl = BandwidthController::new();
        let now = Instant::now();
        let result = ctrl.update_with_time(ConnectionQuality::Lost, now);
        assert_eq!(result, Some(BandwidthMode::AudioOnly));
    }

    #[test]
    fn good_does_not_restore_before_hysteresis() {
        let mut ctrl = BandwidthController::new();
        let t0 = Instant::now();
        ctrl.update_with_time(ConnectionQuality::Poor, t0);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Quality improves immediately — should NOT upgrade yet
        let t1 = t0 + Duration::from_millis(100);
        let result = ctrl.update_with_time(ConnectionQuality::Good, t1);
        assert_eq!(result, None);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);
    }

    #[test]
    fn good_restores_after_hysteresis() {
        let mut ctrl = BandwidthController::new();
        let t0 = Instant::now();
        ctrl.update_with_time(ConnectionQuality::Poor, t0);

        // Quality improves
        let t1 = t0 + Duration::from_secs(1);
        ctrl.update_with_time(ConnectionQuality::Good, t1);

        // Wait 5 seconds
        let t6 = t1 + Duration::from_secs(5);
        let result = ctrl.update_with_time(ConnectionQuality::Good, t6);
        assert_eq!(result, Some(BandwidthMode::Full));
    }

    #[test]
    fn hysteresis_resets_on_quality_drop() {
        let mut ctrl = BandwidthController::new();
        let t0 = Instant::now();
        ctrl.update_with_time(ConnectionQuality::Poor, t0);

        // Quality improves for 3s
        let t1 = t0 + Duration::from_secs(1);
        ctrl.update_with_time(ConnectionQuality::Good, t1);
        let t4 = t1 + Duration::from_secs(3);
        ctrl.update_with_time(ConnectionQuality::Good, t4);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        // Quality drops again — resets hysteresis
        let t5 = t4 + Duration::from_millis(100);
        ctrl.update_with_time(ConnectionQuality::Poor, t5);

        // Quality improves again — hysteresis restarts
        let t6 = t5 + Duration::from_secs(1);
        ctrl.update_with_time(ConnectionQuality::Good, t6);
        let t10 = t6 + Duration::from_secs(4);
        ctrl.update_with_time(ConnectionQuality::Good, t10);
        assert_eq!(ctrl.current_mode(), BandwidthMode::ReducedVideo);

        let t12 = t6 + Duration::from_secs(5);
        let result = ctrl.update_with_time(ConnectionQuality::Good, t12);
        assert_eq!(result, Some(BandwidthMode::Full));
    }

    #[test]
    fn excellent_and_good_are_equivalent_for_full() {
        let mut ctrl = BandwidthController::new();
        let now = Instant::now();
        let result = ctrl.update_with_time(ConnectionQuality::Excellent, now);
        assert_eq!(result, None); // Already Full
        let result = ctrl.update_with_time(ConnectionQuality::Good, now);
        assert_eq!(result, None); // Still Full
    }

    #[test]
    fn no_change_emitted_when_mode_stays_same() {
        let mut ctrl = BandwidthController::new();
        let now = Instant::now();
        let result = ctrl.update_with_time(ConnectionQuality::Good, now);
        assert_eq!(result, None);
    }
}
```

**Step 2: Register the module**

In `crates/visio-core/src/lib.rs`, add:

```rust
pub mod bandwidth;
```

**Step 3: Run tests to verify they pass**

Run: `cargo test -p visio-core -- bandwidth`
Expected: All 8 tests pass.

**Step 4: Commit**

```bash
git add crates/visio-core/src/bandwidth.rs crates/visio-core/src/lib.rs
git commit -m "feat(core): add BandwidthController with hysteresis logic"
```

---

### Task 2: BandwidthController — wire into event loop

**Files:**
- Modify: `crates/visio-core/src/events.rs:4-54` — add `BandwidthModeChanged` event
- Modify: `crates/visio-core/src/room.rs:25-50` — add bandwidth controller field
- Modify: `crates/visio-core/src/room.rs:1113-1136` — integrate with ConnectionQualityChanged handler

**Step 1: Add the event variant**

In `crates/visio-core/src/events.rs`, add to the `VisioEvent` enum (after `AdaptiveModeChanged`):

```rust
/// Bandwidth mode changed due to network quality.
BandwidthModeChanged {
    mode: crate::bandwidth::BandwidthMode,
},
```

**Step 2: Add BandwidthController to RoomManager**

In `crates/visio-core/src/room.rs`, add to `RoomManager` struct:

```rust
bandwidth: Arc<std::sync::Mutex<bandwidth::BandwidthController>>,
```

Initialize it in `RoomManager::new()`:

```rust
bandwidth: Arc::new(std::sync::Mutex::new(bandwidth::BandwidthController::new())),
```

Add `use crate::bandwidth;` at the top.

**Step 3: Wire into ConnectionQualityChanged handler**

In `crates/visio-core/src/room.rs`, in the `ConnectionQualityChanged` handler (around line 1113-1136), after the existing code that updates participant quality and emits the event, add:

```rust
// --- Bandwidth adaptation (only for local participant) ---
{
    let local_sid_opt = participants.lock().await.local_sid().map(|s| s.to_string());
    if local_sid_opt.as_deref() == Some(&psid) {
        let mut bw = bandwidth_ctrl.lock().unwrap();
        if let Some(new_mode) = bw.update(q.clone()) {
            tracing::info!("bandwidth mode changed to {:?}", new_mode);
            emitter.emit(VisioEvent::BandwidthModeChanged { mode: new_mode });

            // Apply video track changes
            if let Some(lk_room) = room_ref.lock().await.as_ref() {
                let remote_participants = lk_room.remote_participants();
                let active = participants.lock().await.active_speakers().first().cloned();

                for (_identity, rp) in &remote_participants {
                    for (_sid, pub_) in rp.track_publications() {
                        if pub_.kind() != LkTrackKind::Video {
                            continue;
                        }
                        match new_mode {
                            bandwidth::BandwidthMode::Full => {
                                pub_.set_enabled(true);
                                pub_.set_video_quality(livekit::track::VideoQuality::High);
                            }
                            bandwidth::BandwidthMode::ReducedVideo => {
                                let is_active_speaker = active.as_deref() == Some(&rp.sid().to_string());
                                pub_.set_enabled(is_active_speaker);
                                if is_active_speaker {
                                    pub_.set_video_quality(livekit::track::VideoQuality::Low);
                                }
                            }
                            bandwidth::BandwidthMode::AudioOnly => {
                                pub_.set_enabled(false);
                            }
                        }
                    }
                }
            }
        }
    }
}
```

You'll need to clone `bandwidth` into the event loop closure as `bandwidth_ctrl` and clone `room` as `room_ref`.

**Step 4: Run tests**

Run: `cargo test -p visio-core`
Expected: All tests pass (existing + new bandwidth tests).

Run: `cargo build -p visio-core`
Expected: Compiles without errors.

**Step 5: Commit**

```bash
git add crates/visio-core/src/events.rs crates/visio-core/src/room.rs
git commit -m "feat(core): wire BandwidthController into room event loop"
```

---

### Task 3: BandwidthController — expose to platforms

**Files:**
- Modify: `crates/visio-ffi/src/lib.rs` — expose BandwidthMode via UniFFI
- Modify: `crates/visio-desktop/src/lib.rs` — emit bandwidth mode to frontend
- Modify: `crates/visio-desktop/frontend/src/App.tsx` — show degraded quality indicator

**Step 1: Add BandwidthMode to FFI**

In `crates/visio-ffi/src/lib.rs`, find where `AdaptiveModeChanged` is handled in the event listener and add a case for `BandwidthModeChanged`:

```rust
VisioEvent::BandwidthModeChanged { mode } => {
    let mode_str = match mode {
        visio_core::bandwidth::BandwidthMode::Full => "full",
        visio_core::bandwidth::BandwidthMode::ReducedVideo => "reduced_video",
        visio_core::bandwidth::BandwidthMode::AudioOnly => "audio_only",
    };
    // Forward to platform callback
}
```

**Step 2: Desktop — emit Tauri event**

In `crates/visio-desktop/src/lib.rs`, in the `VisioEventListener` impl, add:

```rust
VisioEvent::BandwidthModeChanged { mode } => {
    let mode_str = match mode {
        visio_core::bandwidth::BandwidthMode::Full => "full",
        visio_core::bandwidth::BandwidthMode::ReducedVideo => "reduced_video",
        visio_core::bandwidth::BandwidthMode::AudioOnly => "audio_only",
    };
    let _ = app.emit("bandwidth-mode-changed", mode_str);
}
```

**Step 3: Desktop frontend — show indicator**

In `crates/visio-desktop/frontend/src/App.tsx`, add state and listener:

```tsx
const [bandwidthMode, setBandwidthMode] = useState<string>("full");

useEffect(() => {
  const unlisten = listen<string>("bandwidth-mode-changed", (e) => {
    setBandwidthMode(e.payload);
  });
  return () => { unlisten.then(f => f()); };
}, []);
```

Add a small indicator in the top bar when not "full":

```tsx
{bandwidthMode !== "full" && (
  <div className="bandwidth-indicator">
    {bandwidthMode === "reduced_video" ? "Low bandwidth — reduced video" : "Very low bandwidth — audio only"}
  </div>
)}
```

Add CSS for `.bandwidth-indicator` in `App.css`:

```css
.bandwidth-indicator {
  position: fixed;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(255, 100, 0, 0.9);
  color: #fff;
  padding: 4px 16px;
  border-radius: 12px;
  font-size: 0.85rem;
  z-index: 1000;
  pointer-events: none;
}
```

**Step 4: Build and verify**

Run: `cargo build -p visio-core -p visio-desktop`
Expected: Compiles.

**Step 5: Commit**

```bash
git add crates/visio-ffi/src/lib.rs crates/visio-desktop/src/lib.rs crates/visio-desktop/frontend/src/App.tsx crates/visio-desktop/frontend/src/App.css
git commit -m "feat: expose bandwidth mode to platforms with degraded quality indicator"
```

---

### Task 4: Android audio device initialization fix

**Files:**
- Modify: `android/app/src/main/kotlin/io/visio/mobile/AudioPlayout.kt:31-64`
- Modify: `android/app/src/main/kotlin/io/visio/mobile/AudioCapture.kt:35-67`
- Modify: `android/app/src/main/kotlin/io/visio/mobile/VisioManager.kt:334-346,385-461`
- Modify: `android/app/src/main/kotlin/io/visio/mobile/ui/CallScreen.kt:346-362`

**Step 1: AudioPlayout — accept device in start()**

Replace `AudioPlayout.start()` (lines 31-86) to accept an optional device:

```kotlin
fun start(device: AudioDeviceInfo? = null) {
    if (running) return
    running = true

    val minBuf =
        AudioTrack.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_OUT_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
    val bufferSize = maxOf(minBuf * 4, SAMPLES_PER_FRAME * 2 * 4)

    val track =
        AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(SAMPLE_RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .build(),
            )
            .setBufferSizeInBytes(bufferSize)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build()

    audioTrack = track
    // Set preferred device BEFORE play() so routing is applied from the start
    if (device != null) {
        track.setPreferredDevice(device)
        Log.i(TAG, "Audio playout preferred device set before play: ${device.productName}")
    }
    track.play()
    Log.i(TAG, "Audio playout started: ${SAMPLE_RATE}Hz mono, ${FRAME_SIZE_MS}ms frames")

    playThread =
        Thread({
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_URGENT_AUDIO)
            val buffer = ShortArray(SAMPLES_PER_FRAME)

            while (running) {
                val pulled = NativeVideo.nativePullAudioPlayback(buffer)
                if (pulled > 0) {
                    track.write(buffer, 0, pulled)
                } else {
                    Thread.sleep(5)
                }
            }

            track.stop()
            track.release()
            Log.i(TAG, "Audio playout stopped")
        }, "AudioPlayout").also { it.start() }
}
```

**Step 2: AudioCapture — accept device in start()**

Replace `AudioCapture.start()` (lines 35-101) to accept an optional device:

```kotlin
@SuppressLint("MissingPermission")
fun start(device: AudioDeviceInfo? = null) {
    synchronized(lock) {
        if (running) return
        running = true

        val bufferSize =
            maxOf(
                AudioRecord.getMinBufferSize(
                    SAMPLE_RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                ),
                SAMPLES_PER_FRAME * 2,
            )

        val rec =
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_COMMUNICATION,
                SAMPLE_RATE,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                bufferSize,
            )

        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            Log.e(TAG, "AudioRecord failed to initialize")
            running = false
            return
        }

        recorder = rec
        // Set preferred device BEFORE startRecording() so routing is applied from the start
        if (device != null) {
            rec.setPreferredDevice(device)
            Log.i(TAG, "Audio capture preferred device set before recording: ${device.productName}")
        }
        rec.startRecording()
    }

    Log.i(TAG, "Audio capture started: ${SAMPLE_RATE}Hz mono, ${FRAME_SIZE_MS}ms frames")

    recordThread =
        Thread({
            val rec = synchronized(lock) { recorder } ?: return@Thread
            val buffer = ByteBuffer.allocateDirect(SAMPLES_PER_FRAME * 2)
            buffer.order(ByteOrder.nativeOrder())
            val shortBuffer = buffer.asShortBuffer()
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_URGENT_AUDIO)
            val tempArray = ShortArray(SAMPLES_PER_FRAME)

            while (running) {
                val read = rec.read(tempArray, 0, SAMPLES_PER_FRAME)
                if (read > 0) {
                    buffer.clear()
                    shortBuffer.clear()
                    shortBuffer.put(tempArray, 0, read)
                    buffer.position(0)
                    buffer.limit(read * 2)
                    NativeVideo.nativePushAudioFrame(buffer, read, SAMPLE_RATE, CHANNELS)
                }
            }

            Log.i(TAG, "Audio capture stopped")
        }, "AudioCapture").also { it.start() }
}
```

**Step 3: VisioManager — detect Bluetooth at startup + restart on device change**

Modify `startAudioPlayout()` (line 334-346) to detect and pass Bluetooth device:

```kotlin
fun startAudioPlayout() {
    if (audioPlayout != null) return
    val am = appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    am.mode = AudioManager.MODE_IN_COMMUNICATION
    val pm = appContext.getSystemService(Context.POWER_SERVICE) as PowerManager
    wakeLock =
        pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "VisioMobile::AudioPlayout").apply {
            acquire(4 * 60 * 60 * 1000L)
        }
    // Detect Bluetooth output device at startup
    val btOutput = am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).firstOrNull { device ->
        device.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO ||
        device.type == AudioDeviceInfo.TYPE_BLUETOOTH_A2DP ||
        device.type == AudioDeviceInfo.TYPE_BLE_HEADSET
    }
    if (btOutput != null) {
        Log.i("VisioManager", "Bluetooth output detected at startup: ${btOutput.productName}")
    }
    audioPlayout = AudioPlayout().also { it.start(btOutput) }
}
```

Add a new `startAudioCapture()` method that also detects Bluetooth input:

```kotlin
fun startAudioCapture() {
    if (audioCapture != null) return
    val am = appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    val btInput = am.getDevices(AudioManager.GET_DEVICES_INPUTS).firstOrNull { device ->
        device.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO ||
        device.type == AudioDeviceInfo.TYPE_BLE_HEADSET
    }
    if (btInput != null) {
        Log.i("VisioManager", "Bluetooth input detected at startup: ${btInput.productName}")
    }
    audioCapture = AudioCapture().also { it.start(btInput) }
}
```

Modify `setAudioOutputDevice()` (line 396-402) to restart the AudioTrack:

```kotlin
fun setAudioOutputDevice(device: AudioDeviceInfo) {
    // Restart AudioTrack with new device to ensure routing takes effect
    audioPlayout?.stop()
    audioPlayout = AudioPlayout().also { it.start(device) }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        val am = appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        am.setCommunicationDevice(device)
    }
}
```

Modify `setAudioInputDevice()` (line 385-391) to restart the AudioRecord:

```kotlin
fun setAudioInputDevice(device: AudioDeviceInfo) {
    // Restart AudioRecord with new device to ensure routing takes effect
    audioCapture?.stop()
    audioCapture = AudioCapture().also { it.start(device) }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        val am = appContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        am.setCommunicationDevice(device)
    }
}
```

**Step 4: Build**

Run: `cd android && ./gradlew compileDebugKotlin`
Expected: Compiles.

**Step 5: Commit**

```bash
git add android/app/src/main/kotlin/io/visio/mobile/AudioPlayout.kt android/app/src/main/kotlin/io/visio/mobile/AudioCapture.kt android/app/src/main/kotlin/io/visio/mobile/VisioManager.kt
git commit -m "fix(android): set preferred audio device before play/startRecording"
```

---

### Task 5: Android screen share — diagnostic logging

**Files:**
- Modify: `crates/visio-core/src/room.rs:944-964` — add screen share logging
- Modify: `crates/visio-ffi/src/lib.rs:1746-1769` — add JNI logging for track lookup

**Step 1: Add logging in room.rs TrackSubscribed handler**

In `crates/visio-core/src/room.rs`, after line 946 (`p.screen_share_track_sid = Some(track_sid.clone())`), add:

```rust
tracing::info!(
    "screen share track subscribed: participant={psid}, track_sid={track_sid}"
);
```

After line 963 (`subscribed_tracks...insert(track_sid.clone(), video_track.clone())`), add:

```rust
tracing::info!(
    "video track stored in registry: track_sid={}, source={:?}",
    track_sid, source
);
```

**Step 2: Build and verify**

Run: `cargo build -p visio-core`
Expected: Compiles.

**Step 3: Commit**

```bash
git add crates/visio-core/src/room.rs crates/visio-ffi/src/lib.rs
git commit -m "debug(core): add screen share track subscription logging"
```

---

### Task 6: Android screen share — pending surface mechanism

**Files:**
- Modify: `crates/visio-ffi/src/lib.rs` — add pending surfaces HashMap
- Modify: `crates/visio-core/src/room.rs` — trigger pending surface on TrackSubscribed

**Step 1: Add pending surfaces storage in visio-ffi**

In `crates/visio-ffi/src/lib.rs`, add a static for pending surfaces (near the existing statics):

```rust
use std::sync::Mutex as StdMutex;

static PENDING_SURFACES: std::sync::LazyLock<StdMutex<HashMap<String, *mut std::ffi::c_void>>> =
    std::sync::LazyLock::new(|| StdMutex::new(HashMap::new()));
```

Note: The `*mut c_void` must be `Send` — wrap in a newtype if needed:

```rust
struct RawSurface(*mut std::ffi::c_void);
unsafe impl Send for RawSurface {}

static PENDING_SURFACES: std::sync::LazyLock<StdMutex<HashMap<String, RawSurface>>> =
    std::sync::LazyLock::new(|| StdMutex::new(HashMap::new()));
```

**Step 2: Modify attachSurface to store pending if track not found**

In `crates/visio-ffi/src/lib.rs`, in the `attachSurface` JNI function, replace the `None` branch (around line 1765-1768):

```rust
None => {
    visio_log(&format!("VISIO JNI: track {track_sid} not in registry yet, storing as pending surface"));
    if let Ok(mut pending) = PENDING_SURFACES.lock() {
        pending.insert(track_sid, RawSurface(window_handle.into_raw() as *mut std::ffi::c_void));
    }
}
```

**Step 3: Add a function to check and attach pending surfaces**

Add a new `#[cfg(target_os = "android")]` function:

```rust
pub fn try_attach_pending_surface(track_sid: &str, client: &VisioClient) {
    let surface = {
        let Ok(mut pending) = PENDING_SURFACES.lock() else { return };
        pending.remove(track_sid)
    };
    if let Some(raw_surface) = surface {
        let track = client.rt.block_on(client.room_manager.get_video_track(track_sid));
        if let Some(video_track) = track {
            visio_log(&format!("VISIO JNI: attaching pending surface for {track_sid}"));
            visio_video::start_track_renderer(
                track_sid.to_string(),
                video_track,
                raw_surface.0,
                Some(client.rt.handle().clone()),
            );
        }
    }
}
```

**Step 4: Call pending surface check on TrackSubscribed**

In the FFI event listener's `TrackSubscribed` handler, after forwarding the event, add:

```rust
#[cfg(target_os = "android")]
{
    // Check if a surface was attached before the track arrived
    try_attach_pending_surface(&info.sid, &client_ref);
}
```

**Step 5: Build**

Run: `cargo build -p visio-ffi --target aarch64-linux-android` (or just `cargo build -p visio-ffi` for syntax check)
Expected: Compiles (cross-compilation may require NDK setup).

Run: `cargo build -p visio-core -p visio-ffi`
Expected: Compiles on host.

**Step 6: Commit**

```bash
git add crates/visio-ffi/src/lib.rs crates/visio-core/src/room.rs
git commit -m "fix(android): add pending surface mechanism for screen share tracks"
```

---

### Task 7: Final build verification

**Step 1: Run all Rust tests**

Run: `cargo test -p visio-core`
Expected: All tests pass (existing + new bandwidth tests).

**Step 2: Build all crates**

Run: `cargo build -p visio-core -p visio-video -p visio-desktop`
Expected: Compiles.

**Step 3: Commit if any fixups needed**

Only if build errors required changes.

**Step 4: Push**

```bash
git push
```
