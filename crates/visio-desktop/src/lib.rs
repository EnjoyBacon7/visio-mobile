use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use tauri::{AppHandle, Emitter, Listener, Manager};
use visio_core::{
    AudioPlayoutBuffer, ChatService, MeetingControls, RoomManager, SessionManager, SessionState,
    SettingsStore, TrackInfo, TrackKind, TrackSource, VisioEvent, VisioEventListener,
};

mod audio_cpal;
#[cfg(target_os = "macos")]
mod camera_macos;
mod screen_capture;

// ---------------------------------------------------------------------------
// Global AppHandle for the C video callback
// ---------------------------------------------------------------------------

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// C callback invoked by visio-video for each rendered desktop frame.
/// Emits a Tauri "video-frame" event to the frontend.
unsafe extern "C" fn on_desktop_frame(
    track_sid: *const std::ffi::c_char,
    data: *const u8,
    data_len: usize,
    width: u32,
    height: u32,
    _user_data: *mut std::ffi::c_void,
) {
    let Some(app) = APP_HANDLE.get() else { return };
    let sid = unsafe { std::ffi::CStr::from_ptr(track_sid) };
    let Ok(sid_str) = sid.to_str() else { return };
    let b64 = unsafe { std::slice::from_raw_parts(data, data_len) };
    let Ok(b64_str) = std::str::from_utf8(b64) else {
        return;
    };

    let _ = app.emit(
        "video-frame",
        serde_json::json!({
            "track_sid": sid_str,
            "data": b64_str,
            "width": width,
            "height": height,
        }),
    );
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct VisioState {
    room: Arc<Mutex<RoomManager>>,
    controls: Arc<Mutex<MeetingControls>>,
    chat: Arc<Mutex<ChatService>>,
    session: Mutex<SessionManager>,
    settings: SettingsStore,
    #[cfg(target_os = "macos")]
    camera_capture: std::sync::Mutex<Option<camera_macos::MacCameraCapture>>,
    audio_playout: std::sync::Mutex<audio_cpal::CpalAudioPlayout>,
    playout_buffer: Arc<AudioPlayoutBuffer>,
    audio_capture: std::sync::Mutex<Option<audio_cpal::CpalAudioCapture>>,
    screen_capture: std::sync::Mutex<Option<screen_capture::ScreenCapture>>,
}

// ---------------------------------------------------------------------------
// Event listener — auto-starts/stops video renderers
// ---------------------------------------------------------------------------

struct DesktopEventListener {
    room: Arc<Mutex<RoomManager>>,
}

fn source_to_str(source: &TrackSource) -> &'static str {
    match source {
        TrackSource::Microphone => "microphone",
        TrackSource::Camera => "camera",
        TrackSource::ScreenShare => "screen_share",
        TrackSource::Unknown => "unknown",
    }
}

