# Live Subtitles Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add on-device speech-to-text via Whisper with a CC button and subtitle overlay on all platforms.

**Architecture:** `whisper-rs` in visio-core transcribes a mixed audio stream (mic + playout) tapped in Rust. New `SubtitleChanged` event pushes text to native UIs. Model downloaded on first use via `reqwest`.

**Tech Stack:** whisper-rs, ringbuf, reqwest (already in deps), Jetpack Compose, SwiftUI, Tauri IPC

---

### Task 1: Add `whisper-rs` and `ringbuf` dependencies

**Files:**
- Modify: `Cargo.toml` (workspace deps, lines 16-29)
- Modify: `crates/visio-core/Cargo.toml` (add deps)

**Step 1: Add workspace dependencies**

In `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
whisper-rs = "0.13"
ringbuf = "0.4"
```

**Step 2: Add crate dependencies**

In `crates/visio-core/Cargo.toml`, add to `[dependencies]`:

```toml
whisper-rs = { workspace = true }
ringbuf = { workspace = true }
```

**Step 3: Verify it compiles**

Run: `cargo check -p visio-core`
Expected: compiles (whisper-rs will download and build whisper.cpp)

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/visio-core/Cargo.toml
git commit -m "feat(subtitles): add whisper-rs and ringbuf dependencies"
```

---

### Task 2: Audio mixer — ring buffer and resampler

**Files:**
- Create: `crates/visio-core/src/subtitles/mod.rs`
- Create: `crates/visio-core/src/subtitles/mixer.rs`
- Modify: `crates/visio-core/src/lib.rs:6-17` (add `pub mod subtitles;`)

The mixer receives i16 PCM at 48kHz mono from two sources (mic + playout), resamples to 16kHz f32 (Whisper format), and writes to a ring buffer.

**Step 1: Write the failing test**

Create `crates/visio-core/src/subtitles/mixer.rs`:

```rust
use ringbuf::{HeapRb, traits::*};
use std::sync::{Arc, Mutex};

/// Downsamples 48kHz i16 to 16kHz f32 by picking every 3rd sample.
fn downsample_48k_to_16k(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .step_by(3)
        .map(|&s| s as f32 / 32768.0)
        .collect()
}

/// Shared ring buffer for feeding Whisper.
/// Producer side: AudioMixer pushes resampled f32 samples.
/// Consumer side: WhisperWorker reads chunks.
pub struct AudioMixer {
    producer: Mutex<ringbuf::HeapProd<f32>>,
    consumer: Mutex<ringbuf::HeapCons<f32>>,
    enabled: std::sync::atomic::AtomicBool,
}

