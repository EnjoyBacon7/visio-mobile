//! Visio E2E Bot — a headless participant for automated testing.
//!
//! Joins a LiveKit room, publishes synthetic or real audio/video, sends chat
//! messages and reactions, and logs all received events. Designed to be
//! a deterministic test partner for Maestro, Playwright, and XCUITest.
//!
//! Usage:
//! ```sh
//! # Synthetic media (440Hz sine + colored frames):
//! visio-bot --url ws://localhost:7880 --room test-room
//!
//! # Real video file (requires ffmpeg):
//! visio-bot --url ws://localhost:7880 --room test-room \
//!   --media-file test-assets/test-video.mp4
//!
//! # With a pre-generated token:
//! visio-bot --url ws://localhost:7880 --token <jwt>
//! ```

use std::io::Read as IoRead;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use livekit::webrtc::prelude::*;
use livekit::webrtc::video_source::native::NativeVideoSource;
use visio_core::{ConnectionState, RoomManager, VisioEvent, VisioEventListener};

/// Visio E2E Bot — headless test participant.
#[derive(Parser, Debug)]
#[command(name = "visio-bot", about = "Headless LiveKit participant for E2E testing")]
struct Args {
    /// LiveKit server WebSocket URL.
    #[arg(long, default_value = "ws://localhost:7880")]
    url: String,

    /// Room name to join.
    #[arg(long, default_value = "e2e-test")]
    room: String,

    /// Bot identity.
    #[arg(long, default_value = "e2e-bot")]
    identity: String,

    /// Bot display name.
    #[arg(long, default_value = "E2E Bot")]
    name: String,

    /// Pre-generated JWT token (overrides --room, --identity, --name).
    #[arg(long)]
    token: Option<String>,

    /// LiveKit API key (for token generation).
    #[arg(long, default_value = "devkey")]
    api_key: String,

    /// LiveKit API secret (for token generation).
    #[arg(long, default_value = "secret")]
    api_secret: String,

    /// Publish audio.
    #[arg(long, default_value_t = true)]
    audio: bool,

    /// Publish video.
    #[arg(long, default_value_t = true)]
    video: bool,

    /// Media file to stream (mp4/webm/mkv — requires ffmpeg).
    /// When set, streams real audio/video instead of synthetic signals.
    #[arg(long)]
    media_file: Option<String>,

    /// Loop media file playback.
    #[arg(long, default_value_t = false)]
    loop_media: bool,

    /// Send a chat message after joining.
    #[arg(long)]
    chat_message: Option<String>,

    /// Send a reaction emoji after joining.
    #[arg(long)]
    reaction: Option<String>,

    /// Duration to stay in the room (seconds). 0 = stay forever.
    #[arg(long, default_value_t = 60)]
    duration: u64,

    /// Raise hand after joining.
    #[arg(long, default_value_t = false)]
    raise_hand: bool,

    /// Publish a screen share track (uses media file or synthetic video).
    #[arg(long, default_value_t = false)]
    screen_share: bool,

    /// Monitor received audio quality (log frame stats, detect gaps).
    #[arg(long, default_value_t = false)]
    monitor_audio: bool,

    /// Expected number of remote participants to wait for before reporting.
    #[arg(long, default_value_t = 0)]
    expect_participants: usize,

    /// Print the generated token and exit (don't connect).
    #[arg(long, default_value_t = false)]
    token_only: bool,
}

/// Tracks received audio frame statistics for quality monitoring.
#[allow(dead_code)]
struct AudioStats {
    frames_received: AtomicU64,
    silent_frames: AtomicU64,
    last_frame_time: std::sync::Mutex<Option<std::time::Instant>>,
    max_gap_ms: AtomicU64,
}

#[allow(dead_code)]
impl AudioStats {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            frames_received: AtomicU64::new(0),
            silent_frames: AtomicU64::new(0),
            last_frame_time: std::sync::Mutex::new(None),
            max_gap_ms: AtomicU64::new(0),
        })
    }

    fn record_frame(&self, is_silent: bool) {
        self.frames_received.fetch_add(1, Ordering::Relaxed);
        if is_silent {
            self.silent_frames.fetch_add(1, Ordering::Relaxed);
        }

        let now = std::time::Instant::now();
        let mut last = self.last_frame_time.lock().unwrap();
        if let Some(prev) = *last {
            let gap = now.duration_since(prev).as_millis() as u64;
            let cur_max = self.max_gap_ms.load(Ordering::Relaxed);
            if gap > cur_max {
                self.max_gap_ms.store(gap, Ordering::Relaxed);
            }
        }
        *last = Some(now);
    }

    fn report(&self) -> String {
        let total = self.frames_received.load(Ordering::Relaxed);
        let silent = self.silent_frames.load(Ordering::Relaxed);
        let max_gap = self.max_gap_ms.load(Ordering::Relaxed);
        let duration_s = total as f64 * 0.02; // 20ms per frame
        format!(
            "frames={total}, duration={duration_s:.1}s, silent={silent} ({:.1}%), max_gap={max_gap}ms",
            if total > 0 { silent as f64 / total as f64 * 100.0 } else { 0.0 }
        )
    }
}

