# In-Call Settings Panel — Design

**Date:** 2026-03-04
**Scope:** Android only (iOS follow-up later)

## Problem

The Android app lacks in-call device configuration. Users cannot:
- Select audio input/output devices
- Switch between front/back cameras
- Configure notification sounds

The web version (meet.linagora.com) has a full settings panel with tabbed navigation for Micro, Camera, Notifications, etc.

## Design

### Access Point

Add a **gear icon button** (`ri_settings_3_line`) in the `ControlBar`, between the Chat button and the Hangup button. The existing chevron next to the mic toggle opens the same panel with the Micro tab pre-selected.

### UI Layout

Bottom Sheet (`ModalBottomSheet`) with:
- **Left sidebar**: vertical icon column (3 icons), highlighted icon = active tab
- **Right content area**: content for the selected tab

```
┌──────────────────────────────┐
│  ──── (drag handle)          │
│ ┌────┬──────────────────────┐│
│ │ 🎤 │ [Tab content]        ││
│ │ 📷 │                      ││
│ │ 🔔 │                      ││
│ │    │                      ││
│ └────┴──────────────────────┘│
└──────────────────────────────┘
```

### Tab 1: Micro (`ri_mic_line`)

**Entrée audio** (Audio Input)
- List audio input devices via `AudioManager.getDevices(GET_DEVICES_INPUTS)`
- Filter: `TYPE_BUILTIN_MIC`, `TYPE_BLUETOOTH_SCO`, `TYPE_USB_HEADSET`, `TYPE_WIRED_HEADSET`
- Radio button selection
- Apply via `AudioManager.setPreferredDevice()` on the `AudioRecord` instance

**Sortie audio** (Audio Output)
- List audio output devices via `AudioManager.getDevices(GET_DEVICES_OUTPUTS)`
- Filter: `TYPE_BUILTIN_SPEAKER`, `TYPE_BUILTIN_EARPIECE`, `TYPE_BLUETOOTH_A2DP`, `TYPE_BLUETOOTH_SCO`, `TYPE_WIRED_HEADSET`, `TYPE_WIRED_HEADPHONES`, `TYPE_USB_HEADSET`
- Radio button selection
- Apply via `AudioManager.setCommunicationDevice()` (Android 12+)

Replaces the existing `AudioDeviceSheet`.

### Tab 2: Caméra (`ri_video_on_line`)

**Sélection caméra** (Camera Selection)
- List: Front camera / Back camera
- Radio button selection
- Apply via new `CameraCapture.switchCamera(cameraId)` method
- Uses existing `findFrontCamera()` / `findBackCamera()` from `CameraCapture.kt`

### Tab 3: Notifications (`ri_notification_3_line`)

**Notifications sonores** (Sound Notifications)
- Toggle: Nouveau participant (new participant joined)
- Toggle: Main levée (hand raised)
- Toggle: Message reçu (chat message received)
- All default to `true`

Persisted via new Rust settings fields:
- `notification_participant_join: bool`
- `notification_hand_raised: bool`
- `notification_message_received: bool`

## Architecture

### New Files
- `android/.../ui/InCallSettingsSheet.kt` — main composable with sidebar + tab content

### Modified Files
- `CallScreen.kt` — add gear button, replace `AudioDeviceSheet` usage, wire up camera switch
- `CameraCapture.kt` — add `switchCamera(cameraId)` method
- `VisioManager.kt` — expose camera switch + notification settings
- `crates/visio-core/src/settings.rs` — add notification fields
- `crates/visio-ffi/src/lib.rs` — expose new settings via FFI
- `crates/visio-ffi/src/visio.udl` — add new settings to UDL
- `android/.../assets/i18n/fr.json` + `en.json` — add i18n keys

### i18n Keys Needed
```
settings.incall.micro: "Micro" / "Microphone"
settings.incall.camera: "Caméra" / "Camera"
settings.incall.notifications: "Notifications" / "Notifications"
settings.incall.audioInput: "Entrée audio" / "Audio input"
settings.incall.audioOutput: "Sortie audio" / "Audio output"
settings.incall.cameraSelect: "Sélectionner la caméra" / "Select camera"
settings.incall.cameraFront: "Caméra frontale" / "Front camera"
settings.incall.cameraBack: "Caméra arrière" / "Back camera"
settings.incall.notifParticipant: "Un nouveau participant" / "New participant"
settings.incall.notifHandRaised: "Une main levée" / "Hand raised"
settings.incall.notifMessage: "Un message reçu" / "Message received"
```