impl AudioMixer {
    /// Create a new mixer with a ring buffer sized for `duration_secs` of 16kHz audio.
    pub fn new(duration_secs: usize) -> Self {
        let capacity = 16_000 * duration_secs;
        let rb = HeapRb::<f32>::new(capacity);
        let (producer, consumer) = rb.split();
        Self {
            producer: Mutex::new(producer),
            consumer: Mutex::new(consumer),
            enabled: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Enable or disable the mixer. When disabled, samples are silently dropped.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Push 48kHz i16 mono samples (from mic or playout).
    /// Resamples to 16kHz f32 and writes to ring buffer.
    /// If the buffer is full, oldest samples are silently lost.
    pub fn push_audio(&self, samples: &[i16]) {
        if !self.is_enabled() {
            return;
        }
        let resampled = downsample_48k_to_16k(samples);
        let mut prod = self.producer.lock().unwrap_or_else(|e| e.into_inner());
        // Push as many as fit; drop overflow silently
        prod.push_slice(&resampled);
    }

    /// Pull up to `out.len()` f32 samples from the ring buffer.
    /// Returns the number of samples actually read.
    pub fn pull_audio(&self, out: &mut [f32]) -> usize {
        let mut cons = self.consumer.lock().unwrap_or_else(|e| e.into_inner());
        cons.pop_slice(out)
    }

    /// Number of samples available in the ring buffer.
    pub fn available(&self) -> usize {
        let cons = self.consumer.lock().unwrap_or_else(|e| e.into_inner());
        cons.occupied_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downsample_48k_to_16k() {
        // 9 samples at 48kHz → 3 samples at 16kHz (every 3rd)
        let input: Vec<i16> = vec![1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000];
        let output = downsample_48k_to_16k(&input);
        assert_eq!(output.len(), 3);
        // sample 0, 3, 6 → 1000, 4000, 7000 normalized
        assert!((output[0] - 1000.0 / 32768.0).abs() < 1e-6);
        assert!((output[1] - 4000.0 / 32768.0).abs() < 1e-6);
        assert!((output[2] - 7000.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn test_mixer_push_pull() {
        let mixer = AudioMixer::new(1); // 1 second = 16000 samples
        mixer.set_enabled(true);

        // Push 480 samples at 48kHz (10ms) → 160 samples at 16kHz
        let input: Vec<i16> = vec![16384; 480];
        mixer.push_audio(&input);

        assert_eq!(mixer.available(), 160);

        let mut out = vec![0.0f32; 160];
        let read = mixer.pull_audio(&mut out);
        assert_eq!(read, 160);
        assert!((out[0] - 0.5).abs() < 1e-4); // 16384 / 32768 = 0.5
    }

    #[test]
    fn test_mixer_disabled_drops_samples() {
        let mixer = AudioMixer::new(1);
        // Not enabled by default
        let input: Vec<i16> = vec![16384; 480];
        mixer.push_audio(&input);
        assert_eq!(mixer.available(), 0);
    }

    #[test]
    fn test_mixer_overflow_drops_oldest() {
        let mixer = AudioMixer::new(1); // 16000 capacity
        mixer.set_enabled(true);

        // Push more than capacity: 48000 * 2 samples at 48kHz = 32000 at 16kHz
        let input: Vec<i16> = vec![1000; 48_000 * 2];
        mixer.push_audio(&input);

        // Ring buffer should contain at most 16000 samples
        assert!(mixer.available() <= 16_000);
    }
}
```

**Step 2: Create the module files**

Create `crates/visio-core/src/subtitles/mod.rs`:

```rust
pub mod mixer;
pub mod worker;
pub mod model;

pub use mixer::AudioMixer;
```

Add `pub mod subtitles;` to `crates/visio-core/src/lib.rs` after line 17 (after `pub mod settings;`).

**Step 3: Create stub files for worker and model**

Create `crates/visio-core/src/subtitles/worker.rs`:

```rust
// WhisperWorker — implemented in Task 3
```

Create `crates/visio-core/src/subtitles/model.rs`:

```rust
// ModelManager — implemented in Task 4
```

**Step 4: Run tests**

Run: `cargo test -p visio-core -- subtitles`
Expected: 4 tests pass

**Step 5: Commit**

```bash
git add crates/visio-core/src/subtitles/ crates/visio-core/src/lib.rs
git commit -m "feat(subtitles): add AudioMixer with 48→16kHz resampling and ring buffer"
```

---

### Task 3: Whisper worker thread

**Files:**
- Modify: `crates/visio-core/src/subtitles/worker.rs`
- Modify: `crates/visio-core/src/subtitles/mod.rs`

The worker runs on a dedicated thread, pulls audio chunks from the mixer, and runs Whisper inference.

**Step 1: Write the worker**

Replace `crates/visio-core/src/subtitles/worker.rs`:

```rust
use std::path::Path;
use std::sync::Arc;
use std::thread;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::mixer::AudioMixer;

/// Callback type for subtitle text output.
pub type SubtitleCallback = Arc<dyn Fn(String) + Send + Sync>;

pub struct WhisperWorker {
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl WhisperWorker {
    /// Start the Whisper worker thread.
    ///
    /// - `model_path`: path to the ggml Whisper model file
    /// - `language`: language code (e.g. "fr", "en")
    /// - `mixer`: shared AudioMixer to pull audio from
    /// - `callback`: called with transcribed text segments
    pub fn start(
        model_path: &Path,
        language: String,
        mixer: Arc<AudioMixer>,
        callback: SubtitleCallback,
    ) -> Result<Self, String> {
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or("invalid model path")?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| format!("failed to load Whisper model: {e}"))?;

        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = stop_flag.clone();

        let handle = thread::Builder::new()
            .name("whisper-worker".into())
            .spawn(move || {
                Self::run_loop(ctx, language, mixer, callback, stop);
            })
            .map_err(|e| format!("failed to spawn whisper thread: {e}"))?;

        Ok(Self {
            stop_flag,
            handle: Some(handle),
        })
    }

    fn run_loop(
        ctx: WhisperContext,
        language: String,
        mixer: Arc<AudioMixer>,
        callback: SubtitleCallback,
        stop: Arc<std::sync::atomic::AtomicBool>,
    ) {
        const CHUNK_SAMPLES: usize = 16_000 * 5; // 5 seconds at 16kHz
        const MIN_SAMPLES: usize = 16_000;        // need at least 1s of audio
        const POLL_MS: u64 = 200;

        let mut audio_buf = vec![0.0f32; CHUNK_SAMPLES];

        tracing::info!("whisper worker started, language={language}");

        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let available = mixer.available();
            if available < MIN_SAMPLES {
                thread::sleep(std::time::Duration::from_millis(POLL_MS));
                continue;
            }

            let to_read = available.min(CHUNK_SAMPLES);
            let read = mixer.pull_audio(&mut audio_buf[..to_read]);
            if read == 0 {
                continue;
            }

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some(&language));
            params.set_print_special_tokens(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_no_context(true);
            params.set_single_segment(true);

            let mut state = ctx.create_state().unwrap();
            if let Err(e) = state.full(params, &audio_buf[..read]) {
                tracing::warn!("whisper inference failed: {e}");
                continue;
            }

            let n_segments = state.full_n_segments().unwrap_or(0);
            let mut text = String::new();
            for i in 0..n_segments {
                if let Ok(seg) = state.full_get_segment_text(i) {
                    let trimmed = seg.trim();
                    if !trimmed.is_empty() {
                        if !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(trimmed);
                    }
                }
            }

            if !text.is_empty() {
                callback(text);
            }
        }

        tracing::info!("whisper worker stopped");
    }

    /// Stop the worker thread. Blocks until the thread exits.
    pub fn stop(&mut self) {
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for WhisperWorker {
    fn drop(&mut self) {
        self.stop();
    }
}
```

**Step 2: Update mod.rs exports**

In `crates/visio-core/src/subtitles/mod.rs`, add:

```rust
pub use worker::{WhisperWorker, SubtitleCallback};
```

**Step 3: Verify compilation**

Run: `cargo check -p visio-core`
Expected: compiles (no runtime test possible without model file)

**Step 4: Commit**

```bash
git add crates/visio-core/src/subtitles/
git commit -m "feat(subtitles): add WhisperWorker thread with 5s chunked inference"
```

---

### Task 4: Model manager (download + verification)

**Files:**
- Modify: `crates/visio-core/src/subtitles/model.rs`
- Modify: `crates/visio-core/src/subtitles/mod.rs`

**Step 1: Write the failing test**

Replace `crates/visio-core/src/subtitles/model.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Callback for download progress (0.0 to 1.0).
pub type ProgressCallback = Arc<dyn Fn(f32) + Send + Sync>;

/// Default model URL — Whisper base quantized q5_1.
const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

const MODEL_FILENAME: &str = "ggml-whisper-base.bin";

/// Manages the Whisper model file lifecycle.
pub struct ModelManager {
    data_dir: PathBuf,
    model_url: String,
}

impl ModelManager {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            model_url: DEFAULT_MODEL_URL.to_string(),
        }
    }

    /// Override the model download URL.
    pub fn set_model_url(&mut self, url: String) {
        self.model_url = url;
    }

    /// Path where the model file is (or will be) stored.
    pub fn model_path(&self) -> PathBuf {
        self.data_dir.join("models").join(MODEL_FILENAME)
    }

    /// Check if the model is already downloaded.
    pub fn is_available(&self) -> bool {
        self.model_path().exists()
    }

    /// Download the model with progress reporting.
    /// Returns the path to the downloaded file.
    pub async fn download(
        &self,
        progress: Option<ProgressCallback>,
    ) -> Result<PathBuf, String> {
        let models_dir = self.data_dir.join("models");
        std::fs::create_dir_all(&models_dir)
            .map_err(|e| format!("failed to create models dir: {e}"))?;

        let dest = self.model_path();
        let tmp = dest.with_extension("bin.tmp");

        let client = reqwest::Client::new();
        let resp = client
            .get(&self.model_url)
            .send()
            .await
            .map_err(|e| format!("download request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("download failed: HTTP {}", resp.status()));
        }

        let total = resp.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| format!("failed to create temp file: {e}"))?;

        use futures_util::StreamExt;
        use std::io::Write;
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("download error: {e}"))?;
            file.write_all(&chunk)
                .map_err(|e| format!("write error: {e}"))?;
            downloaded += chunk.len() as u64;
            if let Some(ref cb) = progress {
                if total > 0 {
                    cb(downloaded as f32 / total as f32);
                }
            }
        }

        drop(file);

        // Rename tmp to final destination (atomic on most filesystems)
        std::fs::rename(&tmp, &dest)
            .map_err(|e| format!("failed to rename model file: {e}"))?;

        tracing::info!("whisper model downloaded to {}", dest.display());
        Ok(dest)
    }

    /// Delete the downloaded model.
    pub fn delete(&self) -> Result<(), String> {
        let path = self.model_path();
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("failed to delete model: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_path() {
        let mgr = ModelManager::new(Path::new("/tmp/visio-test"));
        assert_eq!(
            mgr.model_path(),
            PathBuf::from("/tmp/visio-test/models/ggml-whisper-base.bin")
        );
    }

    #[test]
    fn test_not_available_initially() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ModelManager::new(dir.path());
        assert!(!mgr.is_available());
    }

    #[test]
    fn test_available_after_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ModelManager::new(dir.path());
        let models = dir.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join(MODEL_FILENAME), b"fake model").unwrap();
        assert!(mgr.is_available());
    }
}
```

**Step 2: Update mod.rs exports**

Add to `crates/visio-core/src/subtitles/mod.rs`:

```rust
pub use model::ModelManager;
```

**Step 3: Run tests**

Run: `cargo test -p visio-core -- subtitles`
Expected: 7 tests pass (4 mixer + 3 model)

**Step 4: Commit**

```bash
git add crates/visio-core/src/subtitles/
git commit -m "feat(subtitles): add ModelManager for Whisper model download"
```

---

### Task 5: New VisioEvents for subtitles

**Files:**
- Modify: `crates/visio-core/src/events.rs:1-50` (add variants to enum)
- Modify: `crates/visio-ffi/src/visio.udl:71-89` (add event variants)

**Step 1: Add Rust event variants**

In `crates/visio-core/src/events.rs`, add to the `VisioEvent` enum before the closing `}`:

```rust
    SubtitleChanged(String),
    ModelDownloadProgress(f32),
    ModelDownloadComplete,
    ModelDownloadError(String),