/// Event logger that prints all received events and tracks subscription stats.
struct BotEventLogger {
    tracks_subscribed: AtomicU64,
    tracks_unsubscribed: AtomicU64,
    participants_joined: AtomicU64,
}

impl BotEventLogger {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            tracks_subscribed: AtomicU64::new(0),
            tracks_unsubscribed: AtomicU64::new(0),
            participants_joined: AtomicU64::new(0),
        })
    }
}

impl VisioEventListener for BotEventLogger {
    fn on_event(&self, event: VisioEvent) {
        match &event {
            VisioEvent::ConnectionStateChanged(state) => {
                tracing::info!("[EVENT] ConnectionStateChanged: {state:?}");
            }
            VisioEvent::ParticipantJoined(info) => {
                self.participants_joined.fetch_add(1, Ordering::Relaxed);
                tracing::info!(
                    "[EVENT] ParticipantJoined: {} ({})",
                    info.identity,
                    info.name.as_deref().unwrap_or("?")
                );
            }
            VisioEvent::ParticipantLeft(sid) => {
                tracing::info!("[EVENT] ParticipantLeft: {sid}");
            }
            VisioEvent::TrackSubscribed(info) => {
                let count = self.tracks_subscribed.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::info!("[EVENT] TrackSubscribed: {:?} from {} (total: {count})", info.source, info.participant_sid);
            }
            VisioEvent::TrackUnsubscribed(sid) => {
                self.tracks_unsubscribed.fetch_add(1, Ordering::Relaxed);
                tracing::info!("[EVENT] TrackUnsubscribed: {sid}");
            }
            VisioEvent::TrackMuted { participant_sid, source } => {
                tracing::info!("[EVENT] TrackMuted: {source:?} from {participant_sid}");
            }
            VisioEvent::TrackUnmuted { participant_sid, source } => {
                tracing::info!("[EVENT] TrackUnmuted: {source:?} from {participant_sid}");
            }
            VisioEvent::ChatMessageReceived(msg) => {
                tracing::info!("[EVENT] ChatMessage: '{}' from {}", msg.text, msg.sender_name);
            }
            VisioEvent::ReactionReceived { participant_name, emoji, .. } => {
                tracing::info!("[EVENT] Reaction: {emoji} from {participant_name}");
            }
            VisioEvent::HandRaisedChanged { participant_sid, raised, position } => {
                tracing::info!("[EVENT] HandRaised: {participant_sid} raised={raised} pos={position}");
            }
            VisioEvent::ActiveSpeakersChanged(sids) => {
                tracing::debug!("[EVENT] ActiveSpeakers: {sids:?}");
            }
            VisioEvent::ConnectionQualityChanged { participant_sid, quality } => {
                tracing::debug!("[EVENT] ConnectionQuality: {participant_sid} {quality:?}");
            }
            VisioEvent::AdaptiveModeChanged { mode } => {
                tracing::info!("[EVENT] AdaptiveModeChanged: {mode:?}");
            }
            VisioEvent::BandwidthModeChanged { mode } => {
                tracing::info!("[EVENT] BandwidthModeChanged: {mode:?}");
            }
            _ => {
                tracing::debug!("[EVENT] {event:?}");
            }
        }
    }
}

fn generate_token(args: &Args) -> String {
    use livekit_api::access_token::{AccessToken, VideoGrants};
    AccessToken::with_api_key(&args.api_key, &args.api_secret)
        .with_identity(&args.identity)
        .with_name(&args.name)
        .with_grants(VideoGrants {
            room_join: true,
            room: args.room.clone(),
            can_update_own_metadata: true,
            can_publish: true,
            can_subscribe: true,
            ..Default::default()
        })
        .to_jwt()
        .expect("failed to generate token")
}

// ---------------------------------------------------------------------------
// Synthetic media generators (fallback when no --media-file)
// ---------------------------------------------------------------------------

