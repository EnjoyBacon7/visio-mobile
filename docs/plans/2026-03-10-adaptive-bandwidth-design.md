# Adaptive Bandwidth & Android Fixes Design

**Goal:** Ensure audio quality is always preserved under degraded network conditions via progressive video degradation, fix Android audio device initialization race, and fix Android screen share display issue.

**Scope:** visio-core (Rust), Android (Kotlin). iOS and Desktop are not affected by the two bugs.

---

## 1. Adaptive Bandwidth Management

### Architecture

A `BandwidthController` component in `visio-core` listens to `ConnectionQualityChanged` events from LiveKit and applies progressive degradation/restoration of video tracks.

### Degradation Paliers

| ConnectionQuality | Action |
|---|---|
| Excellent | Full resolution, all video streams active |
| Good | Nominal state, no changes |
| Poor | (1) Lower received video resolution via `set_video_dimensions()` on remote tracks (request lower simulcast layer). (2) If still Poor after 3s, unsubscribe all videos except active speaker |
| Lost | Unsubscribe all videos, audio only |

### Restoration

- **Hystérésis of 5 seconds**: quality must remain stable for 5s before restoring a palier
- Restoration is progressive (inverse of degradation): Lost → re-enable active speaker video → re-enable all videos at low res → full resolution

### Rules

- Audio is **never** touched — always preserved
- Screen share is treated as a normal video track (same degradation logic)
- Uses LiveKit `RemoteTrackPublication::set_enabled(false)` to disable video (server stops sending data)
- Uses `set_video_dimensions()` to request lower simulcast layers

### Events

- Emit `VisioEvent::BandwidthModeChanged { mode: BandwidthMode }` to UI so it can display a degraded quality indicator
- `BandwidthMode` enum: `Full`, `ReducedVideo`, `AudioOnly`

---

## 2. Android Audio Device Initialization Fix

### Problem

`AudioTrack.play()` and `AudioRecord.startRecording()` are called **before** `setPreferredDevice()`. On Android, changing the preferred device on an already-started track may not take effect.

### Solution

1. `startAudioPlayout(device: AudioDeviceInfo?)` and `startAudioCapture(device: AudioDeviceInfo?)` accept an optional device parameter
2. In `AudioPlayout.start()` and `AudioCapture.start()`, call `setPreferredDevice(device)` **before** `play()` / `startRecording()`
3. In `CallScreen.kt` at join time: detect if a Bluetooth device is connected and pass it to `startAudioPlayout()` / `startAudioCapture()`
4. For dynamic device changes (connect/disconnect during call): `stop()` then `start(newDevice)` instead of just `setPreferredDevice()` on running tracks

### Files

- `AudioPlayout.kt`: Add `device` parameter to `start()`
- `AudioCapture.kt`: Add `device` parameter to `start()`
- `VisioManager.kt`: Pass detected device at startup, restart tracks on device change
- `CallScreen.kt`: Detect Bluetooth at join time

---

## 3. Android Screen Share Display Fix

### Problem

When a remote participant starts screen sharing, their camera video disappears on Android. The screen share tile appears but shows nothing. Toggling the camera off/on on the remote side forces a re-render that fixes it.

### Root Cause (probable)

When `buildDisplayItems()` adds a screen share tile, the new `VideoSurfaceView` is created by Compose but `attachSurface()` is either not called with the correct screen share track SID, or is called before the track is registered in `subscribed_tracks`.

### Solution

1. **Diagnostic logging**: Log in `room.rs` (screen share track subscribed), JNI `attachSurface` (track lookup result), and `frame_loop` (frames received for screen share tracks)
2. **Fix surface attachment**: Ensure `LaunchedEffect` in `ParticipantTile` re-executes with the correct `trackSid` when the tile switches from camera to screen share
3. **Pending surfaces mechanism**: When `attachSurface()` is called for a track not yet in the registry, store the pending request and execute it when `TrackSubscribed` arrives

### Files

- `visio-core/src/room.rs`: Add screen share logging
- `visio-ffi/src/lib.rs`: Add pending surface mechanism in JNI
- `visio-video/src/lib.rs`: Add frame logging for screen share tracks
- `android/CallScreen.kt`: Ensure correct trackSid in LaunchedEffect key