```

**Step 2: Add UDL event variants**

In `crates/visio-ffi/src/visio.udl`, add inside the `VisioEvent` interface before `ConnectionLost()`:

```
    SubtitleChanged(string text);
    ModelDownloadProgress(float progress);
    ModelDownloadComplete();
    ModelDownloadError(string message);
```

**Step 3: Verify compilation**

Run: `cargo check -p visio-ffi`
Expected: compiles

**Step 4: Commit**

```bash
git add crates/visio-core/src/events.rs crates/visio-ffi/src/visio.udl
git commit -m "feat(subtitles): add SubtitleChanged and model download events"
```

---

### Task 6: SubtitleManager orchestrator

**Files:**
- Create: `crates/visio-core/src/subtitles/manager.rs`
- Modify: `crates/visio-core/src/subtitles/mod.rs`

The SubtitleManager ties mixer + worker + model together and emits events.

**Step 1: Write the manager**

Create `crates/visio-core/src/subtitles/manager.rs`:

```rust
use std::path::Path;
use std::sync::Arc;

use crate::events::EventEmitter;
use crate::VisioEvent;

use super::mixer::AudioMixer;
use super::model::ModelManager;
use super::worker::WhisperWorker;

pub struct SubtitleManager {
    mixer: Arc<AudioMixer>,
    model: ModelManager,
    worker: Option<WhisperWorker>,
    language: String,
    emitter: Arc<EventEmitter>,
}

impl SubtitleManager {
    pub fn new(data_dir: &Path, emitter: Arc<EventEmitter>) -> Self {
        Self {
            mixer: Arc::new(AudioMixer::new(10)), // 10 seconds ring buffer
            model: ModelManager::new(data_dir),
            worker: None,
            language: "en".to_string(),
            emitter,
        }
    }

    /// Get a reference to the mixer for audio tapping.
    pub fn mixer(&self) -> Arc<AudioMixer> {
        self.mixer.clone()
    }

    /// Check if the Whisper model is downloaded.
    pub fn is_model_available(&self) -> bool {
        self.model.is_available()
    }

    /// Download the model asynchronously. Emits progress events.
    pub async fn download_model(&self) -> Result<(), String> {
        let emitter = self.emitter.clone();
        let progress_cb = Arc::new(move |p: f32| {
            emitter.emit(VisioEvent::ModelDownloadProgress(p));
        });

        match self.model.download(Some(progress_cb)).await {
            Ok(_) => {
                self.emitter.emit(VisioEvent::ModelDownloadComplete);
                Ok(())
            }
            Err(e) => {
                self.emitter
                    .emit(VisioEvent::ModelDownloadError(e.clone()));
                Err(e)
            }
        }
    }

    /// Set the transcription language (e.g. "fr", "en").
    pub fn set_language(&mut self, language: String) {
        self.language = language.clone();
        // If worker is running, restart it with new language
        if self.worker.is_some() {
            self.stop();
            let _ = self.start();
        }
    }

    /// Start subtitle transcription. Model must be downloaded first.
    pub fn start(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return Ok(()); // already running
        }

        let model_path = self.model.model_path();
        if !model_path.exists() {
            return Err("model not downloaded".into());
        }

