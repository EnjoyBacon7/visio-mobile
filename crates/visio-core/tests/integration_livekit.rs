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
use visio_core::adaptive::{AdaptiveMode, ContextSignal};
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
            can_update_own_metadata: true,
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

    fn count<F: Fn(&VisioEvent) -> bool>(&self, predicate: F) -> usize {
        self.events.lock().unwrap().iter().filter(|e| predicate(e)).count()
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

// ===========================================================================
// BATCH 1: Reactions, Hand Raise, Chat advanced, Screen Share
// ===========================================================================

// ---------------------------------------------------------------------------
// Test: send and receive reaction emoji
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_receive_reaction() {
    let room_name = format!("test-react-{}", uuid::Uuid::new_v4());
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

    // Alice sends a thumbs up
    rm1.send_reaction("👍").await.expect("send_reaction failed");

    let saw_reaction = wait_for(
        || {
            capture2.has(|e| {
                matches!(e, VisioEvent::ReactionReceived { emoji, .. } if emoji == "👍")
            })
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_reaction, "bob should receive reaction 👍");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: reaction includes correct sender info
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reaction_sender_info() {
    let room_name = format!("test-react-info-{}", uuid::Uuid::new_v4());
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

    rm1.send_reaction("🎉").await.expect("send_reaction failed");

    let saw = wait_for(
        || {
            capture2.has(|e| {
                matches!(e, VisioEvent::ReactionReceived { participant_name, emoji, .. }
                    if participant_name == "Alice" && emoji == "🎉")
            })
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(saw, "reaction should include participant_name='Alice'");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: multiple reactions delivered in order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_multiple_reactions() {
    let room_name = format!("test-multi-react-{}", uuid::Uuid::new_v4());
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

    let emojis = ["👍", "❤️", "😂"];
    for emoji in &emojis {
        rm1.send_reaction(emoji).await.expect("send_reaction failed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Wait for all 3 to arrive
    let saw_all = wait_for(
        || capture2.count(|e| matches!(e, VisioEvent::ReactionReceived { .. })) >= 3,
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_all, "bob should receive all 3 reactions");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: raise hand
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_raise_hand() {
    let room_name = format!("test-hand-{}", uuid::Uuid::new_v4());
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

    rm1.raise_hand().await.expect("raise_hand failed");
    assert!(rm1.is_hand_raised().await, "alice should have hand raised locally");

    let saw_raised = wait_for(
        || {
            capture2.has(|e| {
                matches!(e, VisioEvent::HandRaisedChanged { raised: true, .. })
            })
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_raised, "bob should see HandRaisedChanged(raised=true)");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: raise then lower hand
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_raise_then_lower_hand() {
    let room_name = format!("test-hand-lower-{}", uuid::Uuid::new_v4());
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

    rm1.raise_hand().await.expect("raise_hand failed");

    let saw_raised = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::HandRaisedChanged { raised: true, .. })),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_raised, "bob should see hand raised");

    rm1.lower_hand().await.expect("lower_hand failed");
    assert!(!rm1.is_hand_raised().await, "alice should have hand lowered");

    let saw_lowered = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::HandRaisedChanged { raised: false, .. })),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_lowered, "bob should see hand lowered");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: chat bidirectional
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_bidirectional() {
    let room_name = format!("test-chat-bidi-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let token2 = make_token("bob", "Bob", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let capture1 = EventCapture::new();
    let capture2 = EventCapture::new();
    rm1.add_listener(capture1.clone());
    rm2.add_listener(capture2.clone());

    rm1.connect_with_token(&url, &token1).await.expect("connect rm1");
    rm2.connect_with_token(&url, &token2).await.expect("connect rm2");
    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    // Alice → Bob
    rm1.chat().send_message("Hello Bob!").await.expect("send failed");
    let saw_at_bob = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::ChatMessageReceived(m) if m.text == "Hello Bob!")),
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_at_bob, "bob should receive alice's message");

    // Bob → Alice
    rm2.chat().send_message("Hello Alice!").await.expect("send failed");
    let saw_at_alice = wait_for(
        || capture1.has(|e| matches!(e, VisioEvent::ChatMessageReceived(m) if m.text == "Hello Alice!")),
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_at_alice, "alice should receive bob's message");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: chat message store persistence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_message_store() {
    let room_name = format!("test-chat-store-{}", uuid::Uuid::new_v4());
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

    let messages = ["msg1", "msg2", "msg3"];
    for msg in &messages {
        rm1.chat().send_message(msg).await.expect("send failed");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Wait for Bob to receive all 3
    let saw_all = wait_for(
        || capture2.count(|e| matches!(e, VisioEvent::ChatMessageReceived(_))) >= 3,
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_all, "bob should receive all 3 messages");

    // Verify message store has all 3 in order
    let stored = rm2.chat().messages().await;
    assert_eq!(stored.len(), 3, "message store should have 3 messages");
    assert_eq!(stored[0].text, "msg1");
    assert_eq!(stored[1].text, "msg2");
    assert_eq!(stored[2].text, "msg3");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: unread count increments and resets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_unread_count() {
    let room_name = format!("test-unread-{}", uuid::Uuid::new_v4());
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

    // Bob's chat is closed (default)
    assert_eq!(rm2.unread_count(), 0, "initial unread should be 0");

    rm1.chat().send_message("hi1").await.expect("send failed");
    rm1.chat().send_message("hi2").await.expect("send failed");

    // Wait for both UnreadCountChanged events
    let saw_unread = wait_for(
        || rm2.unread_count() >= 2,
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_unread, "unread count should be >= 2, got {}", rm2.unread_count());

    // Open chat → unread resets to 0
    rm2.set_chat_open(true);
    assert_eq!(rm2.unread_count(), 0, "unread should reset on chat open");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: video track publish and subscribe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_video_track_publish_subscribe() {
    let room_name = format!("test-video-{}", uuid::Uuid::new_v4());
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

    let controls1 = rm1.controls();
    let _source = controls1.publish_camera().await.expect("publish_camera failed");

    // Bob should receive TrackSubscribed for Camera
    let saw_video = wait_for(
        || {
            capture2.has(|e| {
                matches!(e, VisioEvent::TrackSubscribed(info) if info.source == TrackSource::Camera)
            })
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_video, "bob should receive TrackSubscribed(Camera)");

    // video_track_sids on Bob's side should be non-empty
    let sids = rm2.video_track_sids().await;
    assert!(!sids.is_empty(), "bob should have video track SIDs");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: video track mute/unmute
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_video_track_mute_unmute() {
    let room_name = format!("test-vidmute-{}", uuid::Uuid::new_v4());
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

    let controls1 = rm1.controls();
    controls1.publish_camera().await.expect("publish_camera failed");

    // Wait for subscription
    let saw_sub = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackSubscribed(info) if info.source == TrackSource::Camera)),
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_sub, "bob should subscribe to camera");

    // Alice mutes camera
    controls1.set_camera_enabled(false).await.expect("set_camera_enabled failed");

    let saw_mute = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackMuted { source, .. } if *source == TrackSource::Camera)),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_mute, "bob should receive TrackMuted(Camera)");

    // Alice unmutes camera
    controls1.set_camera_enabled(true).await.expect("set_camera_enabled failed");

    let saw_unmute = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackUnmuted { source, .. } if *source == TrackSource::Camera)),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_unmute, "bob should receive TrackUnmuted(Camera)");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: screen share publish and subscribe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_screen_share_publish_subscribe() {
    let room_name = format!("test-screen-{}", uuid::Uuid::new_v4());
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

    let controls1 = rm1.controls();
    let _source = controls1.publish_screen_share().await.expect("publish_screen_share failed");

    // Bob should receive TrackSubscribed for ScreenShare
    let saw_screen = wait_for(
        || {
            capture2.has(|e| {
                matches!(e, VisioEvent::TrackSubscribed(info) if info.source == TrackSource::ScreenShare)
            })
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_screen, "bob should receive TrackSubscribed(ScreenShare)");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: screen share stop triggers unsubscribe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_screen_share_stop() {
    let room_name = format!("test-screen-stop-{}", uuid::Uuid::new_v4());
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

    let controls1 = rm1.controls();
    controls1.publish_screen_share().await.expect("publish_screen_share failed");

    let saw_sub = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackSubscribed(info) if info.source == TrackSource::ScreenShare)),
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_sub, "bob should subscribe to screen share");

    // Alice stops screen share
    controls1.stop_screen_share().await.expect("stop_screen_share failed");

    let saw_unsub = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackUnsubscribed(_))),
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_unsub, "bob should receive TrackUnsubscribed after screen share stops");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: simultaneous audio and video tracks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_simultaneous_audio_video_tracks() {
    let room_name = format!("test-multi-track-{}", uuid::Uuid::new_v4());
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

    let controls1 = rm1.controls();
    controls1.publish_microphone().await.expect("publish_microphone failed");
    controls1.publish_camera().await.expect("publish_camera failed");

    // Bob should receive 2 TrackSubscribed events
    let saw_both = wait_for(
        || {
            let has_mic = capture2.has(|e| matches!(e, VisioEvent::TrackSubscribed(info) if info.source == TrackSource::Microphone));
            let has_cam = capture2.has(|e| matches!(e, VisioEvent::TrackSubscribed(info) if info.source == TrackSource::Camera));
            has_mic && has_cam
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_both, "bob should receive both Microphone and Camera TrackSubscribed");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: track disable does NOT trigger unsubscribe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_track_disable_does_not_unsubscribe() {
    let room_name = format!("test-disable-{}", uuid::Uuid::new_v4());
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

    let controls1 = rm1.controls();
    controls1.publish_microphone().await.expect("publish_microphone failed");

    let saw_sub = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackSubscribed(_))),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_sub, "bob should subscribe to mic");

    // Alice mutes mic (disable, not unpublish)
    controls1.set_microphone_enabled(false).await.expect("mute failed");

    // Should get TrackMuted but NOT TrackUnsubscribed
    let saw_mute = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::TrackMuted { .. })),
        Duration::from_secs(5),
    )
    .await;
    assert!(saw_mute, "should get TrackMuted");

    // Brief pause to ensure no unsub arrives
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !capture2.has(|e| matches!(e, VisioEvent::TrackUnsubscribed(_))),
        "muting should NOT trigger TrackUnsubscribed"
    );

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ===========================================================================
// BATCH 2: Multi-participants, participant info
// ===========================================================================