impl VisioEventListener for DesktopEventListener {
    fn on_event(&self, event: VisioEvent) {
        match event {
            VisioEvent::ConnectionStateChanged(state) => {
                let name = match &state {
                    visio_core::ConnectionState::Disconnected => "disconnected",
                    visio_core::ConnectionState::Connecting => "connecting",
                    visio_core::ConnectionState::Connected => "connected",
                    visio_core::ConnectionState::Reconnecting { .. } => "reconnecting",
                    visio_core::ConnectionState::WaitingForHost => "waiting_for_host",
                };
                tracing::info!("connection state changed: {name}");
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("connection-state-changed", name);
                }
            }
            VisioEvent::ParticipantJoined(info) => {
                tracing::info!("participant joined: {} ({})", info.identity, info.sid);
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "participant-joined",
                        serde_json::json!({
                            "sid": info.sid,
                            "identity": info.identity,
                            "name": info.name,
                        }),
                    );
                }
            }
            VisioEvent::ParticipantLeft(sid) => {
                tracing::info!("participant left: {sid}");
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("participant-left", &sid);
                }
            }
            VisioEvent::TrackSubscribed(TrackInfo {
                sid: ref track_sid,
                ref participant_sid,
                kind: TrackKind::Video,
                ref source,
            }) => {
                let room = self.room.clone();
                let sid = track_sid.clone();
                tokio::spawn(async move {
                    let rm = room.lock().await;
                    if let Some(video_track) = rm.get_video_track(&sid).await {
                        tracing::info!("auto-starting video renderer for track {sid}");
                        visio_video::start_track_renderer(
                            sid,
                            video_track,
                            std::ptr::null_mut(),
                            None,
                        );
                    }
                });
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "track-subscribed",
                        serde_json::json!({
                            "trackSid": track_sid,
                            "participantSid": participant_sid,
                            "source": source_to_str(source),
                        }),
                    );
                }
            }
            VisioEvent::TrackSubscribed(_) => {}
            VisioEvent::TrackUnsubscribed(track_sid) => {
                tracing::info!("auto-stopping video renderer for track {track_sid}");
                visio_video::stop_track_renderer(&track_sid);
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("track-unsubscribed", &track_sid);
                }
            }
            VisioEvent::TrackMuted {
                participant_sid,
                source,
            } => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "track-muted",
                        serde_json::json!({
                            "participantSid": participant_sid,
                            "source": source_to_str(&source),
                        }),
                    );
                }
            }
            VisioEvent::TrackUnmuted {
                participant_sid,
                source,
            } => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "track-unmuted",
                        serde_json::json!({
                            "participantSid": participant_sid,
                            "source": source_to_str(&source),
                        }),
                    );
                }
            }
            VisioEvent::HandRaisedChanged {
                participant_sid,
                raised,
                position,
            } => {
                tracing::info!(
                    "DesktopEventListener: HandRaisedChanged sid={participant_sid} raised={raised} position={position}"
                );
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "hand-raised-changed",
                        serde_json::json!({
                            "participantSid": participant_sid,
                            "raised": raised,
                            "position": position,
                        }),
                    );
                }
            }
            VisioEvent::UnreadCountChanged(count) => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("unread-count-changed", count);
                }
            }
            VisioEvent::ActiveSpeakersChanged(sids) => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("active-speakers-changed", &sids);
                }
            }
            VisioEvent::ConnectionQualityChanged {
                participant_sid,
                quality,
            } => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "connection-quality-changed",
                        serde_json::json!({
                            "participantSid": participant_sid,
                            "quality": format!("{:?}", quality),
                        }),
                    );
                }
            }
            VisioEvent::ChatMessageReceived(msg) => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "chat-message-received",
                        serde_json::json!({
                            "id": msg.id,
                            "senderSid": msg.sender_sid,
                            "senderName": msg.sender_name,
                            "text": msg.text,
                            "timestampMs": msg.timestamp_ms,
                        }),
                    );
                }
            }
            VisioEvent::LobbyParticipantJoined { id, username } => {
                tracing::info!("lobby participant joined: {username} (id={id})");
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "lobby-participant-joined",
                        serde_json::json!({ "id": id, "username": username }),
                    );
                }
            }
            VisioEvent::LobbyParticipantLeft { id } => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("lobby-participant-left", &id);
                }
            }
            VisioEvent::LobbyDenied => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("lobby-denied", ());
                }
            }
            VisioEvent::ReactionReceived {
                participant_sid,
                participant_name,
                emoji,
            } => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit(
                        "reaction-received",
                        serde_json::json!({
                            "participantSid": participant_sid,
                            "participantName": participant_name,
                            "emoji": emoji,
                        }),
                    );
                }
            }
            VisioEvent::AdaptiveModeChanged { mode } => {
                let mode_str = match mode {
                    visio_core::adaptive::AdaptiveMode::Office => "office",
                    visio_core::adaptive::AdaptiveMode::Pedestrian => "pedestrian",
                    visio_core::adaptive::AdaptiveMode::Car => "car",
                };
                tracing::info!("adaptive mode changed: {mode_str}");
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("adaptive-mode-changed", mode_str);
                }
            }
            VisioEvent::BandwidthModeChanged { mode } => {
                let mode_str = match mode {
                    visio_core::bandwidth::BandwidthMode::Full => "full",
                    visio_core::bandwidth::BandwidthMode::ReducedVideo => "reduced_video",
                    visio_core::bandwidth::BandwidthMode::AudioOnly => "audio_only",
                };
                tracing::info!("bandwidth mode changed: {mode_str}");
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("bandwidth-mode-changed", mode_str);
                }
            }
            VisioEvent::ConnectionLost => {
                if let Some(app) = APP_HANDLE.get() {
                    let _ = app.emit("connection-lost", ());
                }
                let room = self.room.clone();
                tokio::spawn(async move {
                    let rm = room.lock().await;
                    tracing::info!("connection lost, attempting reconnection");
                    if let Err(e) = rm.reconnect().await {
                        tracing::error!("desktop reconnection failed: {e}");
                    }
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn validate_room(
    state: tauri::State<'_, VisioState>,
    url: String,
    username: Option<String>,
) -> Result<serde_json::Value, String> {
    if let Err(e) = visio_core::AuthService::extract_slug(&url) {
        return Ok(serde_json::json!({ "status": "invalid_format", "message": e.to_string() }));
    }
    let cookie = {
        let session = state.session.lock().await;
        session.cookie()
    };
    match visio_core::AuthService::validate_room(&url, username.as_deref(), cookie.as_deref()).await
    {
        Ok(token_info) => Ok(serde_json::json!({
            "status": "valid",
            "livekit_url": token_info.livekit_url,
            "token": token_info.token,
        })),
        Err(visio_core::VisioError::AuthRequired) => {
            Ok(serde_json::json!({ "status": "auth_required" }))
        }
        Err(visio_core::VisioError::Auth(msg)) if msg.contains("404") => {
            Ok(serde_json::json!({ "status": "not_found" }))
        }
        Err(e) => Ok(serde_json::json!({ "status": "error", "message": e.to_string() })),
    }
}

#[tauri::command]
async fn connect(
    state: tauri::State<'_, VisioState>,
    meet_url: String,
    username: Option<String>,
) -> Result<(), String> {
    let cookie = {
        let session = state.session.lock().await;
        session.cookie()
    };
    let room = state.room.lock().await;
    room.connect(&meet_url, username.as_deref(), cookie.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, VisioState>) -> Result<(), String> {
    let room = state.room.lock().await;
    room.disconnect().await;
    Ok(())
}

#[tauri::command]
async fn get_connection_state(state: tauri::State<'_, VisioState>) -> Result<String, String> {
    let room = state.room.lock().await;
    let cs = room.connection_state().await;
    let name = match cs {
        visio_core::ConnectionState::Disconnected => "disconnected",
        visio_core::ConnectionState::Connecting => "connecting",
        visio_core::ConnectionState::Connected => "connected",
        visio_core::ConnectionState::Reconnecting { .. } => "reconnecting",
        visio_core::ConnectionState::WaitingForHost => "waiting_for_host",
    };
    Ok(name.to_string())
}

#[tauri::command]
async fn get_participants(
    state: tauri::State<'_, VisioState>,
) -> Result<Vec<serde_json::Value>, String> {
    let room = state.room.lock().await;
    let participants = room.participants().await;
    let result: Vec<serde_json::Value> = participants
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "sid": p.sid,
                "identity": p.identity,
                "name": p.name,
                "is_muted": p.is_muted,
                "has_video": p.has_video,
                "video_track_sid": p.video_track_sid,
                "connection_quality": format!("{:?}", p.connection_quality),
                "has_screen_share": p.has_screen_share,
                "screen_share_track_sid": p.screen_share_track_sid,
            })
        })
        .collect();
    Ok(result)
}

