# PipeWire Camera Portal Migration — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add PipeWire Camera portal support on Linux with V4L2 fallback, enabling proper Flatpak sandboxing without `--device=all`.

**Architecture:** Split `camera_linux.rs` into a façade + two backends (PipeWire, V4L2) + shared conversions. Runtime detection via `ashpd` determines which backend to use. Public API unchanged — `lib.rs` requires no modifications.

**Tech Stack:** `ashpd` 0.10 (XDG Camera portal), `pipewire` 0.8 (PipeWire stream), `v4l` 0.14 (fallback), Rust edition 2024.

**Spec:** `docs/superpowers/specs/2026-03-18-pipewire-camera-portal-design.md`

**Branch:** Create from `main` as `feat/pipewire-camera`

---

### Task 1: Extract shared conversion functions

Extract pixel format conversions from `camera_linux.rs` into a shared module. Add the new NV12→I420 and optimized YUYV→I420 conversions.

**Files:**
- Create: `crates/visio-desktop/src/camera_linux_convert.rs`
- Modify: `crates/visio-desktop/src/camera_linux.rs` (remove conversion functions, add `mod camera_linux_convert`)

- [ ] **Step 1: Write tests for existing conversions**

Create `crates/visio-desktop/src/camera_linux_convert.rs` with test module:

```rust
//! Shared pixel format conversion functions for Linux camera backends.

/// Decode MJPEG frame to RGB using the image crate.
pub fn decode_mjpeg(data: &[u8]) -> Result<Vec<u8>, String> {
    use image::io::Reader as ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| format!("Failed to guess format: {e}"))?;
    let img = reader
        .decode()
        .map_err(|e| format!("Failed to decode JPEG: {e}"))?;
    Ok(img.to_rgb8().into_raw())
}

/// Convert RGB24 to I420 (BT.601 full range).
pub fn rgb_to_i420(
    rgb: &[u8],
    width: usize,
    height: usize,
    y_dst: &mut [u8],
    y_stride: usize,
    u_dst: &mut [u8],
    u_stride: usize,
    v_dst: &mut [u8],
    v_stride: usize,
) {
    for row in 0..height {
        for col in 0..width {
            let rgb_idx = (row * width + col) * 3;
            let r = rgb[rgb_idx] as f32;
            let g = rgb[rgb_idx + 1] as f32;
            let b = rgb[rgb_idx + 2] as f32;

            let y = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
            y_dst[row * y_stride + col] = y;

            if row % 2 == 0 && col % 2 == 0 {
                let u = ((-0.169 * r - 0.331 * g + 0.5 * b) + 128.0).clamp(0.0, 255.0) as u8;
                let v = ((0.5 * r - 0.419 * g - 0.081 * b) + 128.0).clamp(0.0, 255.0) as u8;
                let cr = row / 2;
                let cc = col / 2;
                u_dst[cr * u_stride + cc] = u;
                v_dst[cr * v_stride + cc] = v;
            }
        }
    }
}

/// Convert YUYV (YUV 4:2:2) directly to I420 — single pass, no intermediate RGB.
pub fn yuyv_to_i420(
    data: &[u8],
    width: usize,
    height: usize,
    y_dst: &mut [u8],
    y_stride: usize,
    u_dst: &mut [u8],
    u_stride: usize,
    v_dst: &mut [u8],
    v_stride: usize,
) {
    for row in 0..height {
        for col in (0..width).step_by(2) {
            let base = (row * width + col) * 2;
            let y0 = data[base];
            let u = data[base + 1];
            let y1 = data[base + 2];
            let v = data[base + 3];

            y_dst[row * y_stride + col] = y0;
            y_dst[row * y_stride + col + 1] = y1;

            if row % 2 == 0 {
                let cr = row / 2;
                let cc = col / 2;
                u_dst[cr * u_stride + cc] = u;
                v_dst[cr * v_stride + cc] = v;
            }
        }
    }
}

/// Convert NV12 (Y plane + interleaved UV plane) to I420 (separate Y, U, V planes).
pub fn nv12_to_i420(
    nv12: &[u8],
    width: usize,
    height: usize,
    y_dst: &mut [u8],
    y_stride: usize,
    u_dst: &mut [u8],
    u_stride: usize,
    v_dst: &mut [u8],
    v_stride: usize,
) {
    let nv12_y_stride = width;
    let nv12_uv_offset = width * height;
    let nv12_uv_stride = width;

    // Copy Y plane
    for row in 0..height {
        let src_start = row * nv12_y_stride;
        let dst_start = row * y_stride;
        y_dst[dst_start..dst_start + width].copy_from_slice(&nv12[src_start..src_start + width]);
    }

    // Split interleaved UV into separate U and V planes
    let chroma_height = height / 2;
    let chroma_width = width / 2;
    for row in 0..chroma_height {
        let uv_row_start = nv12_uv_offset + row * nv12_uv_stride;
        for col in 0..chroma_width {
            u_dst[row * u_stride + col] = nv12[uv_row_start + col * 2];
            v_dst[row * v_stride + col] = nv12[uv_row_start + col * 2 + 1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_to_i420_pure_white() {
        let width = 4;
        let height = 4;
        let rgb = vec![255u8; width * height * 3]; // pure white
        let mut y = vec![0u8; width * height];
        let mut u = vec![0u8; (width / 2) * (height / 2)];
        let mut v = vec![0u8; (width / 2) * (height / 2)];
        rgb_to_i420(&rgb, width, height, &mut y, width, &mut u, width / 2, &mut v, width / 2);
        // White RGB → Y=255, U=128, V=128
        assert!(y.iter().all(|&val| val == 255));
        assert!(u.iter().all(|&val| (val as i16 - 128).abs() <= 1));
        assert!(v.iter().all(|&val| (val as i16 - 128).abs() <= 1));
    }

    #[test]
    fn test_rgb_to_i420_pure_black() {
        let width = 4;
        let height = 4;
        let rgb = vec![0u8; width * height * 3];
        let mut y = vec![255u8; width * height];
        let mut u = vec![255u8; (width / 2) * (height / 2)];
        let mut v = vec![255u8; (width / 2) * (height / 2)];
        rgb_to_i420(&rgb, width, height, &mut y, width, &mut u, width / 2, &mut v, width / 2);
        assert!(y.iter().all(|&val| val == 0));
        assert!(u.iter().all(|&val| val == 128));
        assert!(v.iter().all(|&val| val == 128));
    }

    #[test]
    fn test_yuyv_to_i420_roundtrip_luma() {
        let width = 4;
        let height = 2;
        // YUYV: Y0 U Y1 V pattern, set Y=100, U=128, V=128 (neutral chroma)
        let mut yuyv = vec![0u8; width * height * 2];
        for i in 0..(width * height / 2) {
            yuyv[i * 4] = 100;     // Y0
            yuyv[i * 4 + 1] = 128; // U
            yuyv[i * 4 + 2] = 100; // Y1
            yuyv[i * 4 + 3] = 128; // V
        }
        let mut y = vec![0u8; width * height];
        let mut u = vec![0u8; (width / 2) * (height / 2)];
        let mut v = vec![0u8; (width / 2) * (height / 2)];
        yuyv_to_i420(&yuyv, width, height, &mut y, width, &mut u, width / 2, &mut v, width / 2);
        assert!(y.iter().all(|&val| val == 100));
        assert!(u.iter().all(|&val| val == 128));
        assert!(v.iter().all(|&val| val == 128));
    }

    #[test]
    fn test_nv12_to_i420_basic() {
        let width = 4;
        let height = 4;
        let y_size = width * height;
        let uv_size = width * (height / 2); // interleaved UV
        let mut nv12 = vec![0u8; y_size + uv_size];
        // Fill Y with 200
        for i in 0..y_size {
            nv12[i] = 200;
        }
        // Fill UV interleaved: U=50, V=180
        for i in 0..(width / 2 * height / 2) {
            nv12[y_size + i * 2] = 50;
            nv12[y_size + i * 2 + 1] = 180;
        }
        let mut y_dst = vec![0u8; width * height];
        let mut u_dst = vec![0u8; (width / 2) * (height / 2)];
        let mut v_dst = vec![0u8; (width / 2) * (height / 2)];
        nv12_to_i420(&nv12, width, height, &mut y_dst, width, &mut u_dst, width / 2, &mut v_dst, width / 2);
        assert!(y_dst.iter().all(|&val| val == 200));
        assert!(u_dst.iter().all(|&val| val == 50));
        assert!(v_dst.iter().all(|&val| val == 180));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p visio-desktop camera_linux_convert --lib` (on a Linux machine or with `#[cfg(test)]` available)
