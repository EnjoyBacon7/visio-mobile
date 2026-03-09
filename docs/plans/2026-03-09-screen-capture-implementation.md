# Desktop Screen Capture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Feed real screen frames into the already-published screen share track using the `xcap` crate.

**Architecture:** New `screen_capture.rs` module in visio-desktop: lists sources via `xcap::Monitor/Window`, captures frames at 15 fps via tokio timer, converts RGBA→I420, feeds into `NativeVideoSource`. Frontend gets a source picker modal.

**Tech Stack:** Rust, xcap crate, image crate (DynamicImage), LiveKit WebRTC (I420Buffer, VideoFrame), Tauri 2.x, React/TypeScript

---

### Task 1: Add xcap dependency

**Files:**
- Modify: `crates/visio-desktop/Cargo.toml`

**Step 1: Add xcap to dependencies**

```toml
xcap = "0.9"
```

Add after the `open = "5"` line in `[dependencies]`.

**Step 2: Verify it compiles**

Run: `cargo build -p visio-desktop`
Expected: Compiles (xcap pulls in `image` crate transitively)

**Step 3: Commit**

```bash
git add crates/visio-desktop/Cargo.toml
git commit -m "feat(desktop): add xcap dependency for screen capture"
```

---

### Task 2: Create screen_capture.rs module

**Files:**
- Create: `crates/visio-desktop/src/screen_capture.rs`
- Modify: `crates/visio-desktop/src/lib.rs` (add `mod screen_capture;`)

**Step 1: Create the module with source listing**

Create `crates/visio-desktop/src/screen_capture.rs`:

```rust
//! Cross-platform screen capture using the `xcap` crate.
//!
//! Lists available monitors and windows, captures frames at ~15 fps,
//! converts RGBA→I420, and feeds into a LiveKit NativeVideoSource.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use livekit::webrtc::prelude::*;
use livekit::webrtc::video_source::native::NativeVideoSource;
use serde::Serialize;
use tokio::task::JoinHandle;

/// A capturable screen source (monitor or window).
#[derive(Debug, Clone, Serialize)]
pub struct ScreenSource {
    pub id: String,
    pub name: String,
    pub source_type: String, // "monitor" or "window"
    pub width: u32,
    pub height: u32,
}

/// List all available screen sources (monitors + windows).
pub fn list_sources() -> Vec<ScreenSource> {
    let mut sources = Vec::new();

    if let Ok(monitors) = xcap::Monitor::all() {
        for (i, monitor) in monitors.iter().enumerate() {
            let name = monitor
                .name()
                .unwrap_or_else(|_| format!("Display {}", i + 1));
            let width = monitor.width().unwrap_or(0);
            let height = monitor.height().unwrap_or(0);
            let is_primary = monitor.is_primary().unwrap_or(false);
            let label = if is_primary {
                format!("{name} (primary)")
            } else {
                name
            };
            sources.push(ScreenSource {
                id: format!("monitor-{i}"),
                name: label,
                source_type: "monitor".into(),
                width,
                height,
            });
        }
    }

    if let Ok(windows) = xcap::Window::all() {
        for window in &windows {
            let title = match window.title() {
                Ok(t) if !t.is_empty() => t,
                _ => continue, // skip untitled windows
            };
            // Skip minimized windows
            if window.is_minimized().unwrap_or(false) {
                continue;
            }
            let width = window.width().unwrap_or(0);
            let height = window.height().unwrap_or(0);
            if width == 0 || height == 0 {
                continue;
            }
            let id = window.id();
            sources.push(ScreenSource {
                id: format!("window-{id}"),
                name: title,
                source_type: "window".into(),
                width,
                height,
            });
        }
    }

    sources
}

/// Manages a screen capture session.
///
/// Captures frames at ~15 fps from the selected source and feeds
/// them into a LiveKit NativeVideoSource as I420 frames.
pub struct ScreenCapture {
    stop_flag: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
}

impl ScreenCapture {
    /// Start capturing from the given source ID.
    ///
    /// `source_id` is a string like "monitor-0" or "window-12345".
    pub fn start(source_id: &str, video_source: NativeVideoSource) -> Result<Self, String> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag = stop_flag.clone();
        let sid = source_id.to_string();

        let task = tokio::spawn(async move {
            capture_loop(&sid, video_source, flag).await;
        });

        tracing::info!("screen capture started for source {source_id}");
        Ok(Self {
            stop_flag,
            task: Some(task),
        })
    }

    /// Stop the capture loop.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            task.abort();
        }
        tracing::info!("screen capture stopped");
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Convert an RGBA image to an I420 buffer.
///
/// Uses BT.601 full-range coefficients:
///   Y  =  0.299*R + 0.587*G + 0.114*B
///   U  = -0.169*R - 0.331*G + 0.500*B + 128
///   V  =  0.500*R - 0.419*G - 0.081*B + 128
fn rgba_to_i420(rgba: &[u8], width: u32, height: u32) -> I420Buffer {
    let w = width as usize;
    let h = height as usize;
    let mut buf = I420Buffer::new(width, height);

    let strides = buf.strides();
    let (y_dst, u_dst, v_dst) = buf.data_mut();

    // Y plane: full resolution
    for row in 0..h {
        for col in 0..w {
            let px = (row * w + col) * 4;
            let r = rgba[px] as f32;
            let g = rgba[px + 1] as f32;
            let b = rgba[px + 2] as f32;
            let y = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
            y_dst[row * strides.0 as usize + col] = y;
        }
    }

    // U and V planes: half resolution (2x2 subsampling)
    let chroma_h = h / 2;
    let chroma_w = w / 2;
    for row in 0..chroma_h {
        for col in 0..chroma_w {
            // Average 2x2 block
            let mut r_sum = 0u32;
            let mut g_sum = 0u32;
            let mut b_sum = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let px = ((row * 2 + dy) * w + (col * 2 + dx)) * 4;
                    r_sum += rgba[px] as u32;
                    g_sum += rgba[px + 1] as u32;
                    b_sum += rgba[px + 2] as u32;
                }
            }
            let r = (r_sum / 4) as f32;
            let g = (g_sum / 4) as f32;
            let b = (b_sum / 4) as f32;

            let u = (-0.169 * r - 0.331 * g + 0.500 * b + 128.0).clamp(0.0, 255.0) as u8;
            let v = (0.500 * r - 0.419 * g - 0.081 * b + 128.0).clamp(0.0, 255.0) as u8;

            u_dst[row * strides.1 as usize + col] = u;
            v_dst[row * strides.2 as usize + col] = v;
        }
    }

    buf
}

/// The main capture loop. Runs on a tokio task.
async fn capture_loop(source_id: &str, video_source: NativeVideoSource, stop: Arc<AtomicBool>) {
    // Parse source ID
    let capturer: Box<dyn Fn() -> Result<image::DynamicImage, String> + Send> =
        if let Some(idx_str) = source_id.strip_prefix("monitor-") {
            let idx: usize = match idx_str.parse() {
                Ok(i) => i,
                Err(_) => {
                    tracing::error!("invalid monitor index: {idx_str}");
                    return;
                }
            };
            Box::new(move || {
                let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
                let monitor = monitors
                    .into_iter()
                    .nth(idx)
                    .ok_or_else(|| format!("monitor {idx} not found"))?;
                monitor.capture_image().map_err(|e| e.to_string())
            })
        } else if let Some(id_str) = source_id.strip_prefix("window-") {
            let win_id: u32 = match id_str.parse() {
                Ok(i) => i,
                Err(_) => {
                    tracing::error!("invalid window id: {id_str}");
                    return;
                }
            };
            Box::new(move || {
                let windows = xcap::Window::all().map_err(|e| e.to_string())?;
                let window = windows
                    .into_iter()
                    .find(|w| w.id() == win_id)
                    .ok_or_else(|| format!("window {win_id} not found"))?;
                window.capture_image().map_err(|e| e.to_string())
            })
        } else {
            tracing::error!("unknown source_id format: {source_id}");
            return;
        };

    let mut interval = tokio::time::interval(Duration::from_millis(67)); // ~15 fps

    loop {
        interval.tick().await;

        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Capture must run on a blocking thread (xcap does synchronous I/O)
        let cap = capturer.as_ref();
        // We need to call the capturer — but it's not Send across spawn_blocking.
        // Instead we do inline capture since xcap is fast enough for 15fps.
        match cap() {
            Ok(img) => {
                let rgba_img = img.to_rgba8();
                let width = rgba_img.width();
                let height = rgba_img.height();

                // Ensure even dimensions (I420 requires it)
                let width = width & !1;
                let height = height & !1;

                if width == 0 || height == 0 {
                    continue;
                }

                let i420 = rgba_to_i420(&rgba_img, width, height);

                let frame = VideoFrame {
                    rotation: VideoRotation::VideoRotation0,
                    timestamp_us: 0,
                    buffer: i420,
                };
                video_source.capture_frame(&frame);
            }
            Err(e) => {
                tracing::warn!("screen capture failed: {e}");
            }
        }
    }

    tracing::info!("screen capture loop ended for {source_id}");
}
```