#[tauri::command]
async fn get_local_participant(
    state: tauri::State<'_, VisioState>,
) -> Result<Option<serde_json::Value>, String> {
    let room = state.room.lock().await;
    let info = room.local_participant_info().await;
    Ok(info.map(|p| {
        serde_json::json!({
            "sid": p.sid,
            "identity": p.identity,
            "name": p.name,
            "is_muted": p.is_muted,
            "has_video": p.has_video,
            "video_track_sid": p.video_track_sid,
            "connection_quality": format!("{:?}", p.connection_quality),
            "has_screen_share": p.has_screen_share,
            "screen_share_track_sid": p.screen_share_track_sid,
        })
    }))
}

#[tauri::command]
async fn get_video_tracks(state: tauri::State<'_, VisioState>) -> Result<Vec<String>, String> {
    let room = state.room.lock().await;
    let sids = room.video_track_sids().await;
    Ok(sids)
}

#[tauri::command]
async fn toggle_mic(state: tauri::State<'_, VisioState>, enabled: bool) -> Result<(), String> {
    let controls = state.controls.lock().await;
    controls
        .set_microphone_enabled(enabled)
        .await
        .map_err(|e| e.to_string())?;

    if enabled {
        // Start capture if not already running
        let already_running = state
            .audio_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        if !already_running {
            if let Some(source) = controls.audio_source().await {
                let capture = audio_cpal::CpalAudioCapture::start(source)
                    .map_err(|e| format!("audio capture: {e}"))?;
                *state
                    .audio_capture
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(capture);
            }
        }
    } else {
        // Stop capture
        let mut cap = state
            .audio_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(capture) = cap.take() {
            capture.stop();
        }
    }

    Ok(())
}

#[tauri::command]
async fn toggle_camera(state: tauri::State<'_, VisioState>, enabled: bool) -> Result<(), String> {
    let controls = state.controls.lock().await;
    if enabled {
        // Publish camera track if not yet published
        if controls.video_source().await.is_none() {
            let _source = controls.publish_camera().await.map_err(|e| e.to_string())?;
            tracing::info!("camera track published via toggle_camera");

            // Start native camera capture
            #[cfg(target_os = "macos")]
            {
                let capture = camera_macos::MacCameraCapture::start(source)
                    .map_err(|e| format!("camera capture: {e}"))?;
                let mut cam = state
                    .camera_capture
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                *cam = Some(capture);
            }
        }
    } else {
        // Stop camera capture when disabling
        #[cfg(target_os = "macos")]
        {
            let mut cam = state
                .camera_capture
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(mut capture) = cam.take() {
                capture.stop();
            }
        }
    }
    controls
        .set_camera_enabled(enabled)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_chat(
    state: tauri::State<'_, VisioState>,
    text: String,
) -> Result<serde_json::Value, String> {
    let chat = state.chat.lock().await;
    let msg = chat.send_message(&text).await.map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "id": msg.id,
        "sender_sid": msg.sender_sid,
        "sender_name": msg.sender_name,
        "text": msg.text,
        "timestamp_ms": msg.timestamp_ms,
    }))
}