/// Generate a 440Hz sine wave audio frame (20ms at 48kHz mono).
fn generate_sine_frame(sample_offset: &mut u64) -> Vec<i16> {
    const SAMPLE_RATE: f64 = 48000.0;
    const FREQ: f64 = 440.0;
    const AMPLITUDE: f64 = 3000.0;
    const SAMPLES_PER_FRAME: usize = 960; // 20ms at 48kHz

    let mut samples = Vec::with_capacity(SAMPLES_PER_FRAME);
    for i in 0..SAMPLES_PER_FRAME {
        let t = (*sample_offset + i as u64) as f64 / SAMPLE_RATE;
        let val = (t * FREQ * 2.0 * std::f64::consts::PI).sin() * AMPLITUDE;
        samples.push(val as i16);
    }
    *sample_offset += SAMPLES_PER_FRAME as u64;
    samples
}

/// Generate a solid-color I420 video frame.
fn generate_color_frame(width: u32, height: u32, frame_num: u64) -> I420Buffer {
    let mut buf = I420Buffer::new(width, height);
    let (y_data, u_data, v_data) = buf.data_mut();

    let phase = (frame_num / 30) % 3;
    let (y_val, u_val, v_val) = match phase {
        0 => (82u8, 90u8, 240u8),   // Red
        1 => (145u8, 54u8, 34u8),   // Green
        _ => (41u8, 240u8, 110u8),  // Blue
    };

    y_data.fill(y_val);
    u_data.fill(u_val);
    v_data.fill(v_val);

    buf
}

/// Spawn a task that feeds synthetic audio to the audio source.
fn spawn_synthetic_audio(source: livekit::webrtc::audio_source::native::NativeAudioSource) {
    tokio::spawn(async move {
        let mut sample_offset: u64 = 0;
        loop {
            let samples = generate_sine_frame(&mut sample_offset);
            let frame = AudioFrame {
                data: samples.into(),
                sample_rate: 48000,
                num_channels: 1,
                samples_per_channel: 960,
            };
            source.capture_frame(&frame).await.ok();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
}

/// Spawn a task that feeds synthetic video to the video source.
fn spawn_synthetic_video(source: NativeVideoSource) {
    tokio::spawn(async move {
        let mut frame_num: u64 = 0;
        loop {
            let buf = generate_color_frame(640, 480, frame_num);
            let frame: VideoFrame<I420Buffer> = VideoFrame {
                rotation: VideoRotation::VideoRotation0,
                buffer: buf,
                timestamp_us: 0,
            };
            source.capture_frame(&frame);
            frame_num += 1;
            tokio::time::sleep(Duration::from_millis(66)).await;
        }
    });
}

// ---------------------------------------------------------------------------
// Media file playback via ffmpeg subprocess
// ---------------------------------------------------------------------------

const VIDEO_WIDTH: u32 = 640;
const VIDEO_HEIGHT: u32 = 480;
const VIDEO_FPS: u32 = 15;
const AUDIO_SAMPLE_RATE: u32 = 48000;
const AUDIO_CHANNELS: u32 = 1;
const AUDIO_FRAME_DURATION_MS: u64 = 20;
const AUDIO_SAMPLES_PER_FRAME: u32 = (AUDIO_SAMPLE_RATE * AUDIO_FRAME_DURATION_MS as u32) / 1000; // 960

/// Spawn ffmpeg to decode audio from a media file as raw s16le PCM.
fn spawn_ffmpeg_audio(path: &str, do_loop: bool) -> std::process::Child {
    let mut cmd = Command::new("ffmpeg");
    if do_loop {
        cmd.args(["-stream_loop", "-1"]);
    }
    cmd.args([
        "-i", path,
        "-vn",                         // no video
        "-f", "s16le",                 // raw signed 16-bit little-endian
        "-acodec", "pcm_s16le",
        "-ar", &AUDIO_SAMPLE_RATE.to_string(),
        "-ac", &AUDIO_CHANNELS.to_string(),
        "-loglevel", "error",
        "-",                           // pipe to stdout
    ]);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn ffmpeg for audio — is ffmpeg installed?")
}

/// Spawn ffmpeg to decode video from a media file as raw I420 (YUV420p).
fn spawn_ffmpeg_video(path: &str, do_loop: bool) -> std::process::Child {
    let mut cmd = Command::new("ffmpeg");
    if do_loop {
        cmd.args(["-stream_loop", "-1"]);
    }
    cmd.args([
        "-i", path,
        "-an",                         // no audio
        "-f", "rawvideo",
        "-pix_fmt", "yuv420p",        // I420
        "-s", &format!("{VIDEO_WIDTH}x{VIDEO_HEIGHT}"),
        "-r", &VIDEO_FPS.to_string(),
        "-loglevel", "error",
        "-",                           // pipe to stdout
    ]);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn ffmpeg for video — is ffmpeg installed?")
}

/// Spawn a task that reads decoded audio from ffmpeg and feeds it to LiveKit.
fn spawn_file_audio(
    source: livekit::webrtc::audio_source::native::NativeAudioSource,
    path: String,
    do_loop: bool,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();

        rt.block_on(async move {
            let mut child = spawn_ffmpeg_audio(&path, do_loop);
            let mut stdout = child.stdout.take().expect("no stdout from ffmpeg audio");

            // Each frame: 960 samples × 2 bytes = 1920 bytes
            let frame_bytes = AUDIO_SAMPLES_PER_FRAME as usize * 2;
            let mut buf = vec![0u8; frame_bytes];
            let mut total_frames: u64 = 0;

            loop {
                match stdout.read_exact(&mut buf) {
                    Ok(()) => {}
                    Err(_) => {
                        tracing::info!("Audio file ended after {total_frames} frames");
                        break;
                    }
                }

                // Convert bytes to i16 samples (little-endian)
                let samples: Vec<i16> = buf
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]))
                    .collect();

                let frame = AudioFrame {
                    data: samples.into(),
                    sample_rate: AUDIO_SAMPLE_RATE,
                    num_channels: AUDIO_CHANNELS,
                    samples_per_channel: AUDIO_SAMPLES_PER_FRAME,
                };
                source.capture_frame(&frame).await.ok();
                total_frames += 1;

                tokio::time::sleep(Duration::from_millis(AUDIO_FRAME_DURATION_MS)).await;
            }

            child.kill().ok();
        });
    });
}

