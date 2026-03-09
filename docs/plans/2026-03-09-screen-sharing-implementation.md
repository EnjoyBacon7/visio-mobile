# Screen Sharing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable viewing of shared screens on all platforms and publishing screen shares from Desktop.

**Architecture:** Extend `ParticipantInfo` with `screen_share_track_sid` and `has_screen_share` fields. Handle ScreenShare source in all track events. Add screen capture/publish on Desktop via LiveKit SDK. Update all platform UIs to show screen share as a virtual participant tile with auto-focus.

**Tech Stack:** Rust (visio-core, visio-video, visio-desktop), Kotlin/Compose (Android), SwiftUI (iOS), React/TypeScript (Desktop frontend), LiveKit Rust SDK, Tauri 2.x

---

### Task 1: Extend ParticipantInfo with screen share fields (Rust core)

**Files:**
- Modify: `crates/visio-core/src/events.rs:62-70`
- Modify: `crates/visio-core/src/participants.rs:85-95` (test helper)

**Step 1: Add fields to ParticipantInfo**

In `crates/visio-core/src/events.rs`, add two fields to the `ParticipantInfo` struct:

```rust
#[derive(Debug, Clone)]
pub struct ParticipantInfo {
    pub sid: String,
    pub identity: String,
    pub name: Option<String>,
    pub is_muted: bool,
    pub has_video: bool,
    pub video_track_sid: Option<String>,
    pub has_screen_share: bool,
    pub screen_share_track_sid: Option<String>,
    pub connection_quality: ConnectionQuality,
}
```

**Step 2: Fix all ParticipantInfo constructors**

Update every place that constructs `ParticipantInfo` to include the new fields with defaults:
- `crates/visio-core/src/room.rs` — `remote_participant_to_info()` (~line 770): add `has_screen_share: false, screen_share_track_sid: None`
- `crates/visio-core/src/room.rs` — `local_participant_info()` (~line 230): add `has_screen_share: false, screen_share_track_sid: None`
- `crates/visio-core/src/participants.rs` — `make_participant()` test helper (~line 86): add the fields

**Step 3: Run tests**

Run: `cargo test -p visio-core`
Expected: All 48 tests pass (new fields have defaults, no logic change yet)

**Step 4: Commit**

```bash
git add crates/visio-core/src/events.rs crates/visio-core/src/room.rs crates/visio-core/src/participants.rs
git commit -m "feat(core): add screen_share_track_sid and has_screen_share to ParticipantInfo"
```

---

### Task 2: Handle ScreenShare in track events (Rust core)

**Files:**
- Modify: `crates/visio-core/src/room.rs:900-1032`

**Step 1: Write failing test**

In `crates/visio-core/src/participants.rs`, add tests:

```rust
#[test]
fn track_subscribed_screen_share_sets_fields() {
    let mut mgr = ParticipantManager::new();
    mgr.add_participant(make_participant("p1", "Alice"));

    if let Some(p) = mgr.participant_mut("p1") {
        p.has_screen_share = true;
        p.screen_share_track_sid = Some("TR_SCREEN_1".to_string());
    }

    let p = mgr.participant("p1").unwrap();
    assert!(p.has_screen_share);
    assert_eq!(p.screen_share_track_sid.as_deref(), Some("TR_SCREEN_1"));
}

#[test]
fn track_muted_screen_share_clears_fields() {
    let mut mgr = ParticipantManager::new();
    let mut p = make_participant("p1", "Alice");
    p.has_screen_share = true;
    p.screen_share_track_sid = Some("TR_SCREEN_1".to_string());
    p.has_video = true;
    p.video_track_sid = Some("TR_CAM_1".to_string());
    mgr.add_participant(p);

    // Simulate TrackMuted for screen share
    if let Some(p) = mgr.participant_mut("p1") {
        p.has_screen_share = false;
        p.screen_share_track_sid = None;
    }

    let p = mgr.participant("p1").unwrap();
    assert!(!p.has_screen_share);
    assert!(p.screen_share_track_sid.is_none());
    // Camera should be unaffected
    assert!(p.has_video);
    assert_eq!(p.video_track_sid.as_deref(), Some("TR_CAM_1"));
}

#[test]
fn track_unmuted_screen_share_restores_fields() {
    let mut mgr = ParticipantManager::new();
    mgr.add_participant(make_participant("p1", "Alice"));

    if let Some(p) = mgr.participant_mut("p1") {
        p.has_screen_share = true;
        p.screen_share_track_sid = Some("TR_SCREEN_1".to_string());
    }

    let p = mgr.participant("p1").unwrap();
    assert!(p.has_screen_share);
    assert_eq!(p.screen_share_track_sid.as_deref(), Some("TR_SCREEN_1"));
}
```

