//! Visio E2E Bot — a headless participant for automated testing.
//!
//! Joins a LiveKit room, publishes synthetic audio/video, sends chat
//! messages and reactions, and logs all received events. Designed to be
//! a deterministic test partner for Maestro, Playwright, and XCUITest.
//!
//! Usage:
//! ```sh
//! # With a LiveKit dev server:
//! visio-bot --url ws://localhost:7880 --room test-room --identity bot --name "E2E Bot"
//!
//! # With token generation (default keys for --dev server):
//! visio-bot --url ws://localhost:7880 --room my-room \
//!   --api-key devkey --api-secret secret
//!
//! # With a pre-generated token:
//! visio-bot --url ws://localhost:7880 --token <jwt>
//! ```

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

    /// Publish synthetic audio (440Hz sine wave).
    #[arg(long, default_value_t = true)]
    audio: bool,

    /// Publish synthetic video (colored frames).
    #[arg(long, default_value_t = true)]
    video: bool,

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
}

/// Event logger that prints all received events.
struct BotEventLogger;

impl VisioEventListener for BotEventLogger {
    fn on_event(&self, event: VisioEvent) {
        match &event {
            VisioEvent::ConnectionStateChanged(state) => {
                tracing::info!("[EVENT] ConnectionStateChanged: {state:?}");
            }
            VisioEvent::ParticipantJoined(info) => {
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
                tracing::info!("[EVENT] TrackSubscribed: {:?} from {}", info.source, info.participant_sid);
            }
            VisioEvent::TrackUnsubscribed(sid) => {
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

    // Cycle through colors every 30 frames
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
fn spawn_audio_producer(source: livekit::webrtc::audio_source::native::NativeAudioSource) {
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
fn spawn_video_producer(source: NativeVideoSource) {
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
            // ~15fps
            tokio::time::sleep(Duration::from_millis(66)).await;
        }
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

    let token = args.token.clone().unwrap_or_else(|| generate_token(&args));

    tracing::info!(
        "Visio Bot starting: room={}, identity={}, name={}, audio={}, video={}, duration={}s",
        args.room, args.identity, args.name, args.audio, args.video, args.duration
    );

    let rm = RoomManager::new();
    rm.add_listener(Arc::new(BotEventLogger));

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
                tracing::info!("Publishing synthetic audio (440Hz sine)");
                spawn_audio_producer(source);
            }
            Err(e) => tracing::warn!("Failed to publish mic: {e}"),
        }
    }

    // Publish video
    if args.video {
        match controls.publish_camera().await {
            Ok(source) => {
                tracing::info!("Publishing synthetic video (640x480 color cycling)");
                spawn_video_producer(source);
            }
            Err(e) => tracing::warn!("Failed to publish camera: {e}"),
        }
    }

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

    // Stay in room
    if args.duration > 0 {
        tracing::info!("Staying in room for {}s...", args.duration);
        tokio::time::sleep(Duration::from_secs(args.duration)).await;
    } else {
        tracing::info!("Staying in room indefinitely (Ctrl+C to exit)...");
        tokio::signal::ctrl_c().await.ok();
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