#[tauri::command]
async fn get_messages(
    state: tauri::State<'_, VisioState>,
) -> Result<Vec<serde_json::Value>, String> {
    let chat = state.chat.lock().await;
    let messages = chat.messages().await;
    let result: Vec<serde_json::Value> = messages
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "sender_sid": m.sender_sid,
                "sender_name": m.sender_name,
                "text": m.text,
                "timestamp_ms": m.timestamp_ms,
            })
        })
        .collect();
    Ok(result)
}

#[tauri::command]
fn get_translations(app: AppHandle, lang: String) -> Result<serde_json::Value, String> {
    let supported = ["en", "fr", "de", "es", "it", "nl"];
    let lang = if supported.contains(&lang.as_str()) {
        lang
    } else {
        "en".to_string()
    };

    let resource_path = app
        .path()
        .resource_dir()
        .map_err(|e| format!("resource dir: {e}"))?
        .join("i18n")
        .join(format!("{lang}.json"));

    let content = std::fs::read_to_string(&resource_path).map_err(|e| {
        tracing::warn!("failed to load i18n/{lang}.json from {resource_path:?}: {e}");
        format!("i18n file not found: {lang}.json")
    })?;

    serde_json::from_str(&content).map_err(|e| format!("invalid i18n JSON: {e}"))
}

#[tauri::command]
fn get_system_language() -> String {
    sys_locale::get_locale()
        .and_then(|l| l.split(['-', '_']).next().map(String::from))
        .unwrap_or_else(|| "en".to_string())
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, VisioState>) -> Result<serde_json::Value, String> {
    let s = state.settings.get();
    Ok(serde_json::json!({
        "display_name": s.display_name,
        "language": s.language,
        "mic_enabled_on_join": s.mic_enabled_on_join,
        "camera_enabled_on_join": s.camera_enabled_on_join,
        "theme": s.theme,
        "adaptive_mode_enabled": s.adaptive_mode_enabled,
    }))
}

#[tauri::command]
fn set_display_name(
    app: AppHandle,
    state: tauri::State<'_, VisioState>,
    name: Option<String>,
) -> Result<(), String> {
    if let Some(ref n) = name {
        let trimmed = n.trim();
        if trimmed.is_empty() || trimmed.len() > 100 {
            return Err("display name must be 1-100 characters".into());
        }
    }
    state.settings.set_display_name(name.clone());
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"display_name": name}),
    );
    Ok(())
}

#[tauri::command]
fn set_language(
    app: AppHandle,
    state: tauri::State<'_, VisioState>,
    lang: Option<String>,
) -> Result<(), String> {
    let supported = ["en", "fr", "de", "es", "it", "nl"];
    if let Some(ref l) = lang {
        if !supported.contains(&l.as_str()) {
            return Err(format!("unsupported language: {l}"));
        }
    }
    state.settings.set_language(lang.clone());
    let _ = app.emit("settings-changed", serde_json::json!({"language": lang}));
    Ok(())
}

#[tauri::command]
fn set_mic_enabled_on_join(app: AppHandle, state: tauri::State<'_, VisioState>, enabled: bool) {
    state.settings.set_mic_enabled_on_join(enabled);
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"mic_enabled_on_join": enabled}),
    );
}

#[tauri::command]
fn set_camera_enabled_on_join(app: AppHandle, state: tauri::State<'_, VisioState>, enabled: bool) {
    state.settings.set_camera_enabled_on_join(enabled);
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"camera_enabled_on_join": enabled}),
    );
}

#[tauri::command]
fn set_theme(
    app: AppHandle,
    state: tauri::State<'_, VisioState>,
    theme: String,
) -> Result<(), String> {
    let valid = ["light", "dark", "system"];
    if !valid.contains(&theme.as_str()) {
        return Err(format!("invalid theme: {theme}"));
    }
    state.settings.set_theme(theme.clone());
    let _ = app.emit("settings-changed", serde_json::json!({"theme": theme}));
    Ok(())
}

#[tauri::command]
fn get_meet_instances(state: tauri::State<'_, VisioState>) -> Result<Vec<String>, String> {
    Ok(state.settings.get_meet_instances())
}

#[tauri::command]
fn set_meet_instances(state: tauri::State<'_, VisioState>, instances: Vec<String>) {
    state.settings.set_meet_instances(instances);
}

