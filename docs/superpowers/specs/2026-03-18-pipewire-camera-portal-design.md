# PipeWire Camera Portal Migration — Design Spec

**Issue:** #30
**Date:** 2026-03-18
**Status:** Draft

## Goal

Replace direct V4L2 camera access on Linux with PipeWire Camera portal (`org.freedesktop.portal.Camera`) when available, falling back to V4L2 on systems without portal support. This enables proper Flatpak sandboxing with user consent dialogs and removes the need for `--device=all`.

## Architecture

### Module structure

```
crates/visio-desktop/src/
├── camera_linux.rs              # Façade: runtime detection, dispatch, public API
├── camera_linux_pipewire.rs     # PipeWire portal backend (ashpd + pipewire crate)
├── camera_linux_v4l2.rs         # V4L2 backend (existing code, moved as-is)
└── camera_linux_convert.rs      # Shared pixel format conversion functions
```

### Public API (unchanged)

The public API in `camera_linux.rs` remains identical:

```rust
pub struct VideoDeviceInfo {
    pub name: String,
    pub unique_id: String,
    pub is_default: bool,
}

pub fn list_cameras() -> Vec<VideoDeviceInfo>;

pub struct LinuxCameraCapture { ... }
impl LinuxCameraCapture {
    pub fn start(source: NativeVideoSource) -> Result<Self, String>;
    pub fn start_with_unique_id(uid: &str, source: NativeVideoSource) -> Result<Self, String>;
    pub fn stop(&mut self);
}
```

### lib.rs integration