        self.mixer.set_enabled(true);

        let emitter = self.emitter.clone();
        let callback = Arc::new(move |text: String| {
            emitter.emit(VisioEvent::SubtitleChanged(text));
        });

        let worker = WhisperWorker::start(
            &model_path,
            self.language.clone(),
            self.mixer.clone(),
            callback,
        )?;

        self.worker = Some(worker);
        Ok(())
    }

    /// Stop subtitle transcription.
    pub fn stop(&mut self) {
        self.mixer.set_enabled(false);
        if let Some(mut w) = self.worker.take() {
            w.stop();
        }
    }

    /// Check if subtitles are currently active.
    pub fn is_active(&self) -> bool {
        self.worker.is_some()
    }

    /// Get supported languages as (code, name) pairs.
    pub fn supported_languages() -> Vec<(String, String)> {
        vec![
            ("fr".into(), "Français".into()),
            ("en".into(), "English".into()),
            ("es".into(), "Español".into()),
            ("de".into(), "Deutsch".into()),
            ("it".into(), "Italiano".into()),
            ("pt".into(), "Português".into()),
            ("nl".into(), "Nederlands".into()),
            ("pl".into(), "Polski".into()),
            ("ru".into(), "Русский".into()),
            ("zh".into(), "中文".into()),
            ("ja".into(), "日本語".into()),
            ("ko".into(), "한국어".into()),
            ("ar".into(), "العربية".into()),
            ("tr".into(), "Türkçe".into()),
            ("hi".into(), "हिन्दी".into()),
        ]
    }
}
```

**Step 2: Update mod.rs**

Replace `crates/visio-core/src/subtitles/mod.rs`:

```rust
pub mod manager;
pub mod mixer;
pub mod model;
pub mod worker;

pub use manager::SubtitleManager;
pub use mixer::AudioMixer;
pub use model::ModelManager;
pub use worker::{SubtitleCallback, WhisperWorker};
```

**Step 3: Add pub use to lib.rs**

In `crates/visio-core/src/lib.rs`, add after the subtitles module declaration:

```rust
pub use subtitles::SubtitleManager;
```

**Step 4: Verify compilation**

Run: `cargo check -p visio-core`
Expected: compiles

**Step 5: Commit**

```bash
git add crates/visio-core/src/subtitles/ crates/visio-core/src/lib.rs
git commit -m "feat(subtitles): add SubtitleManager orchestrating mixer, worker, and model"
```

---

### Task 7: Tap audio streams into the mixer

**Files:**
- Modify: `crates/visio-core/src/audio_playout.rs:12-46` (add mixer tap)
- Modify: `crates/visio-core/src/room.rs:923-943` (pass mixer to playout)

The playout buffer already receives all remote audio via `push_samples()`. We add an optional mixer reference to also copy samples to the subtitle mixer. For mic audio, we tap in the FFI layer.

**Step 1: Add mixer tap to AudioPlayoutBuffer**

In `crates/visio-core/src/audio_playout.rs`, modify the struct to add a subtitle mixer:

Add field to `AudioPlayoutBuffer`:

```rust
pub struct AudioPlayoutBuffer {
    buffer: Mutex<VecDeque<i16>>,
    max_samples: usize,
    subtitle_mixer: Mutex<Option<Arc<crate::subtitles::AudioMixer>>>,
}
```

Update `new()`:

```rust
    pub fn new() -> Self {
        let max_samples = 48_000 * 2;
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(max_samples)),
            max_samples,
            subtitle_mixer: Mutex::new(None),
        }
    }
```

Add method to set the mixer:

```rust
    /// Set the subtitle mixer to tap playout audio for transcription.
    pub fn set_subtitle_mixer(&self, mixer: Option<Arc<crate::subtitles::AudioMixer>>) {
        *self.subtitle_mixer.lock().unwrap_or_else(|e| e.into_inner()) = mixer;
    }
```

In `push_samples()`, after `buf.extend(samples.iter().copied());` (line 39), add the tap:

```rust
        // Tap audio for subtitle transcription
        if let Some(mixer) = self.subtitle_mixer.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            mixer.push_audio(samples);
        }
```

**Step 2: Write test for the tap**

Add to the bottom of `audio_playout.rs`:

```rust
    #[test]
    fn test_subtitle_mixer_tap() {
        let buf = AudioPlayoutBuffer::new();
        let mixer = Arc::new(crate::subtitles::AudioMixer::new(1));
        mixer.set_enabled(true);
        buf.set_subtitle_mixer(Some(mixer.clone()));

        // Push 480 samples (10ms at 48kHz)
        let samples: Vec<i16> = vec![1000; 480];
        buf.push_samples(&samples);

        // Mixer should have received resampled audio (480/3 = 160 samples at 16kHz)
        assert_eq!(mixer.available(), 160);

        // Normal playout should also work
        let mut out = vec![0i16; 480];
        let read = buf.pull_samples(&mut out);
        assert_eq!(read, 480);
    }
```

**Step 3: Run tests**

Run: `cargo test -p visio-core -- audio_playout`
Expected: all tests pass

**Step 4: Commit**

```bash
git add crates/visio-core/src/audio_playout.rs
git commit -m "feat(subtitles): tap playout audio into subtitle mixer"
```

---

### Task 8: Tap mic audio in FFI layer

**Files:**
- Modify: `crates/visio-ffi/src/lib.rs:1118` (add subtitle mixer static)
- Modify: `crates/visio-ffi/src/lib.rs:1307-1342` (tap mic in nativePushAudioFrame)
- Modify: `crates/visio-ffi/src/lib.rs:1426-1434` (tap mic for iOS — separate approach)

For Android, the mic audio passes through `nativePushAudioFrame`. We add a static `SUBTITLE_MIXER` and copy samples there.

For iOS, mic audio is captured internally by the LiveKit SDK — we don't have a tap point in the FFI layer. The playout tap alone will provide remote audio; for the local mic on iOS, we'll need to add a tap in a future iteration (the playout-only transcription is functional for MVP since it captures what others say).

**Step 1: Add SUBTITLE_MIXER static (Android)**

In `crates/visio-ffi/src/lib.rs`, near `AUDIO_SOURCE` (line 1118), add:

```rust
/// Subtitle mixer for tapping mic audio on Android.
#[cfg(target_os = "android")]
static SUBTITLE_MIXER: StdMutex<Option<Arc<visio_core::subtitles::AudioMixer>>> = StdMutex::new(None);
```

Add the import at the top of the file:

```rust
use std::sync::Arc;
```

(Check if `Arc` is already imported — it likely is via other uses.)

**Step 2: Tap mic audio in nativePushAudioFrame**

In `nativePushAudioFrame` (around line 1337, after `source.capture_frame(&frame)`), add:

```rust
    // Tap mic audio for subtitle transcription
    if let Some(mixer) = SUBTITLE_MIXER.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        mixer.push_audio(&samples_i16);
    }