// ---------------------------------------------------------------------------
// Test: three participants discover each other
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_three_participants_discovery() {
    let room_name = format!("test-3p-{}", uuid::Uuid::new_v4());
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let rm3 = RoomManager::new();

    rm1.connect_with_token(&url, &make_token("alice", "Alice", &room_name))
        .await
        .expect("connect rm1");
    rm2.connect_with_token(&url, &make_token("bob", "Bob", &room_name))
        .await
        .expect("connect rm2");
    rm3.connect_with_token(&url, &make_token("charlie", "Charlie", &room_name))
        .await
        .expect("connect rm3");

    // Wait for full mesh discovery
    let timeout = Duration::from_secs(15);
    let start = std::time::Instant::now();
    loop {
        let p1 = rm1.participants().await;
        let p2 = rm2.participants().await;
        let p3 = rm3.participants().await;

        let p1_ok = p1.iter().any(|p| p.identity == "bob") && p1.iter().any(|p| p.identity == "charlie");
        let p2_ok = p2.iter().any(|p| p.identity == "alice") && p2.iter().any(|p| p.identity == "charlie");
        let p3_ok = p3.iter().any(|p| p.identity == "alice") && p3.iter().any(|p| p.identity == "bob");

        if p1_ok && p2_ok && p3_ok {
            break;
        }
        if start.elapsed() > timeout {
            panic!("3 participants did not fully discover each other");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    rm1.disconnect().await;
    rm2.disconnect().await;
    rm3.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: ParticipantJoined event has correct name
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_participant_joined_has_correct_name() {
    let room_name = format!("test-name-{}", uuid::Uuid::new_v4());
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let capture1 = EventCapture::new();
    rm1.add_listener(capture1.clone());

    rm1.connect_with_token(&url, &make_token("alice", "Alice", &room_name))
        .await
        .expect("connect rm1");

    let rm2 = RoomManager::new();
    rm2.connect_with_token(&url, &make_token("bob", "Bob McBobface", &room_name))
        .await
        .expect("connect rm2");

    let saw_join = wait_for(
        || {
            capture1.has(|e| {
                matches!(e, VisioEvent::ParticipantJoined(info) if info.name.as_deref() == Some("Bob McBobface"))
            })
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(saw_join, "ParticipantJoined should include name 'Bob McBobface'");

    rm1.disconnect().await;
    rm2.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: chat with three participants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_three_participants() {
    let room_name = format!("test-chat3-{}", uuid::Uuid::new_v4());
    let url = livekit_url();

    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();
    let rm3 = RoomManager::new();
    let capture2 = EventCapture::new();
    let capture3 = EventCapture::new();
    rm2.add_listener(capture2.clone());
    rm3.add_listener(capture3.clone());

    rm1.connect_with_token(&url, &make_token("alice", "Alice", &room_name)).await.expect("rm1");
    rm2.connect_with_token(&url, &make_token("bob", "Bob", &room_name)).await.expect("rm2");
    rm3.connect_with_token(&url, &make_token("charlie", "Charlie", &room_name)).await.expect("rm3");

    // Wait for full mesh
    tokio::time::sleep(Duration::from_secs(3)).await;

    rm1.chat().send_message("hello everyone").await.expect("send failed");

    let bob_saw = wait_for(
        || capture2.has(|e| matches!(e, VisioEvent::ChatMessageReceived(m) if m.text == "hello everyone")),
        Duration::from_secs(10),
    )
    .await;
    let charlie_saw = wait_for(
        || capture3.has(|e| matches!(e, VisioEvent::ChatMessageReceived(m) if m.text == "hello everyone")),
        Duration::from_secs(10),
    )
    .await;

    assert!(bob_saw, "bob should receive the chat message");
    assert!(charlie_saw, "charlie should receive the chat message");

    rm1.disconnect().await;
    rm2.disconnect().await;
    rm3.disconnect().await;
}

// ===========================================================================
// BATCH 3: Robustness and edge cases
// ===========================================================================

// ---------------------------------------------------------------------------
// Test: double disconnect does not crash
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_double_disconnect() {
    let room_name = format!("test-ddisc-{}", uuid::Uuid::new_v4());
    let token = make_token("user-1", "User 1", &room_name);

    let rm = RoomManager::new();
    rm.connect_with_token(&livekit_url(), &token)
        .await
        .expect("connect failed");

    rm.disconnect().await;
    rm.disconnect().await; // should not panic or crash
}

// ---------------------------------------------------------------------------
// Test: publish microphone before connect returns error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_publish_before_connect() {
    let rm = RoomManager::new();
    let controls = rm.controls();
    let result = controls.publish_microphone().await;
    assert!(result.is_err(), "publish_microphone before connect should fail");
}

// ---------------------------------------------------------------------------
// Test: send chat before connect returns error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_send_chat_before_connect() {
    let rm = RoomManager::new();
    let chat = rm.chat();
    let result = chat.send_message("should fail").await;
    assert!(result.is_err(), "send_message before connect should fail");
}

// ---------------------------------------------------------------------------
// Test: rapid mute/unmute toggle stability
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rapid_mute_toggle() {
    let room_name = format!("test-rapid-{}", uuid::Uuid::new_v4());
    let token1 = make_token("alice", "Alice", &room_name);
    let url = livekit_url();

    let rm1 = RoomManager::new();
    rm1.connect_with_token(&url, &token1).await.expect("connect rm1");

    let controls1 = rm1.controls();
    if controls1.publish_microphone().await.is_ok() {
        // Toggle 10 times rapidly
        for _ in 0..10 {
            let _ = controls1.set_microphone_enabled(false).await;
            let _ = controls1.set_microphone_enabled(true).await;
        }
        // Final state should be enabled
        assert!(
            controls1.is_microphone_enabled().await,
            "mic should be enabled after rapid toggles"
        );
    }

    rm1.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: last_connection_info is None for connect_with_token
// (only populated by connect() with meet URL)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_last_connection_info_with_token() {
    let room_name = format!("test-lastinfo-{}", uuid::Uuid::new_v4());
    let url = livekit_url();
    let token = make_token("alice", "Alice", &room_name);

    let rm = RoomManager::new();
    let info_before = rm.last_connection_info().await;
    assert!(info_before.is_none(), "no connection info before connect");

    rm.connect_with_token(&url, &token).await.expect("connect failed");

    // connect_with_token does NOT set last_meet_url (only connect() does)
    let info = rm.last_connection_info().await;
    assert!(info.is_none(), "connect_with_token should not populate last_connection_info");

    rm.disconnect().await;
}

// ---------------------------------------------------------------------------
// Test: participants list is clean after reconnect to new room
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reconnect_clean_participants() {
    let url = livekit_url();
    let rm1 = RoomManager::new();
    let rm2 = RoomManager::new();

    // First room with 2 participants
    let room1 = format!("test-clean-1-{}", uuid::Uuid::new_v4());
    rm1.connect_with_token(&url, &make_token("alice", "Alice", &room1)).await.expect("connect");
    rm2.connect_with_token(&url, &make_token("bob", "Bob", &room1)).await.expect("connect");
    wait_mutual_discovery(&rm1, &rm2, "alice", "bob").await;

    rm1.disconnect().await;
    rm2.disconnect().await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Second room with alice alone
    let room2 = format!("test-clean-2-{}", uuid::Uuid::new_v4());
    rm1.connect_with_token(&url, &make_token("alice", "Alice", &room2)).await.expect("connect");
    tokio::time::sleep(Duration::from_secs(1)).await;

    let participants = rm1.participants().await;
    // Should only see alice (local), no leftover bob
    assert!(
        !participants.iter().any(|p| p.identity == "bob"),
        "bob should NOT be in participant list after reconnect to new room"
    );

    rm1.disconnect().await;
}

// ===========================================================================
// BATCH 4: Adaptive mode and bandwidth (unit-level but through RoomManager)
// ===========================================================================

// ---------------------------------------------------------------------------
// Test: adaptive mode defaults to Office
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_adaptive_mode_default() {
    let rm = RoomManager::new();
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Office);
}

// ---------------------------------------------------------------------------
// Test: motion signal switches to Pedestrian
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_adaptive_mode_motion_to_pedestrian() {
    let rm = RoomManager::new();
    let capture = EventCapture::new();
    rm.add_listener(capture.clone());

    let result = rm.report_context_signal(ContextSignal::MotionDetected(true));
    assert_eq!(result, Some(AdaptiveMode::Pedestrian));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Pedestrian);

    assert!(
        capture.has(|e| matches!(e, VisioEvent::AdaptiveModeChanged { mode } if *mode == AdaptiveMode::Pedestrian)),
        "should emit AdaptiveModeChanged(Pedestrian)"
    );
}

// ---------------------------------------------------------------------------
// Test: bluetooth car kit switches to Car mode
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_adaptive_mode_bluetooth_car() {
    let rm = RoomManager::new();
    let result = rm.report_context_signal(ContextSignal::BluetoothCarKit(true));
    assert_eq!(result, Some(AdaptiveMode::Car));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Car);
}

// ---------------------------------------------------------------------------
// Test: car mode takes priority over pedestrian
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_adaptive_mode_car_priority() {
    let rm = RoomManager::new();

    // First: motion → Pedestrian
    rm.report_context_signal(ContextSignal::MotionDetected(true));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Pedestrian);

    // Then: bluetooth → Car (should override)
    rm.report_context_signal(ContextSignal::BluetoothCarKit(true));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Car);
}

