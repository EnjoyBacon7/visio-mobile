use futures_util::StreamExt;
use livekit::data_stream::StreamReader;
use livekit::participant::ConnectionQuality as LkConnectionQuality;
use livekit::prelude::{RemoteParticipant, Room, RoomEvent, RoomOptions};
use livekit::track::{RemoteVideoTrack, TrackKind as LkTrackKind, TrackSource as LkTrackSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::audio_playout::AudioPlayoutBuffer;
use crate::auth::AuthService;
use crate::chat::MessageStore;
use crate::errors::VisioError;
use crate::events::{
    ChatMessage, ConnectionQuality, ConnectionState, EventEmitter, ParticipantInfo, TrackInfo,
    TrackKind, TrackSource, VisioEvent, VisioEventListener,
};
use crate::hand_raise::HandRaiseManager;
use crate::participants::ParticipantManager;

/// Manages the lifecycle of a LiveKit room connection.
pub struct RoomManager {
    room: Arc<Mutex<Option<Arc<Room>>>>,
    emitter: EventEmitter,
    participants: Arc<Mutex<ParticipantManager>>,
    connection_state: Arc<Mutex<ConnectionState>>,
    subscribed_tracks: Arc<Mutex<HashMap<String, RemoteVideoTrack>>>,
    messages: MessageStore,
    playout_buffer: Arc<AudioPlayoutBuffer>,
    hand_raise: Arc<Mutex<Option<HandRaiseManager>>>,
    /// Shared with MeetingControls so local_participant_info() reads the
    /// authoritative camera state without depending on LiveKit publication
    /// mute-state timing.
    camera_enabled: Arc<Mutex<bool>>,
    /// Stored connection info for application-level reconnection.
    last_meet_url: Arc<Mutex<Option<String>>>,
    last_username: Arc<Mutex<Option<String>>>,
    /// Lobby (waiting room) state.
    lobby_cookie: Arc<Mutex<Option<String>>>,
    session_cookie: Arc<Mutex<Option<String>>>,
    lobby_cancel: Arc<tokio::sync::Notify>,
}

impl Default for RoomManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            room: Arc::new(Mutex::new(None)),
            emitter: EventEmitter::new(),
            participants: Arc::new(Mutex::new(ParticipantManager::new())),
            connection_state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            subscribed_tracks: Arc::new(Mutex::new(HashMap::new())),
            messages: Arc::new(Mutex::new(Vec::new())),
            playout_buffer: Arc::new(AudioPlayoutBuffer::new()),
            hand_raise: Arc::new(Mutex::new(None)),
            camera_enabled: Arc::new(Mutex::new(false)),
            last_meet_url: Arc::new(Mutex::new(None)),
            last_username: Arc::new(Mutex::new(None)),
            lobby_cookie: Arc::new(Mutex::new(None)),
            session_cookie: Arc::new(Mutex::new(None)),
            lobby_cancel: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Get a reference to the audio playout buffer.
    ///
    /// Platform audio output (Android AudioTrack, desktop cpal) pulls
    /// decoded remote audio samples from this buffer.
    pub fn playout_buffer(&self) -> Arc<AudioPlayoutBuffer> {
        self.playout_buffer.clone()
    }

    /// Register a listener for room events.
    pub fn add_listener(&self, listener: Arc<dyn VisioEventListener>) {
        self.emitter.add_listener(listener);
    }

    /// Create MeetingControls bound to this room.
    pub fn controls(&self) -> crate::controls::MeetingControls {
        crate::controls::MeetingControls::new(
            self.room.clone(),
            self.emitter.clone(),
            self.camera_enabled.clone(),
        )
    }

    /// Create a ChatService bound to this room.
    pub fn chat(&self) -> crate::chat::ChatService {
        crate::chat::ChatService::new(
            self.room.clone(),
            self.emitter.clone(),
            self.messages.clone(),
        )
    }

    /// Get current connection state.
    pub async fn connection_state(&self) -> ConnectionState {
        self.connection_state.lock().await.clone()
    }

    /// Get a snapshot of current participants.
    pub async fn participants(&self) -> Vec<ParticipantInfo> {
        let mut list = self.participants.lock().await.participants().to_vec();
        // Prepend local participant so the UI can render a self-view tile.
        if let Some(local) = self.local_participant_info().await {
            list.insert(0, local);
        }
        list
    }

    /// Get local participant info (for self-view tile).
    pub async fn local_participant_info(&self) -> Option<ParticipantInfo> {
        let room = self.room.lock().await;
        let room = room.as_ref()?;
        let local = room.local_participant();
        let name = {
            let n = local.name().to_string();
            if n.is_empty() { None } else { Some(n) }
        };
        // Use the authoritative camera_enabled flag rather than checking
        // publication mute state, which may lag behind the actual user intent
        // (pub_.mute() is async and needs server ACK before is_muted() updates).
        let has_video = *self.camera_enabled.lock().await;
        let is_muted = local
            .track_publications()
            .values()
            .any(|pub_| pub_.kind() == LkTrackKind::Audio && pub_.is_muted());
        // "local-camera" is a sentinel SID recognised by the JNI layer:
        // attachSurface stores the ANativeWindow in LOCAL_PREVIEW_SURFACE
        // and nativePushCameraFrame renders I420 frames directly to it,
        // bypassing the NativeVideoStream path used for remote tracks.
        Some(ParticipantInfo {
            sid: local.sid().to_string(),
            identity: local.identity().to_string(),
            name,
            is_muted,
            has_video,
            video_track_sid: if has_video {
                Some("local-camera".to_string())
            } else {
                None
            },
            connection_quality: ConnectionQuality::Excellent,
        })
    }

    /// Get current active speakers.
    pub async fn active_speakers(&self) -> Vec<String> {
        self.participants.lock().await.active_speakers().to_vec()
    }

    /// Get a subscribed remote video track by its SID.
    ///
    /// Returns `None` if the track is not currently subscribed.
    pub async fn get_video_track(&self, track_sid: &str) -> Option<RemoteVideoTrack> {
        self.subscribed_tracks.lock().await.get(track_sid).cloned()
    }

    /// Get all currently subscribed video track SIDs.
    pub async fn video_track_sids(&self) -> Vec<String> {
        self.subscribed_tracks
            .lock()
            .await
            .keys()
            .cloned()
            .collect()
    }

    /// Connect to a room using the Meet API.
    ///
    /// Calls the Meet API to get a token, then connects to the LiveKit room.
    pub async fn connect(
        &self,
        meet_url: &str,
        username: Option<&str>,
        session_cookie: Option<&str>,
    ) -> Result<(), VisioError> {
        // Store connection info for potential reconnection
        *self.last_meet_url.lock().await = Some(meet_url.to_string());
        *self.last_username.lock().await = username.map(|s| s.to_string());
        *self.session_cookie.lock().await = session_cookie.map(|s| s.to_string());

        self.set_connection_state(ConnectionState::Connecting).await;

        match AuthService::request_token(meet_url, username, session_cookie).await {
            Ok(token_info) => {
                self.connect_with_token(&token_info.livekit_url, &token_info.token)
                    .await?;

                // Start lobby polling for host (authenticated users)
                if session_cookie.is_some() {
                    tracing::info!("LOBBY: cookie present, starting host polling");
                    self.start_lobby_host_polling().await;
                } else {
                    tracing::info!("LOBBY: no cookie, skipping host polling");
                }

                Ok(())
            }
            Err(VisioError::Auth(ref msg)) if msg.contains("waiting for host approval") => {
                tracing::info!("room requires host approval, entering lobby");
                let name = username.unwrap_or("Anonymous");
                self.enter_lobby(meet_url, name).await
            }
            Err(e) => Err(e),
        }
    }

    /// Connect directly with a LiveKit URL and token (useful for testing).
    pub async fn connect_with_token(
        &self,
        livekit_url: &str,
        token: &str,
    ) -> Result<(), VisioError> {
        self.set_connection_state(ConnectionState::Connecting).await;

        let mut options = RoomOptions::default();
        options.auto_subscribe = true;
        options.adaptive_stream = true;
        options.dynacast = true;

        let (room, events) = Room::connect(livekit_url, token, options)
            .await
            .map_err(|e| VisioError::Connection(e.to_string()))?;

        let room = Arc::new(room);

        // Store local participant SID
        {
            let local = room.local_participant();
            let mut pm = self.participants.lock().await;
            pm.set_local_sid(local.sid().to_string());
        }

        // Seed existing remote participants
        {
            let mut pm = self.participants.lock().await;
            for (_, participant) in room.remote_participants() {
                let info = Self::remote_participant_to_info(&participant);
                pm.add_participant(info.clone());
                self.emitter.emit(VisioEvent::ParticipantJoined(info));
            }
        }

        // Store room reference
        *self.room.lock().await = Some(room.clone());

        // Initialize HandRaiseManager now that we have a room
        {
            let hm = HandRaiseManager::new(room.clone(), self.emitter.clone());
            *self.hand_raise.lock().await = Some(hm);
        }

        // Update state to connected
        self.set_connection_state(ConnectionState::Connected).await;

        // Spawn event loop
        let emitter = self.emitter.clone();
        let participants = self.participants.clone();
        let connection_state = self.connection_state.clone();
        let room_ref = self.room.clone();
        let subscribed_tracks = self.subscribed_tracks.clone();
        let messages = self.messages.clone();
        let playout_buffer = self.playout_buffer.clone();
        let hand_raise = self.hand_raise.clone();
        let last_meet_url = self.last_meet_url.clone();

        tokio::spawn(async move {
            Self::event_loop(
                events,
                emitter,
                participants,
                connection_state,
                room_ref,
                subscribed_tracks,
                messages,
                playout_buffer,
                hand_raise,
                last_meet_url,
            )
            .await;
        });

        Ok(())
    }

    /// Disconnect from the current room.
    pub async fn disconnect(&self) {
        // Cancel any in-progress lobby polling
        self.lobby_cancel.notify_one();
        *self.lobby_cookie.lock().await = None;

        // Clear reconnection info BEFORE closing — so the event loop
        // knows this disconnect is intentional.
        *self.last_meet_url.lock().await = None;
        *self.last_username.lock().await = None;

        let room = self.room.lock().await.take();
        if let Some(room) = room
            && let Err(e) = room.close().await
        {
            tracing::warn!("error closing room: {e}");
        }
        self.participants.lock().await.clear();
        self.subscribed_tracks.lock().await.clear();
        self.messages.lock().await.clear();
        self.playout_buffer.clear();
        // Clear hand raise state
        if let Some(hm) = self.hand_raise.lock().await.take() {
            hm.clear().await;
        }
        self.set_connection_state(ConnectionState::Disconnected)
            .await;
    }

    /// Raise the local participant's hand.
    pub async fn raise_hand(&self) -> Result<(), VisioError> {
        let hm = self.hand_raise.lock().await;
        hm.as_ref()
            .ok_or(VisioError::Room("not connected".into()))?
            .raise_hand()
            .await
    }

    /// Lower the local participant's hand.
    pub async fn lower_hand(&self) -> Result<(), VisioError> {
        let hm = self.hand_raise.lock().await;
        hm.as_ref()
            .ok_or(VisioError::Room("not connected".into()))?
            .lower_hand()
            .await
    }

    /// Check if the local participant's hand is currently raised.
    pub async fn is_hand_raised(&self) -> bool {
        let hm = self.hand_raise.lock().await;
        match hm.as_ref() {
            Some(hm) => hm.is_hand_raised().await,
            None => false,
        }
    }

    /// Get stored connection info for reconnection.
    pub async fn last_connection_info(&self) -> Option<(String, Option<String>)> {
        let url = self.last_meet_url.lock().await.clone();
        let username = self.last_username.lock().await.clone();
        url.map(|u| (u, username))
    }

    /// Attempt to reconnect to the last room with exponential backoff.
    ///
    /// Called by native UI when ConnectionLost is received.
    pub async fn reconnect(&self) -> Result<(), VisioError> {
        let (meet_url, username) = self
            .last_connection_info()
            .await
            .ok_or_else(|| VisioError::Connection("no previous connection info".into()))?;

        let max_attempts: u32 = 10;
        let base_delay = std::time::Duration::from_secs(1);
        let max_delay = std::time::Duration::from_secs(30);

        for attempt in 1..=max_attempts {
            self.set_connection_state(ConnectionState::Reconnecting { attempt })
                .await;

            tracing::info!("reconnection attempt {attempt}/{max_attempts}");

            match self.connect(&meet_url, username.as_deref(), None).await {
                Ok(()) => {
                    tracing::info!("reconnection successful on attempt {attempt}");
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("reconnection attempt {attempt}/{max_attempts} failed: {e}");
                    if attempt < max_attempts {
                        let delay = base_delay
                            .checked_mul(2u32.pow(attempt - 1))
                            .unwrap_or(max_delay)
                            .min(max_delay);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        // All attempts failed — clear connection info and report disconnect
        *self.last_meet_url.lock().await = None;
        *self.last_username.lock().await = None;
        self.set_connection_state(ConnectionState::Disconnected)
            .await;
        Err(VisioError::Connection(
            "reconnection failed after all attempts".into(),
        ))
    }

    /// Enter the waiting room lobby and start polling for entry approval.
    async fn enter_lobby(&self, meet_url: &str, username: &str) -> Result<(), VisioError> {
        use crate::lobby::{LobbyPollResult, LobbyService};

        let (participant_id, lobby_cookie, poll_result) =
            LobbyService::request_entry(meet_url, username).await?;

        tracing::info!(
            "lobby entry requested: participant_id={participant_id}"
        );

        *self.lobby_cookie.lock().await = Some(lobby_cookie.clone());

        match poll_result {
            LobbyPollResult::Accepted {
                livekit_url,
                token,
            } => {
                tracing::info!("immediately accepted into room");
                return self.connect_with_token(&livekit_url, &token).await;
            }
            LobbyPollResult::Denied => {
                self.emitter.emit(VisioEvent::LobbyDenied);
                self.set_connection_state(ConnectionState::Disconnected)
                    .await;
                return Err(VisioError::Auth("entry denied by host".to_string()));
            }
            LobbyPollResult::Waiting => {
                // Fall through to start polling
            }
        }

        self.set_connection_state(ConnectionState::WaitingForHost)
            .await;

        // Clone Arcs for the spawned polling task
        let meet_url = meet_url.to_string();
        let username = username.to_string();
        let lobby_cookie_arc = self.lobby_cookie.clone();
        let lobby_cancel = self.lobby_cancel.clone();
        let room = self.room.clone();
        let participants = self.participants.clone();
        let subscribed_tracks = self.subscribed_tracks.clone();
        let messages = self.messages.clone();
        let playout_buffer = self.playout_buffer.clone();
        let hand_raise = self.hand_raise.clone();
        let _camera_enabled = self.camera_enabled.clone();
        let connection_state = self.connection_state.clone();
        let emitter = self.emitter.clone();
        let last_meet_url = self.last_meet_url.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = lobby_cancel.notified() => {
                        tracing::info!("lobby polling cancelled");
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {
                        let cookie = lobby_cookie_arc.lock().await.clone().unwrap_or_default();
                        if cookie.is_empty() {
                            tracing::warn!("lobby cookie missing, stopping poll");
                            break;
                        }

                        match LobbyService::poll_entry(&meet_url, &username, &cookie).await {
                            Ok(LobbyPollResult::Accepted { livekit_url, token }) => {
                                tracing::info!("lobby entry accepted, connecting to room");
                                *connection_state.lock().await = ConnectionState::Connecting;
                                emitter.emit(VisioEvent::ConnectionStateChanged(
                                    ConnectionState::Connecting,
                                ));

                                let mut options = RoomOptions::default();
                                options.auto_subscribe = true;
                                options.adaptive_stream = true;
                                options.dynacast = true;

                                match Room::connect(&livekit_url, &token, options).await {
                                    Ok((lk_room, events)) => {
                                        let lk_room = Arc::new(lk_room);

                                        // Store local participant SID
                                        {
                                            let local = lk_room.local_participant();
                                            let mut pm = participants.lock().await;
                                            pm.set_local_sid(local.sid().to_string());
                                        }

                                        // Seed existing remote participants
                                        {
                                            let mut pm = participants.lock().await;
                                            for (_, participant) in lk_room.remote_participants() {
                                                let info = RoomManager::remote_participant_to_info(&participant);
                                                pm.add_participant(info.clone());
                                                emitter.emit(VisioEvent::ParticipantJoined(info));
                                            }
                                        }

                                        // Store room reference
                                        *room.lock().await = Some(lk_room.clone());

                                        // Initialize HandRaiseManager
                                        {
                                            let hm = HandRaiseManager::new(
                                                lk_room.clone(),
                                                emitter.clone(),
                                            );
                                            *hand_raise.lock().await = Some(hm);
                                        }

                                        // Update state to connected
                                        *connection_state.lock().await = ConnectionState::Connected;
                                        emitter.emit(VisioEvent::ConnectionStateChanged(
                                            ConnectionState::Connected,
                                        ));

                                        // Spawn event loop
                                        let ev_emitter = emitter.clone();
                                        let ev_participants = participants.clone();
                                        let ev_connection_state = connection_state.clone();
                                        let ev_room_ref = room.clone();
                                        let ev_subscribed_tracks = subscribed_tracks.clone();
                                        let ev_messages = messages.clone();
                                        let ev_playout_buffer = playout_buffer.clone();
                                        let ev_hand_raise = hand_raise.clone();
                                        let ev_last_meet_url = last_meet_url.clone();

                                        tokio::spawn(async move {
                                            RoomManager::event_loop(
                                                events,
                                                ev_emitter,
                                                ev_participants,
                                                ev_connection_state,
                                                ev_room_ref,
                                                ev_subscribed_tracks,
                                                ev_messages,
                                                ev_playout_buffer,
                                                ev_hand_raise,
                                                ev_last_meet_url,
                                            )
                                            .await;
                                        });
                                    }
                                    Err(e) => {
                                        tracing::error!("failed to connect after lobby acceptance: {e}");
                                        *connection_state.lock().await = ConnectionState::Disconnected;
                                        emitter.emit(VisioEvent::ConnectionStateChanged(
                                            ConnectionState::Disconnected,
                                        ));
                                    }
                                }
                                break;
                            }
                            Ok(LobbyPollResult::Denied) => {
                                tracing::info!("lobby entry denied by host");
                                emitter.emit(VisioEvent::LobbyDenied);
                                *connection_state.lock().await = ConnectionState::Disconnected;
                                emitter.emit(VisioEvent::ConnectionStateChanged(
                                    ConnectionState::Disconnected,
                                ));
                                break;
                            }
                            Ok(LobbyPollResult::Waiting) => {
                                tracing::debug!("still waiting in lobby...");
                            }
                            Err(e) => {
                                tracing::warn!("lobby poll error (will retry): {e}");
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// List participants currently waiting in the lobby (host only).
    pub async fn list_waiting_participants(
        &self,
    ) -> Result<Vec<crate::lobby::WaitingParticipant>, VisioError> {
        let meet_url = self
            .last_meet_url
            .lock()
            .await
            .clone()
            .ok_or_else(|| VisioError::Room("not connected".to_string()))?;
        let cookie = self
            .session_cookie
            .lock()
            .await
            .clone()
            .ok_or_else(|| VisioError::Room("not authenticated".to_string()))?;
        crate::lobby::LobbyService::list_waiting(&meet_url, &cookie).await
    }

    /// Allow or deny a waiting participant (host only).
    pub async fn handle_lobby_entry(
        &self,
        participant_id: &str,
        allow: bool,
    ) -> Result<(), VisioError> {
        let meet_url = self
            .last_meet_url
            .lock()
            .await
            .clone()
            .ok_or_else(|| VisioError::Room("not connected".to_string()))?;
        let cookie = self
            .session_cookie
            .lock()
            .await
            .clone()
            .ok_or_else(|| VisioError::Room("not authenticated".to_string()))?;
        crate::lobby::LobbyService::handle_entry(&meet_url, &cookie, participant_id, allow).await
    }

    /// Cancel lobby polling and clear lobby state.
    pub async fn cancel_lobby(&self) {
        self.lobby_cancel.notify_one();
        *self.lobby_cookie.lock().await = None;
    }

    /// Start polling the Meet API for waiting lobby participants (host side).
    /// Emits LobbyParticipantJoined/Left events when changes are detected.
    async fn start_lobby_host_polling(&self) {
        let meet_url = self.last_meet_url.lock().await.clone();
        let cookie = self.session_cookie.lock().await.clone();

        let (meet_url, cookie) = match (meet_url, cookie) {
            (Some(u), Some(c)) => (u, c),
            _ => return, // Not authenticated, skip
        };

        let emitter = self.emitter.clone();
        // Use the room reference to detect disconnection instead of lobby_cancel
        // (lobby_cancel is shared with the guest lobby polling and may already be notified)
        let connection_state = self.connection_state.clone();

        tracing::info!("starting lobby host polling for {}", meet_url);

        tokio::spawn(async move {
            use std::collections::HashSet;
            let mut known_ids: HashSet<String> = HashSet::new();

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                // Stop polling if disconnected
                let state = connection_state.lock().await.clone();
                if matches!(state, ConnectionState::Disconnected) {
                    tracing::info!("lobby host polling stopped (disconnected)");
                    break;
                }

                match crate::lobby::LobbyService::list_waiting(&meet_url, &cookie).await {
                    Ok(participants) => {
                        let current_ids: HashSet<String> =
                            participants.iter().map(|p| p.id.clone()).collect();

                        // Detect new participants
                        for p in &participants {
                            if !known_ids.contains(&p.id) {
                                tracing::info!("lobby: new waiting participant: {} ({})", p.username, p.id);
                                emitter.emit(VisioEvent::LobbyParticipantJoined {
                                    id: p.id.clone(),
                                    username: p.username.clone(),
                                });
                            }
                        }

                        // Detect departed participants
                        for id in &known_ids {
                            if !current_ids.contains(id) {
                                tracing::info!("lobby: participant left: {}", id);
                                emitter.emit(VisioEvent::LobbyParticipantLeft {
                                    id: id.clone(),
                                });
                            }
                        }

                        known_ids = current_ids;
                    }
                    Err(e) => {
                        tracing::debug!("lobby host poll error (will retry): {e}");
                    }
                }
            }
        });
    }

    async fn set_connection_state(&self, state: ConnectionState) {
        *self.connection_state.lock().await = state.clone();
        self.emitter.emit(VisioEvent::ConnectionStateChanged(state));
    }

    fn lk_source_to_visio(source: LkTrackSource) -> TrackSource {
        match source {
            LkTrackSource::Microphone => TrackSource::Microphone,
            LkTrackSource::Camera => TrackSource::Camera,
            LkTrackSource::Screenshare => TrackSource::ScreenShare,
            _ => TrackSource::Unknown,
        }
    }

    fn remote_participant_to_info(p: &RemoteParticipant) -> ParticipantInfo {
        let name = {
            let n = p.name().to_string();
            if n.is_empty() { None } else { Some(n) }
        };

        // Only use publication metadata for audio mute state.
        // Video state (has_video / video_track_sid) is set exclusively by
        // TrackSubscribed events to avoid a race where the UI creates a
        // VideoSurfaceView before the track is actually subscribed, leading
        // to a permanent black tile (attachSurface finds no track in the
        // subscribed_tracks registry).
        let is_muted = p
            .track_publications()
            .values()
            .any(|pub_| pub_.kind() == LkTrackKind::Audio && pub_.is_muted());

        ParticipantInfo {
            sid: p.sid().to_string(),
            identity: p.identity().to_string(),
            name,
            is_muted,
            has_video: false,
            video_track_sid: None,
            connection_quality: ConnectionQuality::Good,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn event_loop(
        mut events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
        emitter: EventEmitter,
        participants: Arc<Mutex<ParticipantManager>>,
        connection_state: Arc<Mutex<ConnectionState>>,
        room_ref: Arc<Mutex<Option<Arc<Room>>>>,
        subscribed_tracks: Arc<Mutex<HashMap<String, RemoteVideoTrack>>>,
        messages: MessageStore,
        playout_buffer: Arc<AudioPlayoutBuffer>,
        hand_raise: Arc<Mutex<Option<HandRaiseManager>>>,
        last_meet_url: Arc<Mutex<Option<String>>>,
    ) {
        let mut reconnect_attempt: u32 = 0;
        // Track active audio stream tasks so they get cancelled on disconnect
        let mut audio_stream_tasks: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

        while let Some(event) = events.recv().await {
            match event {
                RoomEvent::Connected { .. } => {
                    reconnect_attempt = 0;
                    *connection_state.lock().await = ConnectionState::Connected;
                    emitter.emit(VisioEvent::ConnectionStateChanged(
                        ConnectionState::Connected,
                    ));
                }

                RoomEvent::Reconnecting => {
                    reconnect_attempt += 1;
                    let state = ConnectionState::Reconnecting {
                        attempt: reconnect_attempt,
                    };
                    *connection_state.lock().await = state.clone();
                    emitter.emit(VisioEvent::ConnectionStateChanged(state));
                }

                RoomEvent::Reconnected => {
                    reconnect_attempt = 0;
                    *connection_state.lock().await = ConnectionState::Connected;
                    emitter.emit(VisioEvent::ConnectionStateChanged(
                        ConnectionState::Connected,
                    ));
                }

                RoomEvent::Disconnected { reason } => {
                    tracing::info!("room disconnected: {reason:?}");

                    // Check if this was an intentional disconnect (disconnect()
                    // clears last_meet_url before closing the room).
                    let is_intentional = last_meet_url.lock().await.is_none();

                    *connection_state.lock().await = ConnectionState::Disconnected;
                    participants.lock().await.clear();
                    subscribed_tracks.lock().await.clear();
                    messages.lock().await.clear();
                    playout_buffer.clear();
                    if let Some(hm) = hand_raise.lock().await.take() {
                        hm.clear().await;
                    }
                    for (sid, handle) in audio_stream_tasks.drain() {
                        handle.abort();
                        tracing::info!("audio playout stream aborted on disconnect: {sid}");
                    }
                    *room_ref.lock().await = None;

                    if is_intentional {
                        emitter.emit(VisioEvent::ConnectionStateChanged(
                            ConnectionState::Disconnected,
                        ));
                    } else {
                        // Network loss — emit ConnectionLost so native UI
                        // can trigger reconnect().
                        emitter.emit(VisioEvent::ConnectionLost);
                    }
                    break;
                }

                RoomEvent::ParticipantConnected(participant) => {
                    let info = Self::remote_participant_to_info(&participant);
                    participants.lock().await.add_participant(info.clone());
                    emitter.emit(VisioEvent::ParticipantJoined(info));
                }

                RoomEvent::ParticipantDisconnected(participant) => {
                    let sid = participant.sid().to_string();
                    participants.lock().await.remove_participant(&sid);
                    emitter.emit(VisioEvent::ParticipantLeft(sid));
                }

                RoomEvent::TrackSubscribed {
                    track,
                    publication,
                    participant,
                } => {
                    let source = Self::lk_source_to_visio(publication.source());
                    let track_kind = match publication.kind() {
                        LkTrackKind::Audio => TrackKind::Audio,
                        LkTrackKind::Video => TrackKind::Video,
                    };

                    let psid = participant.sid().to_string();
                    let track_sid = track.sid().to_string();

                    {
                        let mut pm = participants.lock().await;
                        if let Some(p) = pm.participant_mut(&psid)
                            && track_kind == TrackKind::Video
                        {
                            p.has_video = true;
                            p.video_track_sid = Some(track_sid.clone());
                        }
                    }

                    // Store video tracks in the registry for later retrieval
                    if track_kind == TrackKind::Video
                        && let livekit::track::RemoteTrack::Video(video_track) = &track
                    {
                        subscribed_tracks
                            .lock()
                            .await
                            .insert(track_sid.clone(), video_track.clone());
                    }

                    // Start audio playout: create NativeAudioStream and feed
                    // decoded PCM frames into the shared playout buffer.
                    if track_kind == TrackKind::Audio
                        && let livekit::track::RemoteTrack::Audio(audio_track) = &track
                    {
                        let rtc_track = audio_track.rtc_track();
                        let mut audio_stream = NativeAudioStream::new(
                            rtc_track, 48_000, // sample rate
                            1,      // mono
                        );
                        let buf = playout_buffer.clone();
                        let sid = track_sid.clone();
                        let handle = tokio::spawn(async move {
                            tracing::info!("audio playout stream started for track {sid}");
                            while let Some(frame) = audio_stream.next().await {
                                buf.push_samples(&frame.data);
                            }
                            tracing::info!("audio playout stream ended for track {sid}");
                        });
                        audio_stream_tasks.insert(track_sid.clone(), handle);
                    }

                    let info = TrackInfo {
                        sid: track_sid,
                        participant_sid: psid,
                        kind: track_kind,
                        source,
                    };
                    emitter.emit(VisioEvent::TrackSubscribed(info));
                }

                RoomEvent::TrackUnsubscribed {
                    track,
                    publication,
                    participant,
                } => {
                    let psid = participant.sid().to_string();
                    let track_sid = track.sid().to_string();
                    let is_video = publication.kind() == LkTrackKind::Video;
                    let is_audio = publication.kind() == LkTrackKind::Audio;

                    if is_video {
                        let mut pm = participants.lock().await;
                        if let Some(p) = pm.participant_mut(&psid) {
                            p.has_video = false;
                            p.video_track_sid = None;
                        }
                        subscribed_tracks.lock().await.remove(&track_sid);
                    }

                    if is_audio && let Some(handle) = audio_stream_tasks.remove(&track_sid) {
                        handle.abort();
                        tracing::info!("audio playout stream aborted for track {track_sid}");
                    }

                    emitter.emit(VisioEvent::TrackUnsubscribed(track_sid));
                }

                RoomEvent::TrackMuted {
                    participant,
                    publication,
                } => {
                    let psid = participant.sid().to_string();
                    let source = Self::lk_source_to_visio(publication.source());

                    let mut pm = participants.lock().await;
                    if let Some(p) = pm.participant_mut(&psid) {
                        match source {
                            TrackSource::Microphone => p.is_muted = true,
                            TrackSource::Camera => {
                                p.has_video = false;
                                p.video_track_sid = None;
                            }
                            _ => {}
                        }
                    }
                    drop(pm);

                    emitter.emit(VisioEvent::TrackMuted {
                        participant_sid: psid,
                        source,
                    });
                }

                RoomEvent::TrackUnmuted {
                    participant,
                    publication,
                } => {
                    let psid = participant.sid().to_string();
                    let source = Self::lk_source_to_visio(publication.source());
                    let track_sid = publication.sid().to_string();

                    let mut pm = participants.lock().await;
                    if let Some(p) = pm.participant_mut(&psid) {
                        match source {
                            TrackSource::Microphone => p.is_muted = false,
                            TrackSource::Camera => {
                                p.has_video = true;
                                p.video_track_sid = Some(track_sid);
                            }
                            _ => {}
                        }
                    }
                    drop(pm);

                    emitter.emit(VisioEvent::TrackUnmuted {
                        participant_sid: psid,
                        source,
                    });
                }

                RoomEvent::ActiveSpeakersChanged { speakers } => {
                    let sids: Vec<String> = speakers.iter().map(|p| p.sid().to_string()).collect();
                    participants.lock().await.set_active_speakers(sids.clone());
                    // Auto-lower hand if local participant is speaking with hand raised
                    if let Some(hm) = hand_raise.lock().await.as_ref() {
                        hm.start_auto_lower(sids.clone());
                    }
                    emitter.emit(VisioEvent::ActiveSpeakersChanged(sids));
                }

                RoomEvent::ParticipantAttributesChanged {
                    participant,
                    changed_attributes,
                } => {
                    let psid = participant.sid().to_string();
                    if let Some(hm) = hand_raise.lock().await.as_ref() {
                        hm.handle_participant_attributes(psid, &changed_attributes)
                            .await;
                    }
                }

                RoomEvent::ConnectionQualityChanged {
                    quality,
                    participant,
                } => {
                    let psid = participant.sid().to_string();
                    let q = match quality {
                        LkConnectionQuality::Excellent => ConnectionQuality::Excellent,
                        LkConnectionQuality::Good => ConnectionQuality::Good,
                        LkConnectionQuality::Poor => ConnectionQuality::Poor,
                        LkConnectionQuality::Lost => ConnectionQuality::Lost,
                    };

                    {
                        let mut pm = participants.lock().await;
                        if let Some(p) = pm.participant_mut(&psid) {
                            p.connection_quality = q.clone();
                        }
                    }

                    emitter.emit(VisioEvent::ConnectionQualityChanged {
                        participant_sid: psid,
                        quality: q,
                    });
                }

                RoomEvent::ChatMessage {
                    message,
                    participant,
                    ..
                } => {
                    tracing::info!(
                        "ChatMessage received: id={} text={}",
                        message.id,
                        message.message
                    );
                    let sender_sid = participant
                        .as_ref()
                        .map(|p| p.sid().to_string())
                        .unwrap_or_default();
                    let sender_name = participant
                        .as_ref()
                        .map(|p| p.name().to_string())
                        .unwrap_or_default();

                    let msg = ChatMessage {
                        id: message.id,
                        sender_sid,
                        sender_name,
                        text: message.message,
                        timestamp_ms: message.timestamp as u64,
                    };
                    messages.lock().await.push(msg.clone());
                    emitter.emit(VisioEvent::ChatMessageReceived(msg));
                }

                RoomEvent::TextStreamOpened {
                    reader,
                    topic,
                    participant_identity,
                } => {
                    if topic == "lk.chat" {
                        let messages = messages.clone();
                        let emitter = emitter.clone();
                        let room_ref = room_ref.clone();
                        let identity = participant_identity.to_string();

                        tokio::spawn(async move {
                            let reader = reader.take();
                            if reader.is_none() {
                                tracing::warn!("TextStreamOpened: reader already taken");
                                return;
                            }
                            let reader = reader.unwrap();
                            let stream_id = reader.info().id.clone();
                            let timestamp_ms = reader.info().timestamp.timestamp_millis() as u64;

                            match reader.read_all().await {
                                Ok(text) => {
                                    // Look up participant name from room
                                    let sender_name = {
                                        let room = room_ref.lock().await;
                                        room.as_ref()
                                            .and_then(|r| {
                                                r.remote_participants()
                                                    .values()
                                                    .find(|p| p.identity().to_string() == identity)
                                                    .map(|p| p.name().to_string())
                                            })
                                            .unwrap_or_else(|| identity.clone())
                                    };

                                    let msg = ChatMessage {
                                        id: stream_id,
                                        sender_sid: identity,
                                        sender_name,
                                        text,
                                        timestamp_ms,
                                    };
                                    tracing::info!(
                                        "Chat via TextStream: from={} text={}",
                                        msg.sender_name,
                                        msg.text
                                    );
                                    messages.lock().await.push(msg.clone());
                                    emitter.emit(VisioEvent::ChatMessageReceived(msg));
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to read chat text stream: {e}");
                                }
                            }
                        });
                    } else {
                        tracing::debug!("TextStreamOpened: topic={topic} (ignored)");
                    }
                }

                RoomEvent::DataReceived {
                    payload,
                    topic,
                    kind,
                    participant,
                } => {
                    let psid = participant
                        .as_ref()
                        .map(|p| p.sid().to_string())
                        .unwrap_or_default();
                    let topic_str = topic.as_deref().unwrap_or("none");
                    tracing::debug!(
                        "DataReceived: from={psid} topic={topic_str} kind={kind:?} len={}",
                        payload.len()
                    );

                    // Handle lobby/waiting room data channel notifications
                    if topic_str.contains("lobby") || topic_str.contains("waiting") {
                        if let Ok(text) = std::str::from_utf8(&payload) {
                            tracing::info!("lobby notification received: {}", text);
                            if let Ok(data) = serde_json::from_str::<serde_json::Value>(text) {
                                let id = data
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let username = data
                                    .get("username")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Unknown")
                                    .to_string();
                                emitter.emit(VisioEvent::LobbyParticipantJoined { id, username });
                            }
                        }
                        continue;
                    }

                    // Legacy fallback: chat messages via DataReceived with topic "lk-chat-topic"
                    // New clients send both Stream + legacy; "ignoreLegacy" flag means
                    // the TextStreamOpened handler already processed it.
                    if topic_str == "lk-chat-topic"
                        && let Ok(text) = std::str::from_utf8(&payload)
                        && let Ok(json) = serde_json::from_str::<serde_json::Value>(text)
                    {
                        // Skip if sender uses Stream API (we handle it in TextStreamOpened)
                        if json["ignoreLegacy"].as_bool() == Some(true) {
                            tracing::debug!("Skipping legacy DataReceived (ignoreLegacy=true)");
                            continue;
                        }

                        let sender_name = participant
                            .as_ref()
                            .map(|p| p.name().to_string())
                            .unwrap_or_default();

                        let msg = ChatMessage {
                            id: json["id"].as_str().unwrap_or("").to_string(),
                            sender_sid: psid.clone(),
                            sender_name,
                            text: json["message"].as_str().unwrap_or("").to_string(),
                            timestamp_ms: json["timestamp"].as_u64().unwrap_or(0),
                        };

                        if !msg.text.is_empty() {
                            tracing::info!("Chat via DataReceived: from={psid} text={}", msg.text);
                            messages.lock().await.push(msg.clone());
                            emitter.emit(VisioEvent::ChatMessageReceived(msg));
                        }
                    }
                }

                _ => {
                    tracing::debug!("unhandled room event: {event:?}");
                }
            }
        }

        tracing::info!("room event loop ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_participant_info_returns_none_when_disconnected() {
        let rm = RoomManager::new();
        // No room connected, so local_participant_info returns None
        assert!(rm.local_participant_info().await.is_none());
    }

    #[tokio::test]
    async fn camera_enabled_shared_with_controls() {
        let rm = RoomManager::new();
        let controls = rm.controls();

        // Default: camera disabled
        assert!(!controls.is_camera_enabled().await);

        // Modify camera_enabled via the shared Arc inside RoomManager
        *rm.camera_enabled.lock().await = true;

        // Controls should see the updated value
        assert!(controls.is_camera_enabled().await);
    }

    #[tokio::test]
    async fn initial_connection_state_is_disconnected() {
        let rm = RoomManager::new();
        assert_eq!(rm.connection_state().await, ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn participants_empty_when_disconnected() {
        let rm = RoomManager::new();
        // No room means no local participant, no remote participants
        let participants = rm.participants().await;
        assert!(participants.is_empty());
    }
}