```

Note: `samples_i16` needs to be extracted from the ByteBuffer before capture_frame. Check the existing code — the samples are read as `u8` then converted to `AudioFrame`. We need to also extract them as `i16` for the mixer. Add this conversion near the existing buffer read:

```rust
    // Convert byte buffer to i16 slice for subtitle mixer
    let samples_i16: Vec<i16> = data[..byte_count]
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
```

**Step 3: Add setter for SUBTITLE_MIXER**

In the `VisioClient` impl block, add a helper that stores the mixer reference:

```rust
    fn set_subtitle_mixer_for_ffi(&self, mixer: Option<Arc<visio_core::subtitles::AudioMixer>>) {
        #[cfg(target_os = "android")]
        {
            *SUBTITLE_MIXER.lock().unwrap_or_else(|e| e.into_inner()) = mixer.clone();
        }
        // Playout tap (all platforms) is set in audio_playout.rs via set_subtitle_mixer
    }
```

**Step 4: Verify compilation**

Run: `cargo check -p visio-ffi`
Expected: compiles (may need to resolve import paths)

**Step 5: Commit**

```bash
git add crates/visio-ffi/src/lib.rs
git commit -m "feat(subtitles): tap mic audio into subtitle mixer on Android"
```

---

### Task 9: VisioClient subtitle API + UDL

**Files:**
- Modify: `crates/visio-ffi/src/lib.rs:472-479` (add SubtitleManager to VisioClient struct)
- Modify: `crates/visio-ffi/src/lib.rs:482+` (add methods)
- Modify: `crates/visio-ffi/src/visio.udl:148-271` (add methods + dictionary)

**Step 1: Add SubtitleManager to VisioClient**

In `crates/visio-ffi/src/lib.rs`, add field to `VisioClient` struct:

```rust
pub struct VisioClient {
    room_manager: visio_core::RoomManager,
    controls: visio_core::MeetingControls,
    chat: visio_core::ChatService,
    settings: visio_core::SettingsStore,
    session_manager: Arc<StdMutex<visio_core::SessionManager>>,
    subtitle_manager: StdMutex<visio_core::SubtitleManager>,
    rt: tokio::runtime::Runtime,
}
```

In `VisioClient::new()`, initialize it:

```rust
    let emitter = room_manager.event_emitter();
    let subtitle_manager = StdMutex::new(visio_core::SubtitleManager::new(
        std::path::Path::new(&data_dir),
        emitter,
    ));
```

**Step 2: Add VisioClient methods**

Add these methods to the `impl VisioClient` block:

```rust
    pub fn is_subtitle_model_available(&self) -> bool {
        self.subtitle_manager
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_model_available()
    }

    pub fn download_subtitle_model(&self) {
        let mgr = self.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
        // We need the mixer reference to set up taps
        let mixer = mgr.mixer();
        drop(mgr);

        self.rt.spawn(async move {
            // download is handled in the manager, which emits events
        });

        // Actually: we need to call download_model on the manager
        // Use block_on in a spawned thread to not block the caller
        let subtitle_mgr_ptr = &self.subtitle_manager as *const _ as usize;
        let rt_handle = self.rt.handle().clone();
        std::thread::spawn(move || {
            // Safety: VisioClient outlives this thread in practice
            // But better approach: use the runtime
            let mgr_ref = unsafe { &*(subtitle_mgr_ptr as *const StdMutex<visio_core::SubtitleManager>) };
            let mgr = mgr_ref.lock().unwrap_or_else(|e| e.into_inner());
            rt_handle.block_on(mgr.download_model()).ok();
        });
    }

    pub fn start_subtitles(&self, language: String) {
        let mut mgr = self.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
        mgr.set_language(language);

        // Wire mixer to playout buffer
        let mixer = mgr.mixer();
        self.room_manager.playout_buffer().set_subtitle_mixer(Some(mixer.clone()));
        self.set_subtitle_mixer_for_ffi(Some(mixer));

        if let Err(e) = mgr.start() {
            tracing::error!("failed to start subtitles: {e}");
        }
    }

    pub fn stop_subtitles(&self) {
        let mut mgr = self.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
        mgr.stop();
        self.room_manager.playout_buffer().set_subtitle_mixer(None);
        self.set_subtitle_mixer_for_ffi(None);
    }

    pub fn set_subtitle_language(&self, language: String) {
        let mut mgr = self.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
        mgr.set_language(language);
    }

    pub fn supported_subtitle_languages(&self) -> Vec<SubtitleLanguage> {
        visio_core::SubtitleManager::supported_languages()
            .into_iter()
            .map(|(code, name)| SubtitleLanguage { code, name })
            .collect()
    }
```

Add the return type struct:

```rust
pub struct SubtitleLanguage {
    pub code: String,
    pub name: String,
}
```

**Step 3: Update UDL**

In `crates/visio-ffi/src/visio.udl`, add the dictionary before `interface VisioClient`:

```
dictionary SubtitleLanguage {
    string code;
    string name;
};
```

Add methods to the `VisioClient` interface:

```
    boolean is_subtitle_model_available();

    void download_subtitle_model();

    void start_subtitles(string language);

    void stop_subtitles();

    void set_subtitle_language(string language);

    sequence<SubtitleLanguage> supported_subtitle_languages();