// ---------------------------------------------------------------------------
// Test: manual override overrides auto-detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_adaptive_mode_override() {
    let rm = RoomManager::new();
    let capture = EventCapture::new();
    rm.add_listener(capture.clone());

    // Auto-detect Pedestrian
    rm.report_context_signal(ContextSignal::MotionDetected(true));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Pedestrian);

    // Override to Car
    rm.set_adaptive_mode_override(Some(AdaptiveMode::Car));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Car);

    // Clear override → back to auto (Pedestrian)
    rm.set_adaptive_mode_override(None);
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Pedestrian);
}

// ---------------------------------------------------------------------------
// Test: motion stop returns to Office
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_adaptive_mode_motion_stop() {
    let rm = RoomManager::new();

    rm.report_context_signal(ContextSignal::MotionDetected(true));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Pedestrian);

    rm.report_context_signal(ContextSignal::MotionDetected(false));
    assert_eq!(rm.adaptive_mode(), AdaptiveMode::Office);
}

// ---------------------------------------------------------------------------
// Test: high quality mode flag
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_high_quality_mode() {
    let room_name = format!("test-hq-{}", uuid::Uuid::new_v4());
    let token = make_token("user-1", "User 1", &room_name);

    let rm = RoomManager::new();
    rm.set_high_quality_mode(true);

    rm.connect_with_token(&livekit_url(), &token)
        .await
        .expect("connect failed");

    // Should connect fine with high quality mode
    assert_eq!(rm.connection_state().await, ConnectionState::Connected);

    rm.disconnect().await;
}