Expected: 4 tests PASS

- [ ] **Step 3: Update camera_linux.rs to use shared conversions**

In `camera_linux.rs`:
- Add `mod camera_linux_convert;` at the top (after existing `mod` declarations)
- Remove `decode_mjpeg`, `yuyv_to_rgb`, `rgb_to_i420` functions
- Replace calls in `capture_loop`:
  - `decode_mjpeg(buf, width, height)` → `camera_linux_convert::decode_mjpeg(buf)`
  - `yuyv_to_rgb(buf, ...)` + `rgb_to_i420(...)` → `camera_linux_convert::yuyv_to_i420(...)` (direct, single pass into I420Buffer)
  - `rgb_to_i420(...)` → `camera_linux_convert::rgb_to_i420(...)`

- [ ] **Step 4: Run tests again, verify no regression**

Run: `cargo test -p visio-desktop --lib`
Expected: All tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/visio-desktop/src/camera_linux_convert.rs crates/visio-desktop/src/camera_linux.rs
git commit -m "refactor(linux): extract shared pixel format conversions, add NV12 + optimized YUYV"
```

---

### Task 2: Move V4L2 code to dedicated module

Move V4L2-specific code from `camera_linux.rs` to `camera_linux_v4l2.rs`. The façade keeps only the public API.

**Files:**
- Create: `crates/visio-desktop/src/camera_linux_v4l2.rs`
- Modify: `crates/visio-desktop/src/camera_linux.rs`

- [ ] **Step 1: Create camera_linux_v4l2.rs**

Move from `camera_linux.rs` to `camera_linux_v4l2.rs`:
- `list_cameras()` → `pub fn list_cameras() -> Vec<super::VideoDeviceInfo>`
- `LinuxCameraCapture` struct and impl → `pub struct V4lCameraCapture` (same fields: `running`, `_capture_thread`)
- `capture_loop()` function (uses `camera_linux_convert` for conversions)
- All `v4l` imports

The struct is renamed to `V4lCameraCapture` to distinguish from the façade's `LinuxCameraCapture`.

- [ ] **Step 2: Rewrite camera_linux.rs as façade**

```rust
//! Linux camera capture — façade with runtime backend selection.
//!
//! Uses PipeWire Camera portal when available (Flatpak, modern desktops),
//! falls back to V4L2 direct access otherwise.

mod camera_linux_convert;
mod camera_linux_v4l2;

use livekit::webrtc::video_source::native::NativeVideoSource;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct VideoDeviceInfo {
    pub name: String,
    pub unique_id: String,
    pub is_default: bool,
}

pub fn list_cameras() -> Vec<VideoDeviceInfo> {
    camera_linux_v4l2::list_cameras()
}

pub struct LinuxCameraCapture {
    inner: CaptureBackend,
}

enum CaptureBackend {
    V4l(camera_linux_v4l2::V4lCameraCapture),
}

impl LinuxCameraCapture {
    pub fn start(source: NativeVideoSource) -> Result<Self, String> {
        let inner = CaptureBackend::V4l(camera_linux_v4l2::V4lCameraCapture::start(0, source)?);
        Ok(Self { inner })
    }