**Step 2: Run tests to verify they pass (these are unit tests for ParticipantManager)**

Run: `cargo test -p visio-core`
Expected: PASS (these test the data model, not event handling)

**Step 3: Update TrackSubscribed handler in room.rs**

In `crates/visio-core/src/room.rs`, around lines 903-911, change:

```rust
// BEFORE:
{
    let mut pm = participants.lock().await;
    if let Some(p) = pm.participant_mut(&psid)
        && track_kind == TrackKind::Video
    {
        p.has_video = true;
        p.video_track_sid = Some(track_sid.clone());
    }
}

// AFTER:
{
    let mut pm = participants.lock().await;
    if let Some(p) = pm.participant_mut(&psid)
        && track_kind == TrackKind::Video
    {
        match source {
            TrackSource::ScreenShare => {
                p.has_screen_share = true;
                p.screen_share_track_sid = Some(track_sid.clone());
            }
            _ => {
                p.has_video = true;
                p.video_track_sid = Some(track_sid.clone());
            }
        }
    }
}
```

**Step 4: Update TrackUnsubscribed handler**

In `crates/visio-core/src/room.rs`, around lines 964-971, change:

```rust
// BEFORE:
if is_video {
    let mut pm = participants.lock().await;
    if let Some(p) = pm.participant_mut(&psid) {
        p.has_video = false;
        p.video_track_sid = None;
    }
    subscribed_tracks.lock().await.remove(&track_sid);
}

// AFTER:
if is_video {
    let is_screen_share = publication.source() == LkTrackSource::Screenshare;
    let mut pm = participants.lock().await;
    if let Some(p) = pm.participant_mut(&psid) {
        if is_screen_share {
            p.has_screen_share = false;
            p.screen_share_track_sid = None;
        } else {
            p.has_video = false;
            p.video_track_sid = None;
        }
    }
    subscribed_tracks.lock().await.remove(&track_sid);
}
```

**Step 5: Update TrackMuted handler**

In `crates/visio-core/src/room.rs`, around lines 988-998, change:

```rust
// BEFORE:
match source {
    TrackSource::Microphone => p.is_muted = true,
    TrackSource::Camera => {
        p.has_video = false;
        p.video_track_sid = None;
    }
    _ => {}
}

// AFTER:
match source {
    TrackSource::Microphone => p.is_muted = true,
    TrackSource::Camera => {
        p.has_video = false;
        p.video_track_sid = None;
    }
    TrackSource::ScreenShare => {
        p.has_screen_share = false;
        p.screen_share_track_sid = None;
    }
    _ => {}
}
```

**Step 6: Update TrackUnmuted handler**

In `crates/visio-core/src/room.rs`, around lines 1016-1024, change:

```rust
// BEFORE:
match source {
    TrackSource::Microphone => p.is_muted = false,
    TrackSource::Camera => {
        p.has_video = true;
        p.video_track_sid = Some(track_sid);
    }
    _ => {}
}

// AFTER:
match source {
    TrackSource::Microphone => p.is_muted = false,
    TrackSource::Camera => {
        p.has_video = true;
        p.video_track_sid = Some(track_sid);
    }
    TrackSource::ScreenShare => {
        p.has_screen_share = true;
        p.screen_share_track_sid = Some(track_sid);
    }
    _ => {}
}
```

**Step 7: Update remote_participant_to_info to detect existing screen shares**