**Step 2: Register the module**

In `crates/visio-desktop/src/lib.rs`, add after `mod audio_cpal;`:

```rust
mod screen_capture;
```

**Step 3: Verify it compiles**

Run: `cargo build -p visio-desktop`
Expected: Compiles

**Step 4: Commit**

```bash
git add crates/visio-desktop/src/screen_capture.rs crates/visio-desktop/src/lib.rs
git commit -m "feat(desktop): add screen_capture module with xcap-based capture loop"
```

---

### Task 3: Wire up Tauri commands

**Files:**
- Modify: `crates/visio-desktop/src/lib.rs`

**Step 1: Add screen_capture field to VisioState**

In the `VisioState` struct (~line 51), add:

```rust
screen_capture: std::sync::Mutex<Option<screen_capture::ScreenCapture>>,
```

And initialize it in the `setup` closure where `VisioState` is constructed — add:

```rust
screen_capture: std::sync::Mutex::new(None),
```

**Step 2: Add list_screen_sources command**

Add a new Tauri command:

```rust
#[tauri::command]
fn list_screen_sources() -> Vec<screen_capture::ScreenSource> {
    screen_capture::list_sources()
}
```

Register it in `generate_handler![...]` alongside `start_screen_share`.

**Step 3: Update start_screen_share to accept source_id and start capture**

Replace the existing `start_screen_share` command:

```rust
#[tauri::command]
async fn start_screen_share(
    state: tauri::State<'_, VisioState>,
    source_id: String,
) -> Result<(), String> {
    let controls = state.controls.lock().await;
    let source = controls
        .publish_screen_share()
        .await
        .map_err(|e| e.to_string())?;

    let capture = screen_capture::ScreenCapture::start(&source_id, source)
        .map_err(|e| format!("screen capture: {e}"))?;

    let mut cap = state.screen_capture.lock().unwrap_or_else(|e| e.into_inner());
    *cap = Some(capture);

    Ok(())
}
```

**Step 4: Update stop_screen_share to stop capture**

Replace the existing `stop_screen_share` command:

```rust
#[tauri::command]
async fn stop_screen_share(
    state: tauri::State<'_, VisioState>,
) -> Result<(), String> {
    // Stop capture first
    {
        let mut cap = state.screen_capture.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut capture) = cap.take() {
            capture.stop();
        }
    }

    let controls = state.controls.lock().await;
    controls
        .stop_screen_share()
        .await
        .map_err(|e| e.to_string())
}
```

**Step 5: Verify it compiles**

Run: `cargo build -p visio-desktop`
Expected: Compiles

**Step 6: Commit**

```bash
git add crates/visio-desktop/src/lib.rs
git commit -m "feat(desktop): wire screen capture to Tauri commands with source selection"
```

---

### Task 4: Frontend source picker modal

**Files:**
- Modify: `crates/visio-desktop/frontend/src/App.tsx`

**Step 1: Add ScreenSource interface**

Near the other type definitions at the top of App.tsx:

```typescript
interface ScreenSource {
  id: string;
  name: string;
  source_type: string; // "monitor" or "window"
  width: number;
  height: number;
}
```

**Step 2: Add state for source picker**

In the CallView component, near the `isScreenSharing` state:

```typescript
const [showSourcePicker, setShowSourcePicker] = useState(false);
const [screenSources, setScreenSources] = useState<ScreenSource[]>([]);
```

**Step 3: Update the screen share button**

Replace the current screen share button onClick handler. When not sharing, open the picker instead of directly starting:

```typescript
onClick={async () => {
  if (isScreenSharing) {
    try {
      await invoke("stop_screen_share");
      setIsScreenSharing(false);
    } catch (e) {
      console.error("Failed to stop screen share:", e);
    }
  } else {
    try {
      const sources = await invoke<ScreenSource[]>("list_screen_sources");
      setScreenSources(sources);
      setShowSourcePicker(true);
    } catch (e) {
      console.error("Failed to list screen sources:", e);
    }
  }
}}
```