    pub fn start_with_unique_id(unique_id: &str, source: NativeVideoSource) -> Result<Self, String> {
        let idx = if unique_id.starts_with("/dev/video") {
            unique_id.trim_start_matches("/dev/video").parse::<usize>().unwrap_or(0)
        } else {
            0
        };
        let inner = CaptureBackend::V4l(camera_linux_v4l2::V4lCameraCapture::start(idx, source)?);
        Ok(Self { inner })
    }

    pub fn stop(&mut self) {
        match &mut self.inner {
            CaptureBackend::V4l(cap) => cap.stop(),
        }
    }
}

impl Drop for LinuxCameraCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
```

- [ ] **Step 3: Verify compilation and tests**

Run: `cargo check -p visio-desktop` (or `cargo test -p visio-desktop --lib` on Linux)
Expected: compiles clean, all tests PASS

- [ ] **Step 4: Commit**

```bash
git add crates/visio-desktop/src/camera_linux.rs crates/visio-desktop/src/camera_linux_v4l2.rs
git commit -m "refactor(linux): split V4L2 backend into camera_linux_v4l2 module"
```

---

### Task 3: Add dependencies and PipeWire runtime detection

Add `ashpd` and `pipewire` crate dependencies and implement runtime portal detection.

**Files:**
- Modify: `crates/visio-desktop/Cargo.toml`
- Modify: `crates/visio-desktop/src/camera_linux.rs`

- [ ] **Step 1: Add dependencies to Cargo.toml**

In `crates/visio-desktop/Cargo.toml`, add to the `[target.'cfg(target_os = "linux")'.dependencies]` section:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
libpulse-binding = "2"
libpulse-simple-binding = "2"
v4l = "0.14"
ashpd = "0.10"
pipewire = "0.8"
```

- [ ] **Step 2: Add runtime detection to camera_linux.rs**

Add at the top of `camera_linux.rs`:

```rust
use std::sync::OnceLock;

fn pipewire_portal_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        pipewire::init();
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

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p visio-desktop` (on Linux with `libpipewire-0.3-dev` installed)
Expected: compiles with warnings (unused function `pipewire_portal_available`)

- [ ] **Step 4: Commit**

```bash
git add crates/visio-desktop/Cargo.toml crates/visio-desktop/src/camera_linux.rs
git commit -m "feat(linux): add ashpd + pipewire deps, implement portal runtime detection"
```

---

### Task 4: Implement PipeWire camera backend

Core implementation: portal consent, PipeWire stream, frame capture loop.

**Files:**
- Create: `crates/visio-desktop/src/camera_linux_pipewire.rs`

- [ ] **Step 1: Create camera_linux_pipewire.rs with struct and list_cameras**

```rust
//! PipeWire Camera portal backend.
//!
//! Uses ashpd to request camera access via XDG portal (consent dialog),
//! then opens a PipeWire stream to receive video frames.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use livekit::webrtc::prelude::*;
use livekit::webrtc::video_source::native::NativeVideoSource;

use super::VideoDeviceInfo;

pub fn list_cameras() -> Vec<VideoDeviceInfo> {
    vec![VideoDeviceInfo {
        name: "Camera (PipeWire)".to_string(),
        unique_id: "pipewire:camera".to_string(),
        is_default: true,
    }]
}

pub struct PipewireCameraCapture {
    quit_sender: pipewire::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl PipewireCameraCapture {
    pub fn start(source: NativeVideoSource) -> Result<Self, String> {
        // Request camera access via portal (consent dialog)
        let pw_fd = std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("tokio runtime: {e}"))?;
            rt.block_on(async {
                let camera = ashpd::desktop::camera::Camera::new()
                    .await
                    .map_err(|e| format!("camera portal: {e}"))?;
                camera
                    .access_camera()
                    .await
                    .map_err(|e| format!("camera access denied: {e}"))?;
                let fd = camera
                    .open_pipe_wire_remote()
                    .await
                    .map_err(|e| format!("pipewire remote: {e}"))?;
                Ok::<_, String>(fd)
            })
        })
        .join()
        .map_err(|_| "portal thread panicked".to_string())??;

        // Create PipeWire main loop on dedicated thread
        let (quit_sender, quit_receiver) = pipewire::channel::channel::<()>();

        let thread = thread::Builder::new()
            .name("visio-camera-pipewire".into())
            .spawn(move || {
                if let Err(e) = pipewire_capture_loop(pw_fd, source, quit_receiver) {
                    tracing::error!("PipeWire camera capture error: {e}");
                }
            })
            .map_err(|e| format!("Failed to spawn PipeWire thread: {e}"))?;

        tracing::info!("PipeWire camera capture started");

        Ok(Self {
            quit_sender,
            thread: Some(thread),
        })
    }

    pub fn stop(&mut self) {
        let _ = self.quit_sender.send(());
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        tracing::info!("PipeWire camera capture stopped");
    }
}

impl Drop for PipewireCameraCapture {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}
```