In `crates/visio-core/src/room.rs`, function `remote_participant_to_info` (~line 770), the function currently only looks for camera tracks. Add screen share detection:

```rust
fn remote_participant_to_info(p: &RemoteParticipant) -> ParticipantInfo {
    let name = { /* ... existing ... */ };
    let mut has_video = false;
    let mut video_track_sid = None;
    let mut has_screen_share = false;
    let mut screen_share_track_sid = None;
    let mut is_muted = true;

    for (_sid, pub_) in p.track_publications() {
        match pub_.source() {
            LkTrackSource::Camera => {
                if pub_.kind() == LkTrackKind::Video && !pub_.is_muted() {
                    has_video = true;
                    video_track_sid = Some(pub_.sid().to_string());
                }
            }
            LkTrackSource::Screenshare => {
                if pub_.kind() == LkTrackKind::Video && !pub_.is_muted() {
                    has_screen_share = true;
                    screen_share_track_sid = Some(pub_.sid().to_string());
                }
            }
            LkTrackSource::Microphone => {
                if !pub_.is_muted() {
                    is_muted = false;
                }
            }
            _ => {}
        }
    }

    ParticipantInfo {
        sid: p.sid().to_string(),
        identity: p.identity().to_string(),
        name,
        is_muted,
        has_video,
        video_track_sid,
        has_screen_share,
        screen_share_track_sid,
        connection_quality: ConnectionQuality::Good,
    }
}
```

**Step 8: Run tests**

Run: `cargo test -p visio-core`
Expected: All tests pass

**Step 9: Commit**

```bash
git add crates/visio-core/src/room.rs crates/visio-core/src/participants.rs
git commit -m "feat(core): handle ScreenShare source in all track events"
```

---

### Task 3: Update UniFFI bindings (visio-ffi)

**Files:**
- Modify: `crates/visio-ffi/src/visio.udl:33-41`

**Step 1: Add new fields to ParticipantInfo dictionary**

```
dictionary ParticipantInfo {
    string sid;
    string identity;
    string? name;
    boolean is_muted;
    boolean has_video;
    string? video_track_sid;
    boolean has_screen_share;
    string? screen_share_track_sid;
    ConnectionQuality connection_quality;
};
```

**Step 2: Build to verify**

