//! Integration tests against a local LiveKit dev server.
//!
//! These tests require `livekit-server --dev` to be running locally.
//! In CI, the server is started automatically before this test suite.
//!
//! Run manually:
//! ```sh
//! docker run -d --rm --name livekit-e2e -p 7880:7880 -p 7881:7881 livekit/livekit-server --dev
//! LIVEKIT_URL=ws://localhost:7880 LIVEKIT_API_KEY=devkey LIVEKIT_API_SECRET=secret \
//!   cargo test -p visio-core --test integration_livekit
//! ```

use std::sync::Arc;
use std::time::Duration;

use livekit_api::access_token::{AccessToken, VideoGrants};
use visio_core::{ConnectionState, RoomManager, TrackSource, VisioEvent, VisioEventListener};

fn livekit_url() -> String {
    std::env::var("LIVEKIT_URL").unwrap_or_else(|_| "ws://localhost:7880".to_string())
}

fn api_key() -> String {
    std::env::var("LIVEKIT_API_KEY").unwrap_or_else(|_| "devkey".to_string())
}

fn api_secret() -> String {
    std::env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| "secret".to_string())
}

fn make_token(identity: &str, name: &str, room: &str) -> String {
    AccessToken::with_api_key(&api_key(), &api_secret())
        .with_identity(identity)
        .with_name(name)
        .with_grants(VideoGrants {
            room_join: true,
            room: room.to_string(),
            ..Default::default()
        })
        .to_jwt()
        .expect("failed to generate token")
}

/// Listener that captures all events for assertions.
struct EventCapture {
    events: std::sync::Mutex<Vec<VisioEvent>>,
}

impl EventCapture {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn has<F: Fn(&VisioEvent) -> bool>(&self, predicate: F) -> bool {
        self.events.lock().unwrap().iter().any(predicate)
    }

    fn has_state(&self, state: ConnectionState) -> bool {
        self.has(|e| matches!(e, VisioEvent::ConnectionStateChanged(s) if *s == state))
    }
}