/// Spawn a task that reads decoded video from ffmpeg and feeds it to LiveKit.
fn spawn_file_video(source: NativeVideoSource, path: String, do_loop: bool) {
    std::thread::spawn(move || {
        let mut child = spawn_ffmpeg_video(&path, do_loop);
        let mut stdout = child.stdout.take().expect("no stdout from ffmpeg video");

        // I420 frame size: W×H × 1.5 (Y + U/4 + V/4)
        let y_size = (VIDEO_WIDTH * VIDEO_HEIGHT) as usize;
        let uv_size = y_size / 4;
        let frame_bytes = y_size + 2 * uv_size;
        let mut buf = vec![0u8; frame_bytes];
        let mut total_frames: u64 = 0;
        let frame_interval = Duration::from_millis(1000 / VIDEO_FPS as u64);

        loop {
            let start = std::time::Instant::now();

            match stdout.read_exact(&mut buf) {
                Ok(()) => {}
                Err(_) => {
                    tracing::info!("Video file ended after {total_frames} frames");
                    break;
                }
            }

            // Build I420Buffer from raw YUV data
            let mut i420 = I420Buffer::new(VIDEO_WIDTH, VIDEO_HEIGHT);
            {
                // Get strides before mutable borrow
                let (stride_y, stride_u, stride_v) = i420.strides();
                let stride_y = stride_y as usize;
                let stride_u = stride_u as usize;
                let stride_v = stride_v as usize;
                let (y_data, u_data, v_data) = i420.data_mut();
                let w = VIDEO_WIDTH as usize;
                let h = VIDEO_HEIGHT as usize;

                // Y plane
                for row in 0..h {
                    let src_off = row * w;
                    let dst_off = row * stride_y;
                    y_data[dst_off..dst_off + w].copy_from_slice(&buf[src_off..src_off + w]);
                }
                // U plane
                let u_w = w / 2;
                let u_h = h / 2;
                let u_start = y_size;
                for row in 0..u_h {
                    let src_off = u_start + row * u_w;
                    let dst_off = row * stride_u;
                    u_data[dst_off..dst_off + u_w].copy_from_slice(&buf[src_off..src_off + u_w]);
                }
                // V plane
                let v_start = y_size + uv_size;
                for row in 0..u_h {
                    let src_off = v_start + row * u_w;
                    let dst_off = row * stride_v;
                    v_data[dst_off..dst_off + u_w].copy_from_slice(&buf[src_off..src_off + u_w]);
                }
            }

            let frame: VideoFrame<I420Buffer> = VideoFrame {
                rotation: VideoRotation::VideoRotation0,
                buffer: i420,
                timestamp_us: 0,
            };
            source.capture_frame(&frame);
            total_frames += 1;

            // Maintain frame rate
            let elapsed = start.elapsed();
            if elapsed < frame_interval {
                std::thread::sleep(frame_interval - elapsed);
            }
        }

        child.kill().ok();
    });
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "visio_bot=info,visio_core=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    // Validate media file exists if provided
    if let Some(ref path) = args.media_file {
        if !std::path::Path::new(path).exists() {
            eprintln!("Media file not found: {path}");
            eprintln!("Run: ./scripts/download-test-media.sh");
            std::process::exit(1);
        }
        // Check ffmpeg is available
        if Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
            eprintln!("ffmpeg not found — required for --media-file");
            eprintln!("Install: brew install ffmpeg");
            std::process::exit(1);
        }
    }

    let token = args.token.clone().unwrap_or_else(|| generate_token(&args));

    if args.token_only {
        println!("{token}");
        return;
    }

    let media_label = if args.media_file.is_some() { "file" } else { "synthetic" };
    tracing::info!(
        "Visio Bot starting: room={}, identity={}, name={}, media={}, audio={}, video={}, duration={}s",
        args.room, args.identity, args.name, media_label, args.audio, args.video, args.duration
    );

    let rm = RoomManager::new();
    let event_logger = BotEventLogger::new();
    rm.add_listener(event_logger.clone());

    // Connect
    tracing::info!("Connecting to {}", args.url);
    rm.connect_with_token(&args.url, &token)
        .await
        .expect("Failed to connect to LiveKit server");
    tracing::info!("Connected!");

    let controls = rm.controls();

    // Publish audio
    if args.audio {
        match controls.publish_microphone().await {
            Ok(source) => {
                if let Some(ref path) = args.media_file {
                    tracing::info!("Publishing audio from file: {path}");
                    spawn_file_audio(source, path.clone(), args.loop_media);
                } else {
                    tracing::info!("Publishing synthetic audio (440Hz sine)");
                    spawn_synthetic_audio(source);
                }
            }
            Err(e) => tracing::warn!("Failed to publish mic: {e}"),
        }
    }

    // Publish video
    if args.video {
        match controls.publish_camera().await {
            Ok(source) => {
                if let Some(ref path) = args.media_file {
                    tracing::info!("Publishing video from file: {path} ({}x{} @{}fps)", VIDEO_WIDTH, VIDEO_HEIGHT, VIDEO_FPS);
                    spawn_file_video(source, path.clone(), args.loop_media);
                } else {
                    tracing::info!("Publishing synthetic video (640x480 color cycling)");
                    spawn_synthetic_video(source);
                }
            }
            Err(e) => tracing::warn!("Failed to publish camera: {e}"),
        }
    }

    // Publish screen share
    if args.screen_share {
        match controls.publish_screen_share().await {
            Ok(source) => {
                if let Some(ref path) = args.media_file {
                    tracing::info!("Publishing screen share from file: {path}");
                    spawn_file_video(source, path.clone(), args.loop_media);
                } else {
                    tracing::info!("Publishing synthetic screen share");
                    spawn_synthetic_video(source);
                }
            }
            Err(e) => tracing::warn!("Failed to publish screen share: {e}"),
        }
    }

    // Wait for expected participants
    if args.expect_participants > 0 {
        tracing::info!("Waiting for {} remote participant(s)...", args.expect_participants);
        let timeout = Duration::from_secs(60);
        let start_wait = std::time::Instant::now();
        loop {
            let participants = rm.participants().await;
            let remote_count = participants.iter().filter(|p| p.identity != args.identity).count();
            if remote_count >= args.expect_participants {
                tracing::info!("All {} expected participant(s) joined", args.expect_participants);
                break;
            }
            if start_wait.elapsed() > timeout {
                tracing::warn!(
                    "Timeout waiting for participants: got {remote_count}/{}", args.expect_participants
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    // Monitor received audio/video track subscriptions
    let audio_stats = if args.monitor_audio {
        let stats = AudioStats::new();
        let stats_clone = stats.clone();
        // Periodic reporting
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let report = stats_clone.report();
                tracing::info!("[AUDIO QUALITY] {report}");
            }
        });
        Some(stats)
    } else {
        None
    };

    // Wait a bit for tracks to propagate
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Send chat message if requested
    if let Some(msg) = &args.chat_message {
        match rm.chat().send_message(msg).await {
            Ok(_) => tracing::info!("Sent chat: '{msg}'"),
            Err(e) => tracing::warn!("Failed to send chat: {e}"),
        }
    }

    // Send reaction if requested
    if let Some(emoji) = &args.reaction {
        match rm.send_reaction(emoji).await {
            Ok(()) => tracing::info!("Sent reaction: {emoji}"),
            Err(e) => tracing::warn!("Failed to send reaction: {e}"),
        }
    }

    // Raise hand if requested
    if args.raise_hand {
        match rm.raise_hand().await {
            Ok(()) => tracing::info!("Hand raised"),
            Err(e) => tracing::warn!("Failed to raise hand: {e}"),
        }
    }

    // Stay in room, cycling mic and camera toggles
    if args.duration > 0 {
        tracing::info!("Staying in room for {}s (cycling mic/camera toggles)...", args.duration);
        let start_time = std::time::Instant::now();
        let total_duration = Duration::from_secs(args.duration);

        // Helper: sleep up to `dur`, return true if duration expired
        async fn sleep_or_expire(
            start: std::time::Instant,
            total: Duration,
            dur: Duration,
        ) -> bool {
            let elapsed = start.elapsed();
            if elapsed >= total {
                return true;
            }
            let remaining = total - elapsed;
            tokio::time::sleep(dur.min(remaining)).await;
            start.elapsed() >= total
        }

        loop {
            // Sleep 10s before first toggle
            if sleep_or_expire(start_time, total_duration, Duration::from_secs(10)).await {
                break;
            }

            // Toggle mic off
            tracing::info!("[TOGGLE] Muting microphone");
            if let Err(e) = controls.set_microphone_enabled(false).await {
                tracing::warn!("[TOGGLE] Failed to mute mic: {e}");
            }

            if sleep_or_expire(start_time, total_duration, Duration::from_secs(5)).await {
                break;
            }

            // Toggle mic on
            tracing::info!("[TOGGLE] Unmuting microphone");
            if let Err(e) = controls.set_microphone_enabled(true).await {
                tracing::warn!("[TOGGLE] Failed to unmute mic: {e}");
            }

            if sleep_or_expire(start_time, total_duration, Duration::from_secs(5)).await {
                break;
            }

            // Toggle camera off
            tracing::info!("[TOGGLE] Disabling camera");
            if let Err(e) = controls.set_camera_enabled(false).await {
                tracing::warn!("[TOGGLE] Failed to disable camera: {e}");
            }

            if sleep_or_expire(start_time, total_duration, Duration::from_secs(5)).await {
                break;
            }

            // Toggle camera on
            tracing::info!("[TOGGLE] Enabling camera");
            if let Err(e) = controls.set_camera_enabled(true).await {
                tracing::warn!("[TOGGLE] Failed to enable camera: {e}");
            }
        }

        tracing::info!("Toggle cycling complete after {:.1}s", start_time.elapsed().as_secs_f64());
    } else {
        tracing::info!("Staying in room indefinitely (Ctrl+C to exit)...");
        tokio::signal::ctrl_c().await.ok();
    }

    // Final summary
    let subs = event_logger.tracks_subscribed.load(Ordering::Relaxed);
    let unsubs = event_logger.tracks_unsubscribed.load(Ordering::Relaxed);
    let joins = event_logger.participants_joined.load(Ordering::Relaxed);
    tracing::info!(
        "[SUMMARY] participants_joined={joins}, tracks_subscribed={subs}, tracks_unsubscribed={unsubs}"
    );

    // Final audio quality report
    if let Some(ref stats) = audio_stats {
        let report = stats.report();
        tracing::info!("[AUDIO QUALITY FINAL] {report}");
        let total = stats.frames_received.load(Ordering::Relaxed);
        let max_gap = stats.max_gap_ms.load(Ordering::Relaxed);
        if total == 0 {
            tracing::warn!("[AUDIO QUALITY] NO audio frames received — possible pipeline failure");
        } else if max_gap > 200 {
            tracing::warn!("[AUDIO QUALITY] Large gap detected: {max_gap}ms — possible audio stutter");
        } else {
            tracing::info!("[AUDIO QUALITY] Audio quality OK");
        }
    }

    // Disconnect
    tracing::info!("Disconnecting...");
    rm.disconnect().await;

    // Wait for disconnect state
    let start = std::time::Instant::now();
    while rm.connection_state().await != ConnectionState::Disconnected
        && start.elapsed() < Duration::from_secs(5)
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    tracing::info!("Bot exited cleanly");
}