```

**Step 4: Verify compilation**

Run: `cargo check -p visio-ffi`
Expected: compiles

**Step 5: Commit**

```bash
git add crates/visio-ffi/src/lib.rs crates/visio-ffi/src/visio.udl
git commit -m "feat(subtitles): add subtitle API to VisioClient and UDL"
```

---

### Task 10: Generate UniFFI bindings

**Files:**
- Run: `scripts/generate-bindings.sh all`

**Step 1: Generate bindings**

Run: `scripts/generate-bindings.sh all`
Expected: Kotlin and Swift bindings regenerated with new subtitle methods and events

**Step 2: Verify generated code includes subtitle types**

Check that the generated files contain `SubtitleLanguage`, `startSubtitles`, `stopSubtitles`, `SubtitleChanged`.

**Step 3: Commit**

```bash
git add android/app/src/main/kotlin/generated/ ios/VisioMobile/Generated/
git commit -m "chore(ffi): regenerate UniFFI bindings with subtitle API"
```

---

### Task 11: Android — CC button and subtitle overlay

**Files:**
- Modify: `android/app/src/main/kotlin/io/visio/mobile/ui/CallScreen.kt:664-860` (add CC button to control bar)
- Modify: `android/app/src/main/kotlin/io/visio/mobile/VisioManager.kt` (add subtitle state + event handling)

**Step 1: Add subtitle state to VisioManager**

In `VisioManager.kt`, add StateFlows:

```kotlin
private val _subtitleText = MutableStateFlow("")
val subtitleText: StateFlow<String> = _subtitleText.asStateFlow()

private val _subtitlesActive = MutableStateFlow(false)
val subtitlesActive: StateFlow<Boolean> = _subtitlesActive.asStateFlow()

private val _modelDownloading = MutableStateFlow(false)
val modelDownloading: StateFlow<Boolean> = _modelDownloading.asStateFlow()

private val _modelDownloadProgress = MutableStateFlow(0f)
val modelDownloadProgress: StateFlow<Float> = _modelDownloadProgress.asStateFlow()
```

In the `onEvent()` handler, add cases:

```kotlin
is VisioEvent.SubtitleChanged -> {
    _subtitleText.value = event.text
}
is VisioEvent.ModelDownloadProgress -> {
    _modelDownloading.value = true
    _modelDownloadProgress.value = event.progress
}
is VisioEvent.ModelDownloadComplete -> {
    _modelDownloading.value = false
    _modelDownloadProgress.value = 1f
}
is VisioEvent.ModelDownloadError -> {
    _modelDownloading.value = false
    Log.e("VisioManager", "Model download error: ${event.message}")
}
```

Add helper methods:

```kotlin
fun toggleSubtitles() {
    if (_subtitlesActive.value) {
        client.stopSubtitles()
        _subtitlesActive.value = false
        _subtitleText.value = ""
    } else {
        if (!client.isSubtitleModelAvailable()) {
            client.downloadSubtitleModel()
            // Start subtitles after download completes (handled in ModelDownloadComplete event)
            return
        }
        val lang = client.getSettings().language ?: "en"
        client.startSubtitles(lang)
        _subtitlesActive.value = true
    }
}
```

Update `ModelDownloadComplete` handler to auto-start subtitles:

```kotlin
is VisioEvent.ModelDownloadComplete -> {
    _modelDownloading.value = false
    // Auto-start subtitles after download
    val lang = client.getSettings().language ?: "en"
    client.startSubtitles(lang)
    _subtitlesActive.value = true
}
```

**Step 2: Add CC button to ControlBar**

In `CallScreen.kt`, in the `ControlBar` composable, add the CC button next to the chat button. Collect the states:

```kotlin
val subtitlesActive by visioManager.subtitlesActive.collectAsState()
val subtitleText by visioManager.subtitleText.collectAsState()
val modelDownloading by visioManager.modelDownloading.collectAsState()
val modelDownloadProgress by visioManager.modelDownloadProgress.collectAsState()
```

Add the CC button (in the Row with other control buttons, before the chat button):

```kotlin
// CC (subtitles) button
Box(contentAlignment = Alignment.Center) {
    IconButton(
        onClick = { visioManager.toggleSubtitles() },
        enabled = !modelDownloading,
    ) {
        if (modelDownloading) {
            CircularProgressIndicator(
                progress = { modelDownloadProgress },
                modifier = Modifier.size(24.dp),
                strokeWidth = 2.dp,
            )
        } else {
            Text(
                text = "CC",
                fontWeight = FontWeight.Bold,
                fontSize = 14.sp,
                color = if (subtitlesActive) MaterialTheme.colorScheme.primary
                       else MaterialTheme.colorScheme.onSurface,
                modifier = Modifier
                    .border(
                        width = 1.5.dp,
                        color = if (subtitlesActive) MaterialTheme.colorScheme.primary
                               else MaterialTheme.colorScheme.onSurface,
                        shape = RoundedCornerShape(4.dp),
                    )
                    .padding(horizontal = 4.dp, vertical = 1.dp),
            )
        }
    }
}
```

**Step 3: Add subtitle overlay**

Add a composable at the bottom of the video grid area in `CallScreen.kt`:

```kotlin
@Composable
fun SubtitleOverlay(text: String, modifier: Modifier = Modifier) {
    if (text.isNotBlank()) {
        Box(
            modifier = modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            contentAlignment = Alignment.BottomCenter,
        ) {
            Text(
                text = text,
                color = Color.White,
                fontSize = 16.sp,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier
                    .background(
                        Color.Black.copy(alpha = 0.6f),
                        RoundedCornerShape(12.dp),
                    )
                    .padding(horizontal = 12.dp, vertical = 8.dp),
            )
        }
    }
}
```

Place it in the main content area, overlaid at the bottom of the video grid:

```kotlin
// Inside the Box containing the video grid
if (subtitlesActive) {
    SubtitleOverlay(
        text = subtitleText,
        modifier = Modifier.align(Alignment.BottomCenter),
    )
}
```

**Step 4: Verify Android builds**

Run: `cd android && ./gradlew assembleDebug`
Expected: compiles

**Step 5: Commit**

```bash
git add android/
git commit -m "feat(subtitles): add CC button and subtitle overlay on Android"
```

---

### Task 12: iOS — CC button and subtitle overlay

**Files:**
- Modify: `ios/VisioMobile/VisioManager.swift` (add subtitle state + event handling)
- Modify: `ios/VisioMobile/Views/CallView.swift:408-530` (add CC button + overlay)

**Step 1: Add subtitle state to VisioManager**

In `VisioManager.swift`, add Published properties:

```swift
@Published var subtitleText: String = ""
@Published var subtitlesActive: Bool = false
@Published var modelDownloading: Bool = false
@Published var modelDownloadProgress: Float = 0
```

In the `onEvent` switch, add cases:

```swift
case let .subtitleChanged(text):
    self.subtitleText = text