- [ ] **Step 2: Implement the PipeWire capture loop**

Add `pipewire_capture_loop` function below the struct. This is the core frame processing:

```rust
fn pipewire_capture_loop(
    pw_fd: std::os::fd::OwnedFd,
    source: NativeVideoSource,
    quit_receiver: pipewire::channel::Receiver<()>,
) -> Result<(), String> {
    let main_loop = pipewire::main_loop::MainLoop::new(None)
        .map_err(|e| format!("MainLoop::new: {e}"))?;
    let context = pipewire::context::Context::new(&main_loop)
        .map_err(|e| format!("Context::new: {e}"))?;

    use std::os::fd::AsRawFd;
    let core = context
        .connect_fd(pw_fd.as_raw_fd(), None)
        .map_err(|e| format!("connect_fd: {e}"))?;

    let stream = pipewire::stream::Stream::new(
        &core,
        "visio-camera",
        pipewire::properties::properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Camera",
        },
    )
    .map_err(|e| format!("Stream::new: {e}"))?;

    let frame_count = Arc::new(AtomicU64::new(0));
    let frame_count_cb = frame_count.clone();
    let source_cb = source.clone();
    let running = Arc::new(AtomicBool::new(true));
    let running_cb = running.clone();

    // Register quit signal
    let main_loop_weak = main_loop.loop_().downgrade();
    let _quit_listener = quit_receiver.attach(&main_loop.loop_(), move |_| {
        if let Some(main_loop) = main_loop_weak.upgrade() {
            main_loop.quit();
        }
    });

    // Process callback — called for each video frame
    let _listener = stream
        .add_local_listener()
        .process(move |stream, _| {
            if !running_cb.load(Ordering::Relaxed) {
                return;
            }
            if let Some(mut buffer) = stream.dequeue_buffer() {
                if let Some(buf) = buffer.datas_mut().first_mut() {
                    if let Some(data) = buf.data() {
                        // TODO: extract width/height/format from SPA format negotiation
                        // For now, this is a skeleton — actual format parsing added in step 3
                        let count = frame_count_cb.fetch_add(1, Ordering::Relaxed);
                        if count == 0 {
                            tracing::info!("First PipeWire camera frame received ({} bytes)", data.len());
                        }
                    }
                }
            }
        })
        .register()
        .map_err(|e| format!("stream listener: {e}"))?;

    // Connect stream — request video format
    let params = []; // Empty = accept any format
    stream
        .connect(
            pipewire::spa::utils::Direction::Input,
            None,
            pipewire::stream::StreamFlags::AUTOCONNECT | pipewire::stream::StreamFlags::MAP_BUFFERS,
            &mut params.iter().map(|p| *p),
        )
        .map_err(|e| format!("stream connect: {e}"))?;

    tracing::info!("PipeWire stream connected, entering main loop");
    main_loop.run();
    tracing::info!("PipeWire main loop exited");

    Ok(())
}
```