Run: `cargo build -p visio-ffi`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add crates/visio-ffi/src/visio.udl
git commit -m "feat(ffi): expose screen share fields in ParticipantInfo UDL"
```

---

### Task 4: Update Desktop backend — emit screen share info (Tauri)

**Files:**
- Modify: `crates/visio-desktop/src/lib.rs` (get_participants, get_local_participant, TrackSubscribed handler)

**Step 1: Add screen share fields to get_participants JSON**

In `get_participants` command (~line 356), add the new fields:

```rust
serde_json::json!({
    "sid": p.sid,
    "identity": p.identity,
    "name": p.name,
    "is_muted": p.is_muted,
    "has_video": p.has_video,
    "video_track_sid": p.video_track_sid,
    "has_screen_share": p.has_screen_share,
    "screen_share_track_sid": p.screen_share_track_sid,
    "connection_quality": format!("{:?}", p.connection_quality),
})
```

**Step 2: Add screen share fields to get_local_participant JSON**

Same change in `get_local_participant` command (~line 377).

**Step 3: Emit track-subscribed event with source info**

In the `DesktopEventListener::on_event` handler, the `TrackSubscribed` match arm (~line 116) currently only matches `TrackKind::Video`. Keep it, but also emit a Tauri event so the frontend knows about it:

```rust
VisioEvent::TrackSubscribed(TrackInfo {
    sid: ref track_sid,
    ref participant_sid,
    kind: TrackKind::Video,
    ref source,
}) => {
    let room = self.room.clone();
    let sid = track_sid.clone();
    let src_str = source_to_str(source);
    let psid = participant_sid.clone();
    tokio::spawn(async move {
        let rm = room.lock().await;
        if let Some(video_track) = rm.get_video_track(&sid).await {
            tracing::info!("auto-starting video renderer for track {sid} (source={src_str})");
            visio_video::start_track_renderer(
                sid.clone(),
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
```

Also emit `track-unsubscribed`:

```rust
VisioEvent::TrackUnsubscribed(track_sid) => {
    tracing::info!("auto-stopping video renderer for track {track_sid}");
    visio_video::stop_track_renderer(&track_sid);
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit("track-unsubscribed", &track_sid);
    }
}
```

**Step 4: Build**

Run: `cargo build -p visio-desktop`
Expected: Compiles

**Step 5: Commit**

```bash
git add crates/visio-desktop/src/lib.rs
git commit -m "feat(desktop): emit screen share fields and track events"
```

---

### Task 5: Desktop frontend — display screen share and focus logic

**Files:**
- Modify: `crates/visio-desktop/frontend/src/App.tsx`

**Step 1: Update Participant interface**

```typescript
interface Participant {
  sid: string;
  identity: string;
  name: string | null;
  is_muted: boolean;
  has_video: boolean;
  video_track_sid: string | null;
  has_screen_share: boolean;
  screen_share_track_sid: string | null;
  connection_quality: string;
}
```

**Step 2: Add FocusItem type and screen share icon**

```typescript
type FocusItem = {
  participantSid: string;
  source: "camera" | "screen_share";
} | null;
```

Add a simple monitor icon component:

```typescript
function ScreenShareIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M21 3H3c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h7v2H8v2h8v-2h-2v-2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 14H3V5h18v12z"/>
    </svg>
  );
}
```

**Step 3: Add focusedItem state in the main App component**

In the CallView component (or wherever the call state lives), add:

```typescript
const [focusedItem, setFocusedItem] = useState<FocusItem>(null);
```

**Step 4: Auto-focus on screen share arrival**

Listen to `track-subscribed` event:

```typescript
useEffect(() => {
  const unlisten = listen<{ trackSid: string; participantSid: string; source: string }>(
    "track-subscribed",
    (event) => {
      if (event.payload.source === "screen_share") {
        setFocusedItem({
          participantSid: event.payload.participantSid,
          source: "screen_share",
        });
      }
    }
  );
  return () => { unlisten.then(f => f()); };
}, []);
```

Listen to `track-unsubscribed` to auto-unfocus:

```typescript
useEffect(() => {
  const unlisten = listen<string>("track-unsubscribed", (event) => {
    // If the unsubscribed track was the focused screen share, unfocus
    setFocusedItem(prev => {
      if (!prev) return null;
      const participant = participants.find(p => p.sid === prev.participantSid);
      if (prev.source === "screen_share" && participant && participant.screen_share_track_sid === null) {
        return null;
      }
      return prev;
    });
  });
  return () => { unlisten.then(f => f()); };
}, [participants]);
```

**Step 5: Build the display items list**

Create a list of "display items" that includes both camera tiles and screen share tiles:

```typescript
interface DisplayItem {
  key: string;
  participant: Participant;
  source: "camera" | "screen_share";
  trackSid: string | null;
  label: string;
  isScreenShare: boolean;
}

function buildDisplayItems(participants: Participant[], t: TFunction): DisplayItem[] {
  const items: DisplayItem[] = [];
  for (const p of participants) {
    // Camera tile
    items.push({
      key: `${p.sid}-camera`,
      participant: p,
      source: "camera",
      trackSid: p.video_track_sid,
      label: p.name || p.identity || t("unknown"),
      isScreenShare: false,
    });
    // Screen share tile
    if (p.has_screen_share && p.screen_share_track_sid) {
      items.push({
        key: `${p.sid}-screen`,
        participant: p,
        source: "screen_share",
        trackSid: p.screen_share_track_sid,
        label: p.name || p.identity || t("unknown"),
        isScreenShare: true,
      });
    }
  }
  return items;
}
```

**Step 6: Update the tile rendering**

Create a `DisplayItemTile` that handles both camera and screen share:

```typescript
function DisplayItemTile({
  item,
  videoFrames,
  isActiveSpeaker,
  handRaisePosition,
  onClick,
}: {
  item: DisplayItem;
  videoFrames: Map<string, string>;
  isActiveSpeaker?: boolean;
  handRaisePosition?: number;
  onClick?: () => void;
}) {
  const t = useT();
  const displayName = item.label;
  const initials = getInitials(displayName);
  const hue = getHue(displayName);

  const videoSrc = item.trackSid ? videoFrames.get(item.trackSid) : undefined;

  return (
    <div
      className={`tile ${isActiveSpeaker && !item.isScreenShare ? "tile-active-speaker" : ""}`}
      onClick={onClick}
      style={{ cursor: onClick ? "pointer" : undefined }}
    >
      {videoSrc ? (
        <img className="tile-video" src={`data:image/jpeg;base64,${videoSrc}`} alt="" />
      ) : (
        <div className="tile-avatar" style={{ background: `hsl(${hue}, 50%, 35%)` }}>
          <span className="tile-initials">{initials}</span>
        </div>
      )}
      <div className="tile-metadata">
        {!item.isScreenShare && item.participant.is_muted && (
          <span className="tile-muted-icon"><RiMicOffFill size={14} /></span>
        )}
        {item.isScreenShare && (
          <span className="tile-screen-icon"><ScreenShareIcon size={14} /></span>
        )}
        {handRaisePosition != null && handRaisePosition > 0 && !item.isScreenShare && (
          <span className="tile-hand-badge"><RiHand size={12} /> {handRaisePosition}</span>
        )}
        <span className="tile-name">{displayName}</span>
        {!item.isScreenShare && <ConnectionQualityBars quality={item.participant.connection_quality} />}
      </div>
    </div>
  );
}
```

**Step 7: Update the call view grid/focus layout**

Replace the current tile grid with focus/thumbnail logic:

```typescript
// In the call view render:
const displayItems = buildDisplayItems(participants, t);
const focusedDisplayItem = focusedItem
  ? displayItems.find(
      d => d.participant.sid === focusedItem.participantSid && d.source === focusedItem.source
    )
  : null;
const thumbnailItems = focusedDisplayItem
  ? displayItems.filter(d => d.key !== focusedDisplayItem.key)
  : [];

// Render:
{focusedDisplayItem ? (
  <div className="focus-layout">
    <div className="focus-main">
      <DisplayItemTile
        item={focusedDisplayItem}
        videoFrames={videoFrames}
        isActiveSpeaker={activeSpeakers.includes(focusedDisplayItem.participant.sid)}
        onClick={() => setFocusedItem(null)}
      />
    </div>
    {thumbnailItems.length > 0 && (
      <div className="focus-thumbnails">
        {thumbnailItems.map(item => (
          <DisplayItemTile
            key={item.key}
            item={item}
            videoFrames={videoFrames}
            isActiveSpeaker={activeSpeakers.includes(item.participant.sid)}
            onClick={() => setFocusedItem({
              participantSid: item.participant.sid,
              source: item.source,
            })}
          />
        ))}
      </div>
    )}
  </div>
) : (
  <div className="grid-layout">
    {displayItems.map(item => (
      <DisplayItemTile
        key={item.key}
        item={item}
        videoFrames={videoFrames}
        isActiveSpeaker={activeSpeakers.includes(item.participant.sid)}
        onClick={() => setFocusedItem({
          participantSid: item.participant.sid,
          source: item.source,
        })}
      />
    ))}
  </div>
)}
```

**Step 8: Add CSS for focus layout**

Add to the CSS:

```css
.focus-layout {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 8px;
  padding: 8px;
}
.focus-main {
  flex: 1;
  min-height: 0;
}
.focus-main .tile {
  height: 100%;
}
.focus-thumbnails {
  display: flex;
  gap: 8px;
  height: 120px;
  overflow-x: auto;
}
.focus-thumbnails .tile {
  width: 160px;
  min-width: 160px;
  height: 100%;
}
.tile-screen-icon {
  color: #4fc3f7;
  display: flex;
  align-items: center;
}
```

**Step 9: Build and manual test**

Run: `cd crates/visio-desktop && cargo tauri dev`
Expected: App compiles. When another participant shares screen (e.g. from LiveKit playground), it appears as a focused tile with a screen icon.

**Step 10: Commit**

```bash
git add crates/visio-desktop/frontend/src/App.tsx
git commit -m "feat(desktop): display screen shares with focus/thumbnail layout"
```

---

### Task 6: Desktop screen share publishing — Rust backend

**Files:**
- Modify: `crates/visio-core/src/controls.rs`

**Step 1: Add screen share publishing method**

Add to `MeetingControls`:

```rust
/// Publish a screen share track to the room.
///
/// Uses the LiveKit native screen capture source.
/// `source_id` identifies which screen/window to capture.
pub async fn publish_screen_share(&self, source_id: String) -> Result<(), VisioError> {
    let room = self.room.lock().await;
    let room = room
        .as_ref()
        .ok_or_else(|| VisioError::Room("not connected".into()))?;

    // Create a video source configured as screencast
    let source = NativeVideoSource::new(
        VideoResolution {
            width: 1920,
            height: 1080,
        },
        true, // is_screencast
    );

    let track = LocalVideoTrack::create_video_track(
        &format!("screen_{source_id}"),
        RtcVideoSource::Native(source.clone()),
    );

    room.local_participant()
        .publish_track(
            LocalTrack::Video(track),
            TrackPublishOptions {
                source: LkTrackSource::Screenshare,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| VisioError::Room(format!("publish screen share: {e}")))?;

    tracing::info!("screen share track published (source_id={source_id})");
    Ok(())
}

/// Stop publishing the screen share track.
pub async fn stop_screen_share(&self) -> Result<(), VisioError> {
    let room = self.room.lock().await;
    let room = room
        .as_ref()
        .ok_or_else(|| VisioError::Room("not connected".into()))?;

    let local = room.local_participant();
    for (_sid, pub_) in local.track_publications() {
        if pub_.source() == LkTrackSource::Screenshare {
            local
                .unpublish_track(&pub_.sid())
                .await
                .map_err(|e| VisioError::Room(format!("unpublish screen share: {e}")))?;
            tracing::info!("screen share track unpublished");
            break;
        }
    }
    Ok(())
}
```

**Step 2: Build**

Run: `cargo build -p visio-core`
Expected: Compiles

**Step 3: Commit**

```bash
git add crates/visio-core/src/controls.rs
git commit -m "feat(core): add publish_screen_share and stop_screen_share methods"
```

---

### Task 7: Desktop Tauri commands for screen share

**Files:**
- Modify: `crates/visio-desktop/src/lib.rs`

**Step 1: Add Tauri commands**

```rust
#[tauri::command]
async fn start_screen_share(
    state: tauri::State<'_, VisioState>,
    source_id: String,
) -> Result<(), String> {
    let controls = state.controls.lock().await;
    controls
        .publish_screen_share(source_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_screen_share(
    state: tauri::State<'_, VisioState>,
) -> Result<(), String> {
    let controls = state.controls.lock().await;
    controls
        .stop_screen_share()
        .await
        .map_err(|e| e.to_string())
}
```

**Step 2: Register commands in the Tauri builder**

Find the `.invoke_handler(tauri::generate_handler![...])` call and add `start_screen_share` and `stop_screen_share` to the list.

**Step 3: Build**

Run: `cargo build -p visio-desktop`
Expected: Compiles

**Step 4: Commit**

```bash
git add crates/visio-desktop/src/lib.rs
git commit -m "feat(desktop): add Tauri commands for screen share publishing"
```

---

### Task 8: Desktop frontend — screen share button and source picker

**Files:**
- Modify: `crates/visio-desktop/frontend/src/App.tsx`

**Step 1: Add screen share button to control bar**

Add a screen share toggle button next to the camera button in the control bar. Use the `RiApps2Line` icon (already imported) for sharing:

```typescript
const [isScreenSharing, setIsScreenSharing] = useState(false);

// In the control bar:
<button
  className={`control-btn ${isScreenSharing ? "control-btn-active-danger" : ""}`}
  onClick={async () => {
    if (isScreenSharing) {
      await invoke("stop_screen_share");
      setIsScreenSharing(false);
    } else {
      // For now, share the main screen (source_id "0")
      try {
        await invoke("start_screen_share", { sourceId: "0" });
        setIsScreenSharing(true);
      } catch (e) {
        console.error("Failed to start screen share:", e);
      }
    }
  }}
  title={isScreenSharing ? t("call.stopShare") : t("call.startShare")}
>
  <RiApps2Line size={20} />
</button>
```

**Step 2: Add CSS for active danger state**

```css
.control-btn-active-danger {
  background: var(--danger) !important;
  color: white !important;
}
```

**Step 3: Add i18n keys**

Add to `i18n/en.json`:
```json
"call.startShare": "Share screen",
"call.stopShare": "Stop sharing"
```

Add to `i18n/fr.json`:
```json
"call.startShare": "Partager l'écran",
"call.stopShare": "Arrêter le partage"
```

Add equivalent keys to de.json, es.json, it.json, nl.json.

**Step 4: Build and test**

Run: `cd crates/visio-desktop && cargo tauri dev`
Expected: Screen share button visible. Clicking it starts sharing (platform may prompt for permissions).

**Step 5: Commit**

```bash
git add crates/visio-desktop/frontend/src/App.tsx i18n/
git commit -m "feat(desktop): add screen share button with start/stop toggle"
```

---

### Task 9: Update Android UI — screen share display

**Files:**
- Modify: `android/app/src/main/kotlin/io/visio/mobile/ui/CallScreen.kt`
- Modify: `android/app/src/main/kotlin/io/visio/mobile/VisioManager.kt`

**Step 1: Update focusedParticipant state to FocusItem**

Replace `focusedParticipantSid` with a more expressive type:

```kotlin
data class FocusItem(val participantSid: String, val source: String) // "camera" or "screen_share"

var focusedItem by remember { mutableStateOf<FocusItem?>(null) }
```

**Step 2: Build the display items list**

```kotlin
data class DisplayItem(
    val key: String,
    val participant: ParticipantInfo,
    val source: String,
    val trackSid: String?,
    val isScreenShare: Boolean,
)

fun buildDisplayItems(participants: List<ParticipantInfo>): List<DisplayItem> {
    val items = mutableListOf<DisplayItem>()
    for (p in participants) {
        items.add(DisplayItem(
            key = "${p.sid}-camera",
            participant = p,
            source = "camera",
            trackSid = p.videoTrackSid,
            isScreenShare = false,
        ))
        if (p.hasScreenShare && p.screenShareTrackSid != null) {
            items.add(DisplayItem(
                key = "${p.sid}-screen",
                participant = p,
                source = "screen_share",
                trackSid = p.screenShareTrackSid,
                isScreenShare = true,
            ))
        }
    }
    return items
}
```

**Step 3: Auto-focus on screen share arrival**

In the `LaunchedEffect` that listens to events, when a `TrackSubscribed` with `source=ScreenShare` arrives:

```kotlin
// Inside event handling
is VisioEvent.TrackSubscribed -> {
    val info = event.info
    if (info.source == TrackSource.SCREEN_SHARE && info.kind == TrackKind.VIDEO) {
        focusedItem = FocusItem(info.participantSid, "screen_share")
    }
}
```

**Step 4: Auto-unfocus when screen share ends**

```kotlin
is VisioEvent.TrackUnsubscribed -> {
    // If the focused item was the screen share that just ended, unfocus
    if (focusedItem?.source == "screen_share") {
        val participant = participants.find { it.sid == focusedItem?.participantSid }
        if (participant?.hasScreenShare != true) {
            focusedItem = null
        }
    }
}
```

**Step 5: Update grid/focus rendering**

Use `buildDisplayItems` and the focus layout similar to Desktop. The screen share tile shows a monitor icon overlay and the participant name.

**Step 6: Build**

Run: `cd android && ./gradlew assembleDebug`
Expected: Compiles

**Step 7: Commit**

```bash
git add android/
git commit -m "feat(android): display screen shares with focus/thumbnail layout"
```

---

### Task 10: Update iOS UI — screen share display

**Files:**
- Modify: `ios/VisioMobile/Views/CallView.swift`
- Modify: `ios/VisioMobile/VisioManager.swift`

**Step 1: Update focusedParticipant to FocusItem**

```swift
enum FocusSource: Equatable {
    case camera
    case screenShare
}

struct FocusItem: Equatable {
    let participantSid: String
    let source: FocusSource
}

// Replace:
@State private var focusedParticipant: String? = nil
// With:
@State private var focusedItem: FocusItem? = nil
```

**Step 2: Build DisplayItem model**

```swift
struct DisplayItem: Identifiable {
    let id: String  // "\(sid)-camera" or "\(sid)-screen"
    let participant: ParticipantInfo
    let source: FocusSource
    let trackSid: String?
    let isScreenShare: Bool

    var label: String {
        participant.name ?? participant.identity
    }
}

func buildDisplayItems(_ participants: [ParticipantInfo]) -> [DisplayItem] {
    var items: [DisplayItem] = []
    for p in participants {
        items.append(DisplayItem(
            id: "\(p.sid)-camera",
            participant: p,
            source: .camera,
            trackSid: p.videoTrackSid,
            isScreenShare: false
        ))
        if p.hasScreenShare, let ssSid = p.screenShareTrackSid {
            items.append(DisplayItem(
                id: "\(p.sid)-screen",
                participant: p,
                source: .screenShare,
                trackSid: ssSid,
                isScreenShare: true
            ))
        }
    }
    return items
}
```

**Step 3: Auto-focus on screen share**

In VisioManager, when handling `TrackSubscribed` with `source == .screenShare`, post a notification or set a published property that CallView observes:

```swift
// In VisioManager event handling:
case .trackSubscribed(let info):
    if info.source == .screenShare && info.kind == .video {
        DispatchQueue.main.async {
            self.screenShareStarted = (participantSid: info.participantSid, trackSid: info.sid)
        }
    }
```

In CallView, observe this and auto-focus:

```swift
.onChange(of: manager.screenShareStarted) { _, newValue in
    if let ss = newValue {
        focusedItem = FocusItem(participantSid: ss.participantSid, source: .screenShare)
    }
}
```

**Step 4: Update grid and focus layout**

Replace the current grid/focus logic to use `DisplayItem` arrays. The focus layout shows the focused item large with a thumbnail bar. Each screen share tile shows a monitor icon + participant name.

**Step 5: Register screen share video surfaces**

In the `VideoFrameRouter`, when a screen share tile appears, register the `VideoDisplayView` with the screen share track SID — exactly the same mechanism as for camera tracks.

**Step 6: Build**

Run: `cd ios && xcodebuild -scheme VisioMobile -sdk iphonesimulator -destination 'platform=iOS Simulator,name=iPhone 16' build`
Expected: Compiles

**Step 7: Commit**

```bash
git add ios/
git commit -m "feat(ios): display screen shares with focus/thumbnail layout"
```

---

### Task 11: Final integration test

**Step 1: Run all Rust tests**

Run: `cargo test`
Expected: All tests pass (48+ visio-core + 8 visio-desktop)

**Step 2: Manual integration test**

1. Start LiveKit dev server: `livekit-server --dev`
2. Open Desktop app: `cd crates/visio-desktop && cargo tauri dev`
3. Open LiveKit playground in browser, join same room
4. From browser: share screen → verify Desktop app shows it as focused tile with screen icon
5. Click another participant thumbnail → verify focus switches
6. Click screen share thumbnail → verify it re-focuses
7. Stop sharing from browser → verify auto-return to grid
8. From Desktop: click "Share screen" button → verify browser sees the shared screen
9. Click "Stop sharing" → verify it stops

**Step 3: Commit final state if any fixes needed**

```bash
git commit -m "fix(screen-share): integration test fixes"
```