#[tauri::command]
async fn raise_hand(state: tauri::State<'_, VisioState>) -> Result<(), String> {
    tracing::info!("Tauri command: raise_hand");
    let room = state.room.lock().await;
    room.raise_hand().await.map_err(|e| {
        tracing::error!("raise_hand command failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
async fn lower_hand(state: tauri::State<'_, VisioState>) -> Result<(), String> {
    tracing::info!("Tauri command: lower_hand");
    let room = state.room.lock().await;
    room.lower_hand().await.map_err(|e| {
        tracing::error!("lower_hand command failed: {e}");
        e.to_string()
    })
}

#[tauri::command]
async fn is_hand_raised(state: tauri::State<'_, VisioState>) -> Result<bool, String> {
    let room = state.room.lock().await;
    Ok(room.is_hand_raised().await)
}

#[tauri::command]
async fn set_chat_open(state: tauri::State<'_, VisioState>, open: bool) -> Result<(), String> {
    let chat = state.chat.lock().await;
    chat.set_chat_open(open);
    Ok(())
}

#[tauri::command]
async fn send_reaction(state: tauri::State<'_, VisioState>, emoji: String) -> Result<(), String> {
    let room = state.room.lock().await;
    room.send_reaction(&emoji).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn list_screen_sources() -> Vec<screen_capture::ScreenSource> {
    screen_capture::list_sources()
}

#[tauri::command]
async fn start_screen_share(
    state: tauri::State<'_, VisioState>,
    source_id: String,
) -> Result<(), String> {
    tracing::info!("start_screen_share called with source_id={source_id}");
    let controls = state.controls.lock().await;
    let source = controls
        .publish_screen_share()
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("screen share track published, starting capture");

    let capture = screen_capture::ScreenCapture::start(&source_id, source)
        .map_err(|e| format!("screen capture: {e}"))?;

    let mut cap = state
        .screen_capture
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *cap = Some(capture);

    Ok(())
}

#[tauri::command]
async fn stop_screen_share(state: tauri::State<'_, VisioState>) -> Result<(), String> {
    {
        let mut cap = state
            .screen_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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

#[tauri::command]
fn set_background_mode(
    state: tauri::State<'_, VisioState>,
    app: AppHandle,
    mode: String,
) -> Result<(), String> {
    // Validate mode
    if mode != "off" && mode != "blur" && !mode.starts_with("image:") {
        return Err("Invalid background mode".into());
    }
    // Update BlurProcessor
    let bg_mode = match mode.as_str() {
        "blur" => visio_ffi::blur::process::BackgroundMode::Blur,
        m if m.starts_with("image:") => {
            if let Ok(id) = m[6..].parse::<u8>() {
                visio_ffi::blur::process::BackgroundMode::Image(id)
            } else {
                return Err("Invalid image ID".into());
            }
        }
        _ => visio_ffi::blur::process::BackgroundMode::Off,
    };
    visio_ffi::blur::BlurProcessor::set_mode(bg_mode);
    // Persist
    state.settings.set_background_mode(mode);
    let _ = app.emit("settings-changed", ());
    Ok(())
}

#[tauri::command]
fn get_background_mode(state: tauri::State<'_, VisioState>) -> String {
    state.settings.get_background_mode()
}

#[tauri::command]
fn set_adaptive_mode_enabled(
    app: AppHandle,
    state: tauri::State<'_, VisioState>,
    enabled: bool,
) -> Result<(), String> {
    state.settings.set_adaptive_mode_enabled(enabled);
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"adaptive_mode_enabled": enabled}),
    );
    Ok(())
}

#[tauri::command]
fn load_blur_model(model_path: String) -> Result<(), String> {
    visio_ffi::blur::model::load_model(std::path::Path::new(&model_path))
}

#[tauri::command]
fn load_background_image(id: u8, jpeg_path: String) -> Result<(), String> {
    let jpeg_bytes = std::fs::read(&jpeg_path).map_err(|e| e.to_string())?;
    visio_ffi::blur::BlurProcessor::load_replacement_image(id, &jpeg_bytes, 640, 480)
}

#[tauri::command]
fn list_audio_input_devices() -> Vec<audio_cpal::AudioDeviceInfo> {
    audio_cpal::list_input_devices()
}

#[tauri::command]
fn list_audio_output_devices() -> Vec<audio_cpal::AudioDeviceInfo> {
    audio_cpal::list_output_devices()
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn list_video_input_devices() -> Vec<camera_macos::VideoDeviceInfo> {
    camera_macos::list_cameras()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn list_video_input_devices() -> Vec<serde_json::Value> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Device selection commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn select_audio_input(
    state: tauri::State<'_, VisioState>,
    device_name: String,
) -> Result<(), String> {
    let device = audio_cpal::find_input_device(&device_name)
        .ok_or_else(|| format!("Audio input device not found: {device_name}"))?;

    // Stop existing capture
    {
        let mut cap = state
            .audio_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(capture) = cap.take() {
            capture.stop();
        }
    }

    // Restart with selected device if mic is active
    let controls = state.controls.lock().await;
    if let Some(source) = controls.audio_source().await {
        let capture = audio_cpal::CpalAudioCapture::start_with_device(device, source)
            .map_err(|e| format!("audio capture: {e}"))?;
        let mut cap = state
            .audio_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *cap = Some(capture);
    }

    Ok(())
}

#[tauri::command]
fn select_audio_output(
    state: tauri::State<'_, VisioState>,
    device_name: String,
) -> Result<(), String> {
    let device = audio_cpal::find_output_device(&device_name)
        .ok_or_else(|| format!("Audio output device not found: {device_name}"))?;

    let new_playout =
        audio_cpal::CpalAudioPlayout::start_with_device(device, state.playout_buffer.clone())?;

    let mut playout = state
        .audio_playout
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *playout = new_playout;

    Ok(())
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn select_video_input(
    state: tauri::State<'_, VisioState>,
    unique_id: String,
) -> Result<(), String> {
    let controls = state.controls.lock().await;
    let source = controls
        .video_source()
        .await
        .ok_or("No video source — enable camera first")?;

    // Stop existing camera
    {
        let mut cam = state
            .camera_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(mut capture) = cam.take() {
            capture.stop();
        }
    }

    // Start with selected camera
    let capture = camera_macos::MacCameraCapture::start_with_unique_id(&unique_id, source)
        .map_err(|e| format!("camera: {e}"))?;
    let mut cam = state
        .camera_capture
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *cam = Some(capture);

    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn select_video_input(
    _state: tauri::State<'_, VisioState>,
    _unique_id: String,
) -> Result<(), String> {
    Err("Video device selection not implemented on this platform".into())
}

// ---------------------------------------------------------------------------
// Lobby commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn list_waiting_participants(
    state: tauri::State<'_, VisioState>,
) -> Result<serde_json::Value, String> {
    let room = state.room.lock().await;
    let participants = room
        .list_waiting_participants()
        .await
        .map_err(|e| e.to_string())?;
    let json: Vec<_> = participants
        .iter()
        .map(|p| serde_json::json!({ "id": p.id, "username": p.username }))
        .collect();
    Ok(serde_json::json!(json))
}

#[tauri::command]
async fn admit_participant(
    state: tauri::State<'_, VisioState>,
    participant_id: String,
) -> Result<(), String> {
    let room = state.room.lock().await;
    room.handle_lobby_entry(&participant_id, true)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn deny_participant(
    state: tauri::State<'_, VisioState>,
    participant_id: String,
) -> Result<(), String> {
    let room = state.room.lock().await;
    room.handle_lobby_entry(&participant_id, false)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cancel_lobby(state: tauri::State<'_, VisioState>) -> Result<(), String> {
    let room = state.room.lock().await;
    room.cancel_lobby().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Access management commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn search_users(
    state: tauri::State<'_, VisioState>,
    query: String,
) -> Result<serde_json::Value, String> {
    let session = state.session.lock().await;
    let cookie = session.cookie().ok_or("Not authenticated")?;
    let meet_instance = session
        .meet_instance()
        .ok_or("No meet instance")?
        .to_string();
    drop(session);

    let meet_url = format!("https://{}/room", meet_instance);
    let results = visio_core::AccessService::search_users(&meet_url, &cookie, &query)
        .await
        .map_err(|e| e.to_string())?;

    serde_json::to_value(&results).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_accesses(
    state: tauri::State<'_, VisioState>,
    room_id: String,
) -> Result<serde_json::Value, String> {
    let session = state.session.lock().await;
    let cookie = session.cookie().ok_or("Not authenticated")?;
    let meet_instance = session
        .meet_instance()
        .ok_or("No meet instance")?
        .to_string();
    drop(session);

    let meet_url = format!("https://{}/room", meet_instance);
    let results = visio_core::AccessService::list_accesses(&meet_url, &cookie, &room_id)
        .await
        .map_err(|e| e.to_string())?;

    serde_json::to_value(&results).map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_access(
    state: tauri::State<'_, VisioState>,
    user_id: String,
    room_id: String,
) -> Result<serde_json::Value, String> {
    let session = state.session.lock().await;
    let cookie = session.cookie().ok_or("Not authenticated")?;
    let meet_instance = session
        .meet_instance()
        .ok_or("No meet instance")?
        .to_string();
    drop(session);

    let meet_url = format!("https://{}/room", meet_instance);
    let result = visio_core::AccessService::add_access(&meet_url, &cookie, &user_id, &room_id)
        .await
        .map_err(|e| e.to_string())?;

    serde_json::to_value(&result).map_err(|e| e.to_string())
}

#[tauri::command]
async fn remove_access(
    state: tauri::State<'_, VisioState>,
    access_id: String,
) -> Result<(), String> {
    let session = state.session.lock().await;
    let cookie = session.cookie().ok_or("Not authenticated")?;
    let meet_instance = session
        .meet_instance()
        .ok_or("No meet instance")?
        .to_string();
    drop(session);

    let meet_url = format!("https://{}/room", meet_instance);
    visio_core::AccessService::remove_access(&meet_url, &cookie, &access_id)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// OIDC authentication commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn launch_oidc(
    app: AppHandle,
    state: tauri::State<'_, VisioState>,
    meet_instance: String,
) -> Result<serde_json::Value, String> {
    let auth_url = format!("https://{}/api/v1.0/authenticate/", meet_instance);

    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let instance = meet_instance.clone();

    tauri::WebviewWindowBuilder::new(
        &app,
        "auth",
        tauri::WebviewUrl::External(auth_url.parse().map_err(|e| format!("bad URL: {e}"))?),
    )
    .title("Sign in")
    .inner_size(520.0, 700.0)
    .on_navigation(|url| {
        tracing::debug!("auth window navigating to: {}", url);
        true // allow all navigation in the auth window
    })
    .on_page_load({
        let tx = tx.clone();
        move |webview, payload| {
            if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                return;
            }
            let url = payload.url();
            // After SSO callback, Meet redirects to the instance homepage
            if url.host_str() == Some(instance.as_str())
                && !url.path().contains("/oauth2/")
                && !url.path().contains("/authenticate")
                && !url.path().contains("/callback")
            {
                let meet_url: tauri::Url = format!("https://{}/", instance).parse().unwrap();
                if let Ok(cookies) = webview.cookies_for_url(meet_url) {
                    for cookie in &cookies {
                        if cookie.name() == "sessionid" {
                            let mut guard = tx.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(sender) = guard.take() {
                                let _ = sender.send(cookie.value().to_string());
                            }
                            let _ = webview.close();
                            return;
                        }
                    }
                }
            }
        }
    })
    .build()
    .map_err(|e| format!("failed to open auth window: {e}"))?;

    let session_cookie = rx
        .await
        .map_err(|_| "authentication window closed without completing login".to_string())?;

    tracing::info!("OIDC auth complete, session cookie obtained");

    // Fetch user info and store the authenticated session
    let meet_url = format!("https://{}", meet_instance);
    let user = SessionManager::fetch_user(&meet_url, &session_cookie)
        .await
        .map_err(|e| e.to_string())?;
    let mut session = state.session.lock().await;
    session.set_authenticated(user.clone(), session_cookie, meet_instance.clone());

    Ok(serde_json::json!({
        "display_name": user.display_name(),
        "email": user.email,
        "meet_instance": meet_instance,
    }))
}

#[tauri::command]
async fn authenticate(
    state: tauri::State<'_, VisioState>,
    meet_url: String,
    cookie: String,
) -> Result<serde_json::Value, String> {
    let user = SessionManager::fetch_user(&meet_url, &cookie)
        .await
        .map_err(|e| e.to_string())?;
    let mut session = state.session.lock().await;
    let instance = meet_url
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .to_string();
    session.set_authenticated(user.clone(), cookie, instance);
    Ok(serde_json::json!({
        "display_name": user.display_name(),
        "email": user.email,
    }))
}

#[tauri::command]
async fn logout_session(
    state: tauri::State<'_, VisioState>,
    meet_url: String,
) -> Result<(), String> {
    let mut session = state.session.lock().await;
    session.logout(&meet_url).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn create_room(
    state: tauri::State<'_, VisioState>,
    meet_url: String,
    name: String,
    access_level: String,
) -> Result<serde_json::Value, String> {
    let session = state.session.lock().await;
    let cookie = session.cookie().ok_or("Not authenticated")?;
    drop(session);

    let result = visio_core::SessionManager::create_room(&meet_url, &cookie, &name, &access_level)
        .await
        .map_err(|e| e.to_string())?;

    let (livekit_url, livekit_token) = match result.livekit {
        Some(lk) => (
            lk.url
                .replace("https://", "wss://")
                .replace("http://", "ws://"),
            lk.token,
        ),
        None => (String::new(), String::new()),
    };

    Ok(serde_json::json!({
        "id": result.id,
        "slug": result.slug,
        "name": result.name,
        "access_level": result.access_level,
        "livekit_url": livekit_url,
        "livekit_token": livekit_token,
    }))
}

#[tauri::command]
async fn get_session_state(
    state: tauri::State<'_, VisioState>,
) -> Result<serde_json::Value, String> {
    let session = state.session.lock().await;
    match session.state() {
        SessionState::Anonymous => Ok(serde_json::json!({ "state": "anonymous" })),
        SessionState::Authenticated {
            user,
            meet_instance,
            ..
        } => Ok(serde_json::json!({
            "state": "authenticated",
            "display_name": user.display_name(),
            "email": user.email,
            "meet_instance": meet_instance,
        })),
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "visio_core=info,visio_video=info,visio_desktop=info"
                    .parse()
                    .unwrap()
            }),
        )
        .init();

    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("io.visio.desktop");
    std::fs::create_dir_all(&data_dir).ok();
    let settings = SettingsStore::new(data_dir.to_str().unwrap());

    let room_manager = RoomManager::new();
    room_manager.set_high_quality_mode(true);
    let playout_buffer = room_manager.playout_buffer();
    let controls = room_manager.controls();
    let chat = room_manager.chat();

    let audio_playout = audio_cpal::CpalAudioPlayout::start(playout_buffer.clone())
        .expect("failed to start audio playout");

    let room_arc = Arc::new(Mutex::new(room_manager));

    // Register event listener for auto-starting video renderers
    {
        let listener = Arc::new(DesktopEventListener {
            room: room_arc.clone(),
        });
        // We need to add the listener while we can still access room_manager
        // But room_manager is now behind Arc<Mutex>. We'll do it via block_on.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let rm = room_arc.lock().await;
            rm.add_listener(listener);
        });
        // Drop the temp runtime — Tauri will create its own
        drop(rt);
    }

    let state = VisioState {
        room: room_arc,
        controls: Arc::new(Mutex::new(controls)),
        chat: Arc::new(Mutex::new(chat)),
        session: Mutex::new(SessionManager::new()),
        settings,
        #[cfg(target_os = "macos")]
        camera_capture: std::sync::Mutex::new(None),
        audio_playout: std::sync::Mutex::new(audio_playout),
        playout_buffer,
        audio_capture: std::sync::Mutex::new(None),
        screen_capture: std::sync::Mutex::new(None),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .manage(state)
        .setup(|app| {
            // Store handle globally for the C video callback
            let _ = APP_HANDLE.set(app.handle().clone());

            // Store handle for audio error event emission
            audio_cpal::set_app_handle(app.handle().clone());

            // Register the desktop video frame callback
            unsafe {
                visio_video::visio_video_set_desktop_callback(
                    on_desktop_frame,
                    std::ptr::null_mut(),
                );
            }

            tracing::info!("Visio desktop app started, video callback registered");

            // Log deep link events on the Rust side
            app.listen("deep-link://new-url", |event: tauri::Event| {
                tracing::info!("Deep link received (Rust): {:?}", event.payload());
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                tracing::info!("window close requested, disconnecting gracefully");
                let state: tauri::State<'_, VisioState> = window.state();
                let room = state.room.clone();
                // Stop audio/camera capture before disconnect
                {
                    let mut cap = state
                        .audio_capture
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if let Some(capture) = cap.take() {
                        capture.stop();
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    let mut cam = state
                        .camera_capture
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if let Some(mut capture) = cam.take() {
                        capture.stop();
                    }
                }
                {
                    let mut sc = state
                        .screen_capture
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if let Some(mut capture) = sc.take() {
                        capture.stop();
                    }
                }
                // Gracefully disconnect the room
                tauri::async_runtime::block_on(async {
                    let rm = room.lock().await;
                    rm.disconnect().await;
                });
                tracing::info!("graceful disconnect complete");
            }
        })
        .invoke_handler(tauri::generate_handler![
            validate_room,
            connect,
            disconnect,
            get_connection_state,
            get_participants,
            get_local_participant,
            get_video_tracks,
            toggle_mic,
            toggle_camera,
            send_chat,
            get_messages,
            get_translations,
            get_system_language,
            get_settings,
            set_display_name,
            set_language,
            set_mic_enabled_on_join,
            set_camera_enabled_on_join,
            set_theme,
            get_meet_instances,
            set_meet_instances,
            raise_hand,
            lower_hand,
            is_hand_raised,
            set_chat_open,
            list_waiting_participants,
            admit_participant,
            deny_participant,
            cancel_lobby,
            search_users,
            list_accesses,
            add_access,
            remove_access,
            launch_oidc,
            authenticate,
            logout_session,
            create_room,
            get_session_state,
            send_reaction,
            list_screen_sources,
            start_screen_share,
            stop_screen_share,
            set_background_mode,
            get_background_mode,
            load_blur_model,
            load_background_image,
            list_audio_input_devices,
            list_audio_output_devices,
            list_video_input_devices,
            select_audio_input,
            select_audio_output,
            select_video_input,
            set_adaptive_mode_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