Note: the process callback is intentionally a skeleton in this step. Frame conversion is wired in step 3.

- [ ] **Step 3: Add frame format parsing and I420 conversion in the process callback**

Replace the `// TODO` section in the process callback with actual frame processing. The SPA buffer metadata contains the video format. Parse the format from the stream's negotiated params, then convert to I420:

```rust
// Inside the process callback, replace the TODO block:
let chunk = buf.chunk();
let width = /* from negotiated format */ ;
let height = /* from negotiated format */ ;
let format = /* from negotiated format — SPA_VIDEO_FORMAT_* */ ;

let mut i420 = I420Buffer::new(width, height);
let strides = i420.strides();
let (y_data, u_data, v_data) = i420.data_mut();

let converted = match format {
    // NV12 — most common PipeWire camera format
    SPA_VIDEO_FORMAT_NV12 => {
        super::camera_linux_convert::nv12_to_i420(
            data, width as usize, height as usize,
            y_data, strides.0 as usize,
            u_data, strides.1 as usize,
            v_data, strides.2 as usize,
        );
        true
    }
    // YUYV
    SPA_VIDEO_FORMAT_YUY2 => {
        super::camera_linux_convert::yuyv_to_i420(
            data, width as usize, height as usize,
            y_data, strides.0 as usize,
            u_data, strides.1 as usize,
            v_data, strides.2 as usize,
        );
        true
    }
    // MJPEG
    SPA_VIDEO_FORMAT_MJPG => {
        match super::camera_linux_convert::decode_mjpeg(data) {
            Ok(rgb) => {
                super::camera_linux_convert::rgb_to_i420(
                    &rgb, width as usize, height as usize,
                    y_data, strides.0 as usize,
                    u_data, strides.1 as usize,
                    v_data, strides.2 as usize,
                );
                true
            }
            Err(e) => { tracing::warn!("MJPEG decode: {e}"); false }
        }
    }
    _ => {
        tracing::warn!("Unsupported PipeWire format: {format:?}");
        false
    }
};

if converted {
    // Apply background blur
    {
        let strides = i420.strides();
        let (y_data, u_data, v_data) = i420.data_mut();
        visio_ffi::blur::BlurProcessor::process_i420(
            y_data, u_data, v_data,
            width as usize, height as usize,
            strides.0 as usize, strides.1 as usize, strides.2 as usize,
            0,
        );
    }

    let video_frame = VideoFrame {
        rotation: VideoRotation::VideoRotation0,
        timestamp_us: 0,
        buffer: i420,
    };
    source_cb.capture_frame(&video_frame);

    let count = frame_count_cb.fetch_add(1, Ordering::Relaxed);
    if count % 3 == 0 {
        visio_video::render_local_i420(&video_frame.buffer, "local-camera");
    }
}
```

Note: SPA format negotiation details depend on the `pipewire` crate's API. Consult `pipewire` 0.8 docs for `spa::param::video::VideoInfoRaw` to extract width/height/format from the stream's `param_changed` event.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p visio-desktop` (on Linux)
Expected: compiles clean

- [ ] **Step 5: Commit**

```bash
git add crates/visio-desktop/src/camera_linux_pipewire.rs
git commit -m "feat(linux): implement PipeWire Camera portal backend"
```

---

### Task 5: Wire PipeWire backend into the façade

Connect the PipeWire backend to the façade's runtime detection and dispatch.

**Files:**
- Modify: `crates/visio-desktop/src/camera_linux.rs`

- [ ] **Step 1: Add PipeWire module and backend variant**

In `camera_linux.rs`, add:

```rust
mod camera_linux_pipewire;
```

Update the `CaptureBackend` enum:

```rust
enum CaptureBackend {
    V4l(camera_linux_v4l2::V4lCameraCapture),
    Pipewire(camera_linux_pipewire::PipewireCameraCapture),
}
```

- [ ] **Step 2: Update list_cameras to dispatch based on portal availability**

```rust
pub fn list_cameras() -> Vec<VideoDeviceInfo> {
    if pipewire_portal_available() {
        camera_linux_pipewire::list_cameras()
    } else {
        camera_linux_v4l2::list_cameras()
    }
}
```

- [ ] **Step 3: Update start and start_with_unique_id**

```rust
impl LinuxCameraCapture {
    pub fn start(source: NativeVideoSource) -> Result<Self, String> {
        if pipewire_portal_available() {
            let cap = camera_linux_pipewire::PipewireCameraCapture::start(source)?;
            Ok(Self { inner: CaptureBackend::Pipewire(cap) })
        } else {
            let cap = camera_linux_v4l2::V4lCameraCapture::start(0, source)?;
            Ok(Self { inner: CaptureBackend::V4l(cap) })
        }
    }