**Step 4: Add SourcePickerModal component**

Add a new component before CallView:

```typescript
function SourcePickerModal({
  sources,
  onSelect,
  onClose,
}: {
  sources: ScreenSource[];
  onSelect: (sourceId: string) => void;
  onClose: () => void;
}) {
  const t = useT();
  const monitors = sources.filter(s => s.source_type === "monitor");
  const windows = sources.filter(s => s.source_type === "window");

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="settings-modal source-picker" onClick={e => e.stopPropagation()}>
        <div className="settings-header">
          <span>{t("call.selectSource")}</span>
          <button onClick={onClose}><RiCloseLine size={20} /></button>
        </div>
        <div className="settings-body" style={{ maxHeight: "400px", overflowY: "auto" }}>
          {monitors.length > 0 && (
            <>
              <h4 style={{ margin: "0 0 8px", fontSize: "0.85rem", color: "var(--text-secondary)" }}>
                {t("call.monitors")}
              </h4>
              {monitors.map(s => (
                <button
                  key={s.id}
                  className="source-item"
                  onClick={() => onSelect(s.id)}
                >
                  <ScreenShareIcon size={18} />
                  <span>{s.name}</span>
                  <span className="source-dim">{s.width}×{s.height}</span>
                </button>
              ))}
            </>
          )}
          {windows.length > 0 && (
            <>
              <h4 style={{ margin: "16px 0 8px", fontSize: "0.85rem", color: "var(--text-secondary)" }}>
                {t("call.windows")}
              </h4>
              {windows.map(s => (
                <button
                  key={s.id}
                  className="source-item"
                  onClick={() => onSelect(s.id)}
                >
                  <RiApps2Line size={18} />
                  <span>{s.name}</span>
                  <span className="source-dim">{s.width}×{s.height}</span>
                </button>
              ))}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
```

**Step 5: Render the modal and handle selection**

In the CallView render, after the existing modals, add:

```tsx
{showSourcePicker && (
  <SourcePickerModal
    sources={screenSources}
    onSelect={async (sourceId) => {
      setShowSourcePicker(false);
      try {
        await invoke("start_screen_share", { sourceId });
        setIsScreenSharing(true);
      } catch (e) {
        console.error("Failed to start screen share:", e);
      }
    }}
    onClose={() => setShowSourcePicker(false)}
  />
)}
```

**Step 6: Add CSS for source picker**

In the CSS file or inline styles, add:

```css
.source-picker {
  width: 420px;
}
.source-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  border: none;
  background: var(--bg-secondary);
  color: var(--text);
  border-radius: 8px;
  cursor: pointer;
  margin-bottom: 6px;
  font-size: 0.9rem;
  text-align: left;
}
.source-item:hover {
  background: var(--bg-tertiary);
}
.source-dim {
  margin-left: auto;
  font-size: 0.75rem;
  color: var(--text-secondary);
}
```

**Step 7: Add i18n keys**

In `i18n/en.json`: `"call.selectSource": "Share your screen"`, `"call.monitors": "Screens"`, `"call.windows": "Windows"`
In `i18n/fr.json`: `"call.selectSource": "Partager votre écran"`, `"call.monitors": "Écrans"`, `"call.windows": "Fenêtres"`
Add equivalents in de.json, es.json, it.json, nl.json.

**Step 8: Commit**

```bash
git add crates/visio-desktop/frontend/src/App.tsx i18n/
git commit -m "feat(desktop): add screen source picker modal"
```

---

### Task 5: Test and verify

**Step 1: Run Rust tests**

Run: `cargo test -p visio-core --lib && cargo test -p visio-desktop --lib`
Expected: All pass

**Step 2: Build Desktop app**

Run: `cargo build -p visio-desktop`
Expected: Compiles

**Step 3: Manual integration test**

1. `livekit-server --dev`
2. `cd crates/visio-desktop && cargo tauri dev`
3. Open LiveKit playground in browser, join same room
4. Desktop: click "Share screen" → source picker appears with monitors and windows
5. Select a monitor → screen is shared, browser sees the Desktop screen
6. Select "Stop sharing" → stops
7. Try sharing a window → browser sees only that window

**Step 4: Commit any fixes**

```bash
git commit -m "fix(desktop): screen capture integration fixes"
```