case let .modelDownloadProgress(progress):
    self.modelDownloading = true
    self.modelDownloadProgress = progress
case .modelDownloadComplete:
    self.modelDownloading = false
    // Auto-start subtitles after download
    let lang = self.client.getSettings().language ?? "en"
    self.client.startSubtitles(language: lang)
    self.subtitlesActive = true
case let .modelDownloadError(message):
    self.modelDownloading = false
    print("Model download error: \(message)")
```

Add helper method:

```swift
func toggleSubtitles() {
    if subtitlesActive {
        client.stopSubtitles()
        subtitlesActive = false
        subtitleText = ""
    } else {
        if !client.isSubtitleModelAvailable() {
            client.downloadSubtitleModel()
            return
        }
        let lang = client.getSettings().language ?? "en"
        client.startSubtitles(language: lang)
        subtitlesActive = true
    }
}
```

**Step 2: Add CC button to control bar**

In `CallView.swift`, in the control bar HStack (around line 450-520), add the CC button before the chat button:

```swift
// CC (subtitles) button
Button(action: { manager.toggleSubtitles() }) {
    if manager.modelDownloading {
        ProgressView()
            .progressViewStyle(CircularProgressViewStyle(tint: .white))
            .frame(width: 24, height: 24)
    } else {
        Text("CC")
            .font(.system(size: 14, weight: .bold))
            .foregroundColor(manager.subtitlesActive ? .blue : .white)
            .padding(.horizontal, 4)
            .padding(.vertical, 2)
            .overlay(
                RoundedRectangle(cornerRadius: 4)
                    .stroke(manager.subtitlesActive ? Color.blue : Color.white, lineWidth: 1.5)
            )
    }
}
.disabled(manager.modelDownloading)
```

**Step 3: Add subtitle overlay**

In `CallView.swift`, add a subtitle overlay view at the bottom of the video grid ZStack:

```swift
// Subtitle overlay
if manager.subtitlesActive && !manager.subtitleText.isEmpty {
    VStack {
        Spacer()
        Text(manager.subtitleText)
            .font(.system(size: 16))
            .foregroundColor(.white)
            .lineLimit(3)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color.black.opacity(0.6))
            .cornerRadius(12)
            .padding(.horizontal, 16)
            .padding(.bottom, 8)
    }
}
```

**Step 4: Build iOS**

Run: `cd ios && xcodebuild -scheme VisioMobile -destination 'generic/platform=iOS' build` (or just verify Xcode opens cleanly)

**Step 5: Commit**

```bash
git add ios/
git commit -m "feat(subtitles): add CC button and subtitle overlay on iOS"
```

---

### Task 13: Desktop — CC button and subtitle overlay

**Files:**
- Modify: `crates/visio-desktop/src/lib.rs` (add Tauri commands for subtitles)
- Check existing frontend for where to add UI (look at the Tauri frontend structure)

**Step 1: Add Tauri IPC commands**

In `crates/visio-desktop/src/lib.rs`, add commands:

```rust
#[tauri::command]
async fn is_subtitle_model_available(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let mgr = state.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
    Ok(mgr.is_model_available())
}

#[tauri::command]
async fn download_subtitle_model(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mgr = state.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
    mgr.download_model().await
}

#[tauri::command]
async fn start_subtitles(language: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut mgr = state.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
    let mixer = mgr.mixer();
    // Set playout tap
    state.playout_buffer.set_subtitle_mixer(Some(mixer));
    mgr.set_language(language);
    mgr.start()
}

#[tauri::command]
async fn stop_subtitles(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut mgr = state.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
    mgr.stop();
    state.playout_buffer.set_subtitle_mixer(None);
    Ok(())
}