    pub fn start_with_unique_id(unique_id: &str, source: NativeVideoSource) -> Result<Self, String> {
        if unique_id == "pipewire:camera" {
            let cap = camera_linux_pipewire::PipewireCameraCapture::start(source)?;
            Ok(Self { inner: CaptureBackend::Pipewire(cap) })
        } else {
            let idx = if unique_id.starts_with("/dev/video") {
                unique_id.trim_start_matches("/dev/video").parse::<usize>().unwrap_or(0)
            } else {
                0
            };
            let cap = camera_linux_v4l2::V4lCameraCapture::start(idx, source)?;
            Ok(Self { inner: CaptureBackend::V4l(cap) })
        }
    }

    pub fn stop(&mut self) {
        match &mut self.inner {
            CaptureBackend::V4l(cap) => cap.stop(),
            CaptureBackend::Pipewire(cap) => cap.stop(),
        }
    }
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p visio-desktop`
Expected: compiles clean, no warnings about unused functions

- [ ] **Step 5: Commit**

```bash
git add crates/visio-desktop/src/camera_linux.rs
git commit -m "feat(linux): wire PipeWire backend into camera façade with runtime dispatch"
```

---

### Task 6: Update Flatpak manifest

Remove `--device=all`, rely on Camera portal for camera access.

**Files:**
- Modify: `flatpak/io.visio.desktop.yml`

- [ ] **Step 1: Update finish-args**

In `flatpak/io.visio.desktop.yml`, replace:

```yaml
  # Camera
  - --device=all
```

with:

```yaml
  # Camera access via XDG Camera portal (consent dialog)
  # --device=all removed — portal handles camera permissions
```

Keep `--device=dri` for GPU acceleration (already present).

- [ ] **Step 2: Commit**

```bash
git add flatpak/io.visio.desktop.yml
git commit -m "feat(flatpak): remove --device=all, camera access via XDG portal"
```

---

### Task 7: Manual testing

No automated test possible for camera + portal. Follow this checklist on a Linux machine.

- [ ] **Step 1: Test on native Linux with PipeWire**

1. Run `visio-desktop` from terminal
2. Check logs: should see `pipewire_portal_available = true`
3. Toggle camera on → consent dialog should appear
4. After consent: camera LED on, self-view visible
5. Toggle camera off → LED off
6. Toggle camera on again → no dialog (cached)

- [ ] **Step 2: Test V4L2 fallback**

1. Set `XDG_CURRENT_DESKTOP=""` or run without D-Bus portal
2. Run `visio-desktop`
3. Check logs: should see `pipewire_portal_available = false`
4. Toggle camera → should work via V4L2 directly

- [ ] **Step 3: Test in Flatpak**

1. Build Flatpak: `flatpak/build-flatpak.sh`
2. Install: `flatpak install visio-mobile.flatpak`
3. Run: `flatpak run io.visio.desktop`
4. Toggle camera → consent dialog from OS
5. Verify camera works after consent

- [ ] **Step 4: Test macOS no-regression**

1. `cargo build -p visio-desktop` on macOS
2. Run the desktop app
3. Camera works as before (AVFoundation, no Linux modules compiled)
