# OIDC Authentication — Design Document

**Date:** 2026-03-07
**Issue:** https://github.com/mmaudet/visio-mobile/issues/11
**Status:** Approved

## Goal

Add OIDC authentication to Visio Mobile (Android, iOS, Desktop) with full parity with the web client: user identity, room creation, and restricted room access.

## Constraints

- No backend modification required
- Works with any backend using cookie-based sessions after OIDC (Meet, or any other)
- Anonymous flow unchanged — no regression for unauthenticated users

## Authentication Flow

The app uses platform-native secure browser APIs to perform the OIDC flow, then captures the session cookie.

```
Home Screen
  │
  ├─ Tap "Connect"
  │    │
  │    ▼
  │  ASWebAuthenticationSession (iOS)
  │  Custom Tabs (Android)
  │  System browser via tauri-plugin-shell (Desktop)
  │    │
  │    ▼
  │  GET https://{instance}/authenticate/?returnTo=https://{instance}/
  │    │
  │    ▼
  │  OIDC provider login page
  │    │
  │    ▼
  │  Backend sets cookie `sessionid`, redirects to returnTo
  │    │
  │    ▼
  │  App detects end of auth flow
  │  Extracts `sessionid` cookie
  │    │
  │    ▼
  │  GET /api/v1.0/users/me/ (Cookie: sessionid=<value>)
  │    │
  │    ├─ 200 OK → Authenticated (store cookie, show user info)
  │    └─ 401    → Failed (discard, stay anonymous)
  │
  └─ Home Screen (authenticated state)
```

### Token Storage

- **iOS:** Keychain (`kSecClassGenericPassword`)
- **Android:** EncryptedSharedPreferences (Jetpack Security)
- **Desktop:** Encrypted file in `app_data_dir`

### Session Persistence

- On app launch, check for stored cookie
- Call `GET /users/me/` to validate
- If 401: cookie expired, delete it, return to anonymous state
- No refresh token — user re-authenticates manually when session expires

### Logout

1. `GET https://{instance}/logout` with the session cookie
2. Delete cookie from local storage
3. Return to anonymous state on home screen

## Rust Core Changes (visio-core)

### New module: `session.rs`

```rust
pub enum SessionState {
    Anonymous,
    Authenticated { user: UserInfo, cookie: String },
}

pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub display_name: String,
}
```

**Public API:**
- `authenticate(cookie: String) -> Result<UserInfo>` — calls `/users/me/`, returns user info
- `logout(meet_url: String)` — calls `/logout`, clears session
- `session_state() -> SessionState` — returns current state
- `validate_session() -> bool` — silent check via `/users/me/`

### Impact on existing code

- `auth.rs`: `request_token()` and `validate_room()` include `Cookie` header when authenticated
- `VisioClient` (UniFFI): new methods exposed via FFI: `setSessionCookie()`, `getSessionState()`, `logout()`
- No changes to `settings.rs` — cookie is not a user setting

## UI Changes

### Home Screen — Anonymous State (current + Connect button)

```
┌─────────────────────────┐
│              [Settings]  │
│                         │
│        [Logo Visio]     │
│       Visio Mobile      │
│                         │
│  [Connect]              │
│                         │
│  Meeting URL: [_______] │
│  Display Name: [______] │
│  [Join]                 │
└─────────────────────────┘
```

### Home Screen — Authenticated State

```
┌─────────────────────────┐
│              [Settings]  │
│                         │
│        [Logo Visio]     │
│       Visio Mobile      │
│                         │
│  User: Jean Dupont      │
│  [Logout]               │
│                         │
│  Meeting URL: [_______] │
│  Display Name: [______] │ ← pre-filled with OIDC name
│  [Join]                 │
└─────────────────────────┘
```

### i18n — New keys (all 6 languages)

- `home.connect`
- `home.logout`
- `home.loggedAs`

## Implementation Phases

### Phase 1 — Basic Authentication

- `session.rs` in visio-core (SessionState, cookie management, `/users/me/`, `/logout`)
- UniFFI exposure via visio-ffi
- Secure cookie storage (Keychain / EncryptedSharedPreferences / encrypted file)
- Connect button + OIDC flow on Android, iOS, and Desktop
- Authenticated state on home screen (name, logout)
- Display name pre-fill from OIDC identity
- i18n (6 languages)
- Silent session validation on launch

### Phase 2 — Room Creation

- New screen or dialog for room creation (name, access level: public/trusted/restricted)
- `POST /api/v1.0/rooms/` with session cookie
- "Create" button visible only when authenticated
- Navigate to created room

### Phase 3 — Restricted Room Support

- Handle 403 errors on `trusted`/`restricted` rooms
- Prompt user to connect when room requires authentication
- Flow: anonymous join attempt → 403 → propose Connect → retry authenticated

## Platform-Specific Notes

### Android
- Custom Tabs via `androidx.browser:browser`
- Cookie extraction via callback detection
- EncryptedSharedPreferences for storage

### iOS
- ASWebAuthenticationSession (iOS 16+)
- Keychain for storage
- `presentationAnchor` from the active window

### Desktop (Tauri)
- `tauri-plugin-shell` to open system browser
- Deep link interception via Tauri scheme handler (`visio://`)
- `session.rs` used directly (no FFI layer)
- Encrypted file storage in `app_data_dir`