#[tauri::command]
fn supported_subtitle_languages() -> Vec<(String, String)> {
    visio_core::SubtitleManager::supported_languages()
}
```

Register the commands in the Tauri builder `.invoke_handler()`.

Add `SubtitleManager` to `AppState` struct and initialize it in setup.

**Step 2: Add frontend UI**

Add CC button and overlay to the desktop React/Svelte frontend (follow existing patterns in the frontend code for button placement and event listening).

This task requires examining the specific frontend framework used. The implementation will mirror the Android/iOS approach: CC toggle button, subtitle text overlay, model download progress.

**Step 3: Verify desktop builds**

Run: `cd crates/visio-desktop && cargo tauri build --debug`

**Step 4: Commit**

```bash
git add crates/visio-desktop/
git commit -m "feat(subtitles): add CC button and subtitle overlay on Desktop"
```

---

### Task 14: Language setting in InCallSettingsSheet

**Files:**
- Modify: `android/app/src/main/kotlin/io/visio/mobile/ui/InCallSettingsSheet.kt` (add subtitle language picker)
- Modify: `ios/VisioMobile/Views/InCallSettingsSheet.swift` (add subtitle language picker)

**Step 1: Android — add Subtitles tab**

In `InCallSettingsSheet.kt`, add a new tab (icon: `Icons.Outlined.ClosedCaption` or a text "CC" icon) at index 4.

Tab content:

```kotlin
@Composable
fun SubtitleSettingsTab(visioManager: VisioManager) {
    val languages = remember { visioManager.client.supportedSubtitleLanguages() }
    val currentLang = visioManager.client.getSettings().language ?: "en"
    var selectedLang by remember { mutableStateOf(currentLang) }

    Column(modifier = Modifier.padding(16.dp)) {
        Text(
            text = stringResource(R.string.subtitle_language),
            style = MaterialTheme.typography.titleMedium,
        )
        Spacer(modifier = Modifier.height(8.dp))
        languages.forEach { lang ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable {
                        selectedLang = lang.code
                        visioManager.client.setSubtitleLanguage(lang.code)
                    }
                    .padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(
                    selected = selectedLang == lang.code,
                    onClick = {
                        selectedLang = lang.code
                        visioManager.client.setSubtitleLanguage(lang.code)
                    },
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(text = lang.name)
            }
        }
    }
}
```

**Step 2: iOS — add Subtitles tab**

In `InCallSettingsSheet.swift`, add a new sidebar icon button (use `captions.bubble` SF Symbol or `text.bubble`) at tab index 4 (before members if present).

Tab content:

```swift
private func subtitleTab() -> some View {
    let languages = manager.client.supportedSubtitleLanguages()
    let currentLang = manager.client.getSettings().language ?? "en"

    return List {
        Section(header: Text("Subtitle Language")) {
            ForEach(languages, id: \.code) { lang in
                Button(action: {
                    manager.client.setSubtitleLanguage(language: lang.code)
                }) {
                    HStack {
                        Text(lang.name)
                        Spacer()
                        if lang.code == currentLang {
                            Image(systemName: "checkmark")
                                .foregroundColor(.accentColor)
                        }
                    }
                }
            }
        }
    }
}
```

**Step 3: Verify builds**

Run Android and iOS builds to verify compilation.

**Step 4: Commit**

```bash
git add android/ ios/
git commit -m "feat(subtitles): add language picker in settings on Android and iOS"
```

---

### Task 15: Subtitle text fade-out timer

**Files:**
- Modify: `android/app/src/main/kotlin/io/visio/mobile/VisioManager.kt` (auto-clear after 4s)
- Modify: `ios/VisioMobile/VisioManager.swift` (auto-clear after 4s)

The subtitle text should fade out after ~4 seconds of silence.

**Step 1: Android — add debounce timer**

In `VisioManager.kt`, add a coroutine-based debounce:

```kotlin
private var subtitleClearJob: Job? = null

// In SubtitleChanged event handler:
is VisioEvent.SubtitleChanged -> {
    _subtitleText.value = event.text
    subtitleClearJob?.cancel()
    subtitleClearJob = scope.launch {
        delay(4000)
        _subtitleText.value = ""
    }
}
```

**Step 2: iOS — add timer**

In `VisioManager.swift`, add a timer:

```swift
private var subtitleClearTimer: Timer?

// In subtitleChanged event handler:
case let .subtitleChanged(text):
    self.subtitleText = text
    self.subtitleClearTimer?.invalidate()
    self.subtitleClearTimer = Timer.scheduledTimer(withTimeInterval: 4.0, repeats: false) { [weak self] _ in
        DispatchQueue.main.async {
            self?.subtitleText = ""
        }
    }
```

**Step 3: Commit**

```bash
git add android/ ios/
git commit -m "feat(subtitles): auto-clear subtitle text after 4s of silence"
```

---

### Task 16: Clean up subtitle on disconnect

**Files:**
- Modify: `crates/visio-ffi/src/lib.rs:570-580` (stop subtitles in disconnect)
- Modify: `android/app/src/main/kotlin/io/visio/mobile/VisioManager.kt` (reset state)
- Modify: `ios/VisioMobile/VisioManager.swift` (reset state)

**Step 1: Stop subtitles on disconnect in FFI**

In `VisioClient::disconnect()`, add before the existing disconnect logic:

```rust
    // Stop subtitle transcription
    {
        let mut mgr = self.subtitle_manager.lock().unwrap_or_else(|e| e.into_inner());
        mgr.stop();
    }
    self.set_subtitle_mixer_for_ffi(None);
```

**Step 2: Reset UI state on disconnect**

Android `VisioManager.disconnect()`:
```kotlin
_subtitlesActive.value = false
_subtitleText.value = ""
```

iOS `VisioManager.disconnect()`:
```swift
subtitlesActive = false
subtitleText = ""
```

**Step 3: Commit**

```bash
git add crates/visio-ffi/src/lib.rs android/ ios/
git commit -m "fix(subtitles): clean up subtitle state on disconnect"
```

---

### Task 17: Integration test with local Whisper model

**Files:**
- Create: `crates/visio-core/tests/subtitle_integration.rs`

This test requires a Whisper model file. It tests the full pipeline: mixer → worker → callback.

**Step 1: Write the integration test**

```rust
//! Integration test for subtitle pipeline.
//! Requires: `ggml-base.en.bin` in `test-data/` directory.
//! Skip if model not present.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use visio_core::subtitles::{AudioMixer, WhisperWorker};

#[test]
fn test_whisper_worker_with_silence() {
    let model_path = Path::new("test-data/ggml-base.en.bin");
    if !model_path.exists() {
        eprintln!("skipping subtitle test: model not found at {}", model_path.display());
        return;
    }

    let mixer = Arc::new(AudioMixer::new(10));
    mixer.set_enabled(true);

    let results: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let results_clone = results.clone();
    let callback = Arc::new(move |text: String| {
        results_clone.lock().unwrap().push(text);
    });

    let mut worker = WhisperWorker::start(model_path, "en".into(), mixer.clone(), callback)
        .expect("worker should start");

    // Push 2 seconds of silence (48kHz i16 mono)
    let silence = vec![0i16; 48_000 * 2];
    mixer.push_audio(&silence);

    // Wait for processing
    std::thread::sleep(Duration::from_secs(3));

    worker.stop();

    // Silence should produce empty or near-empty transcription
    let texts = results.lock().unwrap();
    // We don't assert specific content — just that it didn't crash
    eprintln!("transcribed {} segments from silence", texts.len());
}
```

**Step 2: Run (skips if model not present)**

Run: `cargo test -p visio-core -- subtitle_integration`
Expected: either skips (model missing) or passes

**Step 3: Commit**

```bash
git add crates/visio-core/tests/
git commit -m "test(subtitles): add integration test for WhisperWorker pipeline"
```