The existing `lib.rs` already has `#[cfg(target_os = "linux")]` blocks for `camera_linux` (added in PR #29). These remain unchanged since the façade preserves the same types and API. The `mod camera_linux` declaration covers all sub-modules (v4l2, pipewire, convert) as they are private to the `camera_linux` façade.

Existing integration points (no changes needed):
- `lib.rs:20` — `mod camera_linux;`
- `lib.rs:74` — `camera_capture: Mutex<Option<camera_linux::LinuxCameraCapture>>`
- `lib.rs:628` — `LinuxCameraCapture::start(source)` in `toggle_camera`
- `lib.rs:1124-1125` — `list_video_input_devices()` calls `camera_linux::list_cameras()`
- `lib.rs:1238-1260` — `select_video_input()` calls `start_with_unique_id()`

### Runtime detection

```rust
fn pipewire_portal_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        // Must use spawn_blocking to avoid panicking inside tokio runtime
        // (block_on from within a runtime context panics)
        std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                ashpd::desktop::camera::Camera::new().await.is_ok()
            })
        })
        .join()
        .unwrap_or(false)
    })
}
```

Detection spawns a dedicated thread with its own single-threaded tokio runtime to avoid panicking when called from an async context. Result is cached in `OnceLock` — tested once per process lifetime.

### PipeWire initialization

`pipewire::init()` must be called exactly once before any PipeWire API usage. This is done in `pipewire_portal_available()` before the detection check, guarded by the `OnceLock` (so it runs once).

## PipeWire Camera portal pipeline

```
1. ashpd::desktop::camera::Camera::new()  → D-Bus connection to portal
2. camera.access_camera().await            → user consent dialog (cached by portal after first grant)
3. camera.open_pipe_wire_remote().await    → OwnedFd for PipeWire connection
4. pipewire::core::Core::connect_fd(fd)   → connect to PipeWire daemon
5. core.create_stream("visio-camera")      → video stream
6. stream.connect(SPA_DIRECTION_INPUT)     → start receiving frames
7. on_process callback:
   - Read SPA buffer (NV12, YUY2, MJPEG, or RGB)
   - Convert to I420 via camera_linux_convert
   - Apply background blur via visio_ffi::blur::BlurProcessor::process_i420()
   - source.capture_frame(&video_frame)    → feed to LiveKit
   - render_local_i420() every 3rd frame   → self-view at ~10fps (AtomicU64 counter)
```

### Thread model

The PipeWire stream runs on a dedicated `std::thread` with its own `pipewire::main_loop::MainLoop`. The thread:
- Creates the MainLoop, Context, Core, and Stream
- Runs `main_loop.run()` which blocks until quit
- On `stop()`: `main_loop.quit()` is called via a `pipewire::channel::Sender`, then the thread is joined

```rust
pub struct PipewareCameraCapture {
    quit_sender: pipewire::channel::Sender<()>,  // Signal main_loop to quit
    thread: Option<JoinHandle<()>>,               // Joined on stop/drop
}

impl Drop for PipewareCameraCapture {
    fn drop(&mut self) {
        let _ = self.quit_sender.send(());
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
```

The `on_process` callback accesses `NativeVideoSource` which is `Send + Sync` (Arc-based internally in LiveKit SDK), so no additional synchronization is needed.

### Consent dialog lifecycle

The XDG Camera portal caches consent per-app. Behavior:
- **First call**: `access_camera()` shows OS consent dialog, user grants/denies
- **Subsequent calls**: portal returns cached permission immediately (no dialog)
- **User revokes in system settings**: next `access_camera()` call re-prompts
- **Implementation**: `Camera` object is created per-session (in `start()`), not held in long-lived state. This ensures permission is re-checked each time the camera is toggled on.

### Camera enumeration

The PipeWire Camera portal does not enumerate individual devices — the OS presents its own device picker if multiple cameras are available. The PipeWire backend returns a single synthetic entry:

```rust
vec![VideoDeviceInfo {
    name: "Camera (PipeWire)".to_string(),
    unique_id: "pipewire:camera".to_string(),
    is_default: true,
}]
```

When `start_with_unique_id()` receives `"pipewire:camera"`, it uses the PipeWire backend. Any other `unique_id` (e.g., `"/dev/video0"`) is dispatched to V4L2. This means when the portal is available, the user sees "Camera (PipeWire)" in the device list. If they want V4L2 directly, they would need to run outside Flatpak.

## Pixel format conversion

Shared in `camera_linux_convert.rs`, used by both backends:

| Source format | Conversion | Notes |
|---|---|---|
| NV12 | NV12 → I420 (split UV planes) | New, ~15 lines. Most common PipeWire format |
| YUY2/YUYV | YUYV → I420 (direct, single pass) | Optimized: no intermediate RGB allocation |
| MJPEG | JPEG decode → RGB → I420 | Existing `decode_mjpeg` + `rgb_to_i420` |
| RGB24 | RGB → I420 | Existing `rgb_to_i420` |

The existing YUYV path (`yuyv_to_rgb` + `rgb_to_i420`) is replaced by a direct `yuyv_to_i420` single-pass conversion — Y is at even byte positions, U/V at alternating odd positions. This avoids an unnecessary intermediate RGB buffer on the per-frame hot path.

Background blur (`visio_ffi::blur::BlurProcessor::process_i420()`) is applied after I420 conversion and before `capture_frame()`, matching the macOS pipeline.

## Flatpak manifest changes

```yaml
finish-args:
  # Remove:
  # - --device=all
  # Add/keep:
  - --device=dri                                    # GPU only
  - --filesystem=xdg-run/pipewire-0                 # PipeWire socket (already present)
  - --talk-name=org.freedesktop.portal.Desktop      # Portal access (already present)
```

The Camera portal handles camera permissions through user consent — no `--device=all` needed.

## Dependencies

### Cargo (Linux only)

```toml
[target.'cfg(target_os = "linux")'.dependencies]
ashpd = "0.10"       # XDG portals — ashpd::desktop::camera::Camera
pipewire = "0.8"     # PipeWire client — Core, MainLoop, Stream, channel
v4l = "0.14"         # V4L2 fallback (existing)
```

### System (build time)

```bash
# Already in CI and GNOME SDK:
libpipewire-0.3-dev libspa-0.2-dev libclang-dev
```

`ashpd` is pure Rust (zbus/zvariant). `pipewire` crate needs `libpipewire-0.3-dev` which is already in the CI workflow and GNOME 47 SDK.

### Optional: feature flag

Consider a `pipewire-camera` cargo feature (enabled by default on Linux) that gates the `ashpd` + `pipewire` dependencies. Users building from source without PipeWire can disable it and get V4L2-only. Not required for v1.

## Error handling

| Situation | Behavior |
|---|---|
| Portal D-Bus absent (old system) | Silent V4L2 fallback, `info` log |
| Portal present but PipeWire daemon not running | V4L2 fallback, `warn` log |
| User denies consent dialog | Return `Err("camera access denied")` → UI shows message |
| PipeWire daemon crash during capture | `warn` log, stop stream, app continues without camera |
| No camera detected (neither portal nor V4L2) | Empty list returned, UI shows "no camera" |
| Unsupported SPA format | Skip frame + `warn` log, continue capture |

## Testing

- **Unit tests**: conversion functions (`nv12_to_i420`, `yuyv_to_i420`, `rgb_to_i420`) with synthetic pixel data, verify Y/U/V output values
- **Integration test**: `pipewire_portal_available()` returns a bool without crash on any platform (including macOS CI where it should return `false`)
- **Manual test checklist**:
  - [ ] Flatpak: consent dialog appears on first camera toggle
  - [ ] Flatpak: camera LED on, self-view visible after consent
  - [ ] Flatpak: second toggle skips consent dialog (cached)
  - [ ] Native Linux with PipeWire: camera works via portal
  - [ ] Native Linux without PipeWire: camera works via V4L2 fallback
  - [ ] macOS: no regression (camera_linux modules not compiled)