impl VisioEventListener for EventCapture {
    fn on_event(&self, event: VisioEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// Helper: wait until a condition is true, with timeout.
async fn wait_for<F: Fn() -> bool>(condition: F, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Helper: wait until participants see each other.
async fn wait_mutual_discovery(rm1: &RoomManager, rm2: &RoomManager, id1: &str, id2: &str) {
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let p1_sees_2 = rm1.participants().await.iter().any(|p| p.identity == id2);
        let p2_sees_1 = rm2.participants().await.iter().any(|p| p.identity == id1);
        if p1_sees_2 && p2_sees_1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("participants did not discover each other within {timeout:?}");
}

// ---------------------------------------------------------------------------
// Test: connect and disconnect
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connect_and_disconnect() {
    let room_name = format!("test-connect-{}", uuid::Uuid::new_v4());
    let token = make_token("user-1", "User 1", &room_name);

    let rm = RoomManager::new();
    let capture = EventCapture::new();
    rm.add_listener(capture.clone());

    rm.connect_with_token(&livekit_url(), &token)
        .await
        .expect("connect failed");

    assert_eq!(rm.connection_state().await, ConnectionState::Connected);

    rm.disconnect().await;

    let saw_disconnected =
        wait_for(|| capture.has_state(ConnectionState::Disconnected), Duration::from_secs(5)).await;
    assert!(saw_disconnected, "should have seen Disconnected event");
}

// ---------------------------------------------------------------------------
// Test: two participants see each other
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_two_participants_see_each_other() {
    let room_name = format!("test-2p-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let token2 = make_token("bob", "Bob", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();

    rm1.connect_with_token(&url, &token1)
        .await
        .expect("connect rm1");
    rm2.connect_with_token(&url, &token2)
        .await
        .expect("connect rm2");

    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    let p1 = rm1.participants().await;
    let p2 = rm2.participants().await;
    assert!(p1.iter().any(|p| p.identity == "bob"), "rm1 should see bob");
    assert!(
        p2.iter().any(|p| p.identity == "alice"),
        "rm2 should see alice"
    );

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: mute/unmute propagation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mute_unmute_propagation() {
    let room_name = format!("test-mute-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let token2 = make_token("bob", "Bob", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let capture2 = EventCapture::new();
    rm2.add_listener(capture2.clone());
    let controls1 = rm1.controls();

    rm1.connect_with_token(&url, &token1)
        .await
        .expect("connect rm1");
    rm2.connect_with_token(&url, &token2)
        .await
        .expect("connect rm2");

    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    // Alice publishes mic then mutes it
    if let Ok(_) = controls1.publish_microphone().await {
        // Wait for track to propagate to Bob
        let saw_track = wait_for(
            || {
                capture2.has(|e| {
                    matches!(e, VisioEvent::TrackSubscribed(info) if info.source == TrackSource::Microphone)
                })
            },
            Duration::from_secs(5),
        )
        .await;
        assert!(saw_track, "bob should receive TrackSubscribed for mic");

        let _ = controls1.set_microphone_enabled(false).await;

        // Wait for TrackMuted event on Bob's side
        let saw_mute = wait_for(
            || {
                capture2.has(|e| {
                    matches!(e, VisioEvent::TrackMuted { source, .. } if *source == TrackSource::Microphone)
                })
            },
            Duration::from_secs(5),
        )
        .await;
        assert!(saw_mute, "bob should receive TrackMuted event");

        // Bob's participant list should show Alice as muted
        let p2 = rm2.participants().await;
        if let Some(alice) = p2.iter().find(|p| p.identity == "alice") {
            assert!(alice.is_muted, "alice should be muted from bob's perspective");
        }
    }

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: participant left event fires when remote disconnects
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_participant_left_event() {
    let room_name = format!("test-left-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let token2 = make_token("bob", "Bob", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let capture1 = EventCapture::new();
    rm1.add_listener(capture1.clone());

    rm1.connect_with_token(&url, &token1).await.expect("connect rm1");
    rm2.connect_with_token(&url, &token2).await.expect("connect rm2");

    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    // Bob disconnects
    rm2.disconnect().await;

    // Alice should receive ParticipantLeft for Bob
    let saw_left = wait_for(
        || capture1.has(|e| matches!(e, VisioEvent::ParticipantLeft(_))),
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_left, "alice should receive ParticipantLeft when bob disconnects");

    // Alice's participant list should no longer contain Bob
    let p1 = rm1.participants().await;
    assert!(
        !p1.iter().any(|p| p.identity == "bob"),
        "bob should be removed from participant list"
    );

    rm1.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: chat message delivery between two participants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_message_delivery() {
    let room_name = format!("test-chat-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let token2 = make_token("bob", "Bob", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let capture2 = EventCapture::new();
    rm2.add_listener(capture2.clone());

    rm1.connect_with_token(&url, &token1).await.expect("connect rm1");
    rm2.connect_with_token(&url, &token2).await.expect("connect rm2");

    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    // Alice sends a chat message
    let chat1 = rm1.chat();
    let msg = chat1
        .send_message("Hello from Alice!")
        .await
        .expect("send_message failed");
    assert_eq!(msg.text, "Hello from Alice!");

    // Bob should receive the message via event
    let saw_chat = wait_for(
        || {
            capture2.has(|e| {
                matches!(e, VisioEvent::ChatMessageReceived(m) if m.text == "Hello from Alice!")
            })
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_chat, "bob should receive ChatMessageReceived event");

    // Bob's chat service should also have the message stored
    let chat2 = rm2.chat();
    let messages = chat2.messages().await;
    assert!(
        messages.iter().any(|m| m.text == "Hello from Alice!"),
        "bob's message store should contain alice's message, got: {messages:?}"
    );

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: audio track subscription event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_audio_track_subscription() {
    let room_name = format!("test-track-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let token2 = make_token("bob", "Bob", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let capture2 = EventCapture::new();
    rm2.add_listener(capture2.clone());
    let controls1 = rm1.controls();

    rm1.connect_with_token(&url, &token1).await.expect("connect rm1");
    rm2.connect_with_token(&url, &token2).await.expect("connect rm2");

    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    // Alice publishes microphone
    controls1
        .publish_microphone()
        .await
        .expect("publish_microphone failed");

    // Bob should receive TrackSubscribed event
    let saw_track = wait_for(
        || {
            capture2.has(|e| {
                matches!(e, VisioEvent::TrackSubscribed(info)
                    if info.source == TrackSource::Microphone)
            })
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(
        saw_track,
        "bob should receive TrackSubscribed for alice's microphone"
    );

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: track unsubscribed on disconnect
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_track_unsubscribed_on_disconnect() {
    let room_name = format!("test-unsub-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let token2 = make_token("bob", "Bob", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let capture2 = EventCapture::new();
    rm2.add_listener(capture2.clone());
    let controls1 = rm1.controls();

    rm1.connect_with_token(&url, &token1).await.expect("connect rm1");
    rm2.connect_with_token(&url, &token2).await.expect("connect rm2");

    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    // Alice publishes microphone
    controls1
        .publish_microphone()
        .await
        .expect("publish_microphone failed");

    // Wait for Bob to subscribe
    let saw_sub = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackSubscribed(_))),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_sub, "bob should subscribe to alice's track");

    // Alice disconnects — Bob should get TrackUnsubscribed
    rm1.disconnect().await;

    let saw_unsub = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackUnsubscribed(_))),
        Duration::from_secs(10),
    )
    .await;
    assert!(
        saw_unsub,
        "bob should receive TrackUnsubscribed when alice disconnects"
    );

    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: connection state transitions are correct
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connection_state_lifecycle() {
    let room_name = format!("test-lifecycle-{}", uuid::Uuid::new_v4());
    let token = make_token("user-1", "User 1", &room_name);

    let rm = RoomManager::new();
    let capture = EventCapture::new();
    rm.add_listener(capture.clone());

    assert_eq!(rm.connection_state().await, ConnectionState::Disconnected);

    rm.connect_with_token(&livekit_url(), &token)
        .await
        .expect("connect failed");

    // Should have transitioned through Connecting → Connected
    let saw_connecting = wait_for(
        || capture.has_state(ConnectionState::Connecting),
        Duration::from_secs(1),
    )
    .await;
    assert!(saw_connecting, "should see Connecting state");
    assert!(
        capture.has_state(ConnectionState::Connected),
        "should see Connected state"
    );

    rm.disconnect().await;

    let saw_disconnected = wait_for(
        || capture.has_state(ConnectionState::Disconnected),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_disconnected, "should see Disconnected state");

    // Verify ordering: Connecting before Connected
    let events = capture.events.lock().unwrap();
    let states: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            VisioEvent::ConnectionStateChanged(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    let connecting_pos = states.iter().position(|s| *s == ConnectionState::Connecting);
    let connected_pos = states.iter().position(|s| *s == ConnectionState::Connected);
    assert!(
        connecting_pos < connected_pos,
        "Connecting should come before Connected, got: {states:?}"
    );
}

// ---------------------------------------------------------------------------
// Test: multiple sequential connect/disconnect cycles
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reconnect_cycle() {
    let url = livekit_url();
    let rm = RoomManager::new();

    for i in 0..3 {
        let room_name = format!("test-reconnect-{}-{}", i, uuid::Uuid::new_v4());
        let token = make_token("cycler", "Cycler", &room_name);

        rm.connect_with_token(&url, &token)
            .await
            .unwrap_or_else(|e| panic!("connect cycle {i} failed: {e}"));

        assert_eq!(
            rm.connection_state().await,
            ConnectionState::Connected,
            "should be connected on cycle {i}"
        );

        rm.disconnect().await;

        let capture = EventCapture::new();
        rm.add_listener(capture.clone());
        let saw_disconnected = wait_for(
            || capture.has_state(ConnectionState::Disconnected),
            Duration::from_secs(5),
        )
        .await;

        // Also check state directly if event wasn't caught (listener added after disconnect)
        if !saw_disconnected {
            assert_eq!(
                rm.connection_state().await,
                ConnectionState::Disconnected,
                "should be disconnected after cycle {i}"
            );
        }
    }
}
