# Live Subtitles (MVP) — Design

## Overview

On-device speech-to-text via Whisper, displayed as an overlay on the video grid.
Each participant transcribes the full audio mix (mic + remote playout) locally.
No text is sent over the network. Privacy by design.

## Scope

**In scope:**
- CC toggle button in control bar
- Whisper `base` model (quantized int8, ~75MB), downloaded on first use
- `whisper-rs` in visio-core (CPU only, all platforms)
- Audio mix (mic + playout) tapped and downsampled in Rust
- Subtitle overlay (semi-transparent, bottom of video grid, 2-3 lines)
- User-configurable source language (default: system language)
- Model download with progress UI and SHA256 verification

**Out of scope (future):**
- Speaker attribution
- Auto language detection
- Subtitle sharing via data channel
- Transcript persistence
- GPU acceleration (CoreML / NNAPI)
- Translation (NLLB — Horizon 2)

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   visio-core                     │
│                                                  │
│  Audio push (mic) ──┐                            │
│                     ├──► AudioMixer ──► RingBuffer ──► WhisperWorker
│  Audio pull (play) ─┘    (mono 16kHz)            │           │
│                                                  │    TranscriptSegment
│                                                  │           │
│                                  VisioEvent::SubtitleChanged(text)
└─────────────────────────────────────────────────┘
```

### Components

**AudioMixer** (`visio-core/src/subtitles/mixer.rs`)
- Taps the mic push path and playout pull path (both already pass through Rust FFI)
- Resamples from 48kHz to 16kHz mono (Whisper input format)
- Writes mixed PCM f32 samples into a lock-free ring buffer
- Zero-copy: operates on existing audio buffers, no extra allocations on the hot path

**WhisperWorker** (`visio-core/src/subtitles/worker.rs`)
- Dedicated thread (not on tokio runtime — whisper-rs is blocking/CPU-bound)
- Consumes ring buffer in ~5s chunks with ~1s overlap (avoids word boundary cuts)
- Calls `whisper-rs` with configured language
- Emits `VisioEvent::SubtitleChanged(text)` with the transcribed segment
- Handles partial results for streaming feel (intermediate text updates)

**ModelManager** (`visio-core/src/subtitles/model.rs`)
- Downloads `ggml-base-q5_1.bin` from configurable CDN URL
- Stores in platform app data dir (Android `filesDir`, iOS `Application Support`, Desktop `app_data_dir`)
- SHA256 verification after download
- Exposes state: `NotDownloaded | Downloading(progress) | Ready(path) | Error(msg)`
- Emits `VisioEvent::ModelDownloadProgress(f32)` during download

**SubtitleManager** (`visio-core/src/subtitles/mod.rs`)
- Orchestrates mixer + worker lifecycle
- `start(language: String)` — initializes mixer taps, spawns worker thread
- `stop()` — tears down worker, removes mixer taps
- `set_language(lang: String)` — updates Whisper language parameter

### New events

```rust
pub enum VisioEvent {
    // ... existing events ...
    SubtitleChanged(String),
    ModelDownloadProgress(f32),     // 0.0 to 1.0
    ModelDownloadComplete,
    ModelDownloadError(String),
}
```

### UniFFI API additions

```rust
impl VisioClient {
    fn is_subtitle_model_available(&self) -> bool;
    fn download_subtitle_model(&self);     // async, emits progress events
    fn start_subtitles(&self, language: String);
    fn stop_subtitles(&self);
    fn set_subtitle_language(&self, language: String);
    fn supported_subtitle_languages(&self) -> Vec<SubtitleLanguage>;
}

pub struct SubtitleLanguage {
    pub code: String,    // "fr", "en", "es", ...
    pub name: String,    // "Francais", "English", "Espanol", ...
}
```

## UX

### CC Button

- Position: control bar, next to chat button
- Icon: text "CC" in a rounded rectangle (universal closed-captions icon)
- States:
  - **Off** (default): muted icon color
  - **Active**: filled/highlighted background (matches mic-on style)
  - **Downloading**: shows a small circular progress indicator on the button
- First tap when model not downloaded: triggers download dialog
- Subsequent taps: toggle subtitles on/off

### Download Dialog

- Modal dialog (Android `AlertDialog` / iOS `.sheet`)
- Text: "Download speech recognition model (~75 MB)?"
- Progress bar during download
- Cancel button (aborts download)
- On completion: dialog dismisses, subtitles activate automatically

### Subtitle Overlay

- Semi-transparent black background (`#000000` at 60% opacity), rounded corners (12dp)
- Positioned at bottom of video grid, 16dp margin from edges
- Text: white, 16sp, system font, left-aligned
- Max 3 lines visible, newest text at bottom
- Text updates in real-time (streaming partial results)
- Fades out after ~4s of silence (animated opacity transition)
- Does not intercept touch events (tap-through to video grid)

### Language Setting

- Located in InCallSettingsSheet, new "Subtitles" section
- Dropdown/picker with top 15 languages: French, English, Spanish, German, Italian, Portuguese, Dutch, Polish, Russian, Chinese, Japanese, Korean, Arabic, Turkish, Hindi
- Default: matches system locale, fallback to English
- Changeable while subtitles are active (restarts Whisper with new language)

## Performance

| Metric | Target |
|--------|--------|
| Whisper `base` q5_1 on ARM NEON | ~0.3x realtime (3s audio in ~1s) |
| Perceived latency | 2-3s speech to display |
| CPU usage | <15% on mid-range device (Snapdragon 7 series / A14) |
| Memory | ~150MB (model loaded in RAM) |
| Chunk size | 5s with 1s overlap |
| Worker thread | dedicated, normal priority |

## Model Distribution

- File: `ggml-base-q5_1.bin`
- Size: ~75MB
- Source URL: configurable in app settings (default: project GitHub Releases or static CDN)
- Checksum: SHA256 verified after download
- Storage: app-private directory (not user-visible, not backed up)
- No auto-update for MVP — model version pinned

## Platform Notes

### Android
- `whisper-rs` compiles via cargo-ndk for `arm64-v8a` (NEON available)
- Model stored in `context.filesDir / "models" / "whisper-base-q5.bin"`
- Download via `OkHttp` or Rust `reqwest` (prefer Rust to keep logic in visio-core)

### iOS
- `whisper-rs` compiles for `aarch64-apple-ios` (NEON available)
- Model stored in `Application Support / models / whisper-base-q5.bin`
- Download via Rust `reqwest` (same as Android)

### Desktop (Tauri)
- `whisper-rs` compiles natively (x86_64 AVX2 or ARM NEON on Apple Silicon)
- Model stored in Tauri `app_data_dir / models / whisper-base-q5.bin`
- Download via Rust `reqwest`

## Dependencies

New Cargo dependencies for `visio-core`:
- `whisper-rs` — Rust bindings for whisper.cpp
- `ringbuf` — lock-free SPSC ring buffer (or similar)
- `reqwest` (already used?) — for model download with progress

No new native (Android/iOS) dependencies. All logic in Rust.
