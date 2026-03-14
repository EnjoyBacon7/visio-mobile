# Rebranding & Store Publication — Design

**Date:** 2026-03-14
**Status:** Approved

## Overview

Replace all app icons across iOS, Android, and Desktop with the new WHITE identity (white background, blue gradient "C" + play triangle logo). Set up Fastlane metadata and release lanes for App Store and Google Play publication. Update the GitHub README with the new branding.

**Source assets:** `/Users/mmaudet/work/Logo Visio Mobile/WHITE/`
**Target project:** `/Users/mmaudet/work/visio-mobile-v2/`

## 1. Icon Replacement — Android

### Launcher Icons

| Density | Size | Source | Destination |
|---------|------|--------|-------------|
| mdpi | 48x48 | `WHITE/android/icon-48x48.png` | `android/app/src/main/res/mipmap-mdpi/ic_launcher.png` + `ic_launcher_round.png` |
| hdpi | 72x72 | `WHITE/android/icon-72x72.png` | `android/app/src/main/res/mipmap-hdpi/ic_launcher.png` + `ic_launcher_round.png` |
| xhdpi | 96x96 | `WHITE/android/icon-96x96.png` | `android/app/src/main/res/mipmap-xhdpi/ic_launcher.png` + `ic_launcher_round.png` |
| xxhdpi | 144x144 | `WHITE/android/icon-144x144.png` | `android/app/src/main/res/mipmap-xxhdpi/ic_launcher.png` + `ic_launcher_round.png` |
| xxxhdpi | 192x192 | `WHITE/android/icon-192x192.png` | `android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png` + `ic_launcher_round.png` |

### Adaptive Icon Foreground

Generate `ic_launcher_foreground.png` per density from `WHITE/android/icon-512x512.png`:
- Apply 66% safe zone padding (logo centered in 66% of canvas, 17% margin each side)
- Resize to each density: mdpi=108, hdpi=162, xhdpi=216, xxhdpi=324, xxxhdpi=432

### Adaptive Icon Background

Retain the existing `ic_launcher_background.xml` (solid white color `#FFFFFF`) — consistent with the WHITE branding. Verify the color value in `android/app/src/main/res/values/ic_launcher_background.xml` and update to `#FFFFFF` if different.

### Play Store Icon

`WHITE/Google Play Market/icon-512x512.png` → `android/app/src/main/assets/icons/icon-playstore-512.png`

## 2. Icon Replacement — iOS

### AppIcon.appiconset

Copy all 37 PNG files from `WHITE/ios/` → `ios/VisioMobile/Assets.xcassets/AppIcon.appiconset/`

Replace `Contents.json` with the one from `WHITE/ios/Contents.json` (already has correct idiom/size/scale/filename mappings for iPhone, iPad, and ios-marketing).

The 1024x1024 App Store icon is included in the set (`WHITE/App Store/icon-1024x1024.png` = `WHITE/ios/icon-1024x1024.png`).

### AppLogo (optional)

Replace `AppLogo.imageset/logo.png` with a resized version of the new logo if it's used in-app.

## 3. Icon Replacement — Desktop (Tauri)

### Direct Copies

| Destination | Source |
|-------------|--------|
| `crates/visio-desktop/icons/icon.png` | `WHITE/web/icon-256x256.png` |
| `crates/visio-desktop/icons/32x32.png` | `WHITE/web/icon-32x32.png` |
| `crates/visio-desktop/icons/128x128.png` | `WHITE/web/icon-128x128.png` |
| `crates/visio-desktop/icons/128x128@2x.png` | `WHITE/web/icon-256x256.png` |
| `crates/visio-desktop/frontend/public/icon.png` | `WHITE/web/icon-128x128.png` |

### Generated Formats

**icon.icns (macOS):**
1. Create temporary `.iconset/` directory with sizes 16, 32, 64, 128, 256, 512 (+ @2x variants) from `WHITE/App Store/icon-1024x1024.png` (highest quality source) via `sips -z`
2. Run `iconutil --convert icns` to produce `icon.icns`

**icon.ico (Windows):**
- Combine 16x16, 32x32, 48x48, 64x64, 128x128, 256x256 PNGs into single `.ico` using ImageMagick `convert`

### Frontend Logo

`crates/visio-desktop/frontend/public/logo.png` — replace with `SVG/transparent light.png` resized to ~400px wide.

## 4. README Update

### Banner

Replace current header in `README.md`:
```markdown
<p align="center">
  <img src="docs/screenshots/visio-mobile-banner.png" alt="Visio Mobile" width="500" />
</p>
```

Source: `SVG/transparent light.png` resized to 1000px wide → `docs/screenshots/visio-mobile-banner.png`

### App Icon

Replace `docs/screenshots/app_icon.png` with `SVG/transparent Dark.png` resized to 256x256.

## 5. Fastlane Metadata — Android

### Directory Structure

```
android/fastlane/metadata/android/
├── fr-FR/
│   ├── title.txt
│   ├── short_description.txt
│   ├── full_description.txt
│   └── changelogs/
│       └── default.txt
└── en-US/
    ├── title.txt
    ├── short_description.txt
    ├── full_description.txt
    └── changelogs/
        └── default.txt
```

> **Note:** The Play Store hi-res icon (512x512) is set via the Google Play Console, not through `supply` metadata. The `images/` directory under `supply` is for screenshots and feature graphics only. The 512x512 icon from `WHITE/Google Play Market/` should be uploaded manually in the Play Console "Store listing" section.

### Content

**title.txt:** "Visio Mobile"

**short_description.txt (FR, ≤80 chars):**
"Visioconférence souveraine — client natif pour La Suite Meet"

**short_description.txt (EN, ≤80 chars):**
"Sovereign video conferencing — native client for La Suite Meet"

**full_description.txt (FR):**
Visio Mobile est le client natif de visioconférence pour La Suite Meet (meet.numerique.gouv.fr). Rejoignez des salles de réunion directement depuis votre appareil, sans navigateur.

Fonctionnalités :
- Appels audio et vidéo en temps réel
- Chat intégré pendant les réunions
- Liste des participants
- Partage de lien de salle
- Authentification OIDC / ProConnect
- Création de salles (publiques, de confiance, restreintes)

Visio Mobile est un logiciel libre (open source) construit sur le SDK LiveKit.

**full_description.txt (EN):**
Visio Mobile is the native video conferencing client for La Suite Meet (meet.numerique.gouv.fr). Join meeting rooms directly from your device, no browser needed.

Features:
- Real-time audio and video calls
- In-meeting chat
- Participant list
- Room link sharing
- OIDC / ProConnect authentication
- Room creation (public, trusted, restricted)

Visio Mobile is free and open-source software built on the LiveKit SDK.

**changelogs/default.txt:** "Initial release"

### Release Lane

Add to `android/fastlane/Fastfile`:

```ruby
lane :release do
  gradle(
    task: "bundle",
    build_type: "Release",
    project_dir: "."
  )
  supply(
    track: "internal",
    aab: "app/build/outputs/bundle/release/app-release.aab",
    json_key: ENV["SUPPLY_JSON_KEY_PATH"],
    package_name: "io.visio.mobile"
  )
end
```

> **Rollout strategy:** The lane targets `internal` track for initial validation. Progression to production: change `track:` to `"alpha"`, `"beta"`, or `"production"` as appropriate. Google Play also supports staged rollouts via `rollout: "0.1"` (10%).

Requires: `SUPPLY_JSON_KEY_PATH` environment variable pointing to Google Play service account JSON.

## 6. Fastlane Metadata — iOS

### Directory Structure

```
ios/fastlane/metadata/
├── fr-FR/
│   ├── name.txt
│   ├── subtitle.txt
│   ├── description.txt
│   ├── keywords.txt
│   ├── promotional_text.txt
│   ├── privacy_url.txt
│   ├── support_url.txt
│   └── release_notes.txt
└── en-US/
    ├── name.txt
    ├── subtitle.txt
    ├── description.txt
    ├── keywords.txt
    ├── promotional_text.txt
    ├── privacy_url.txt
    ├── support_url.txt
    └── release_notes.txt
```

### Content

**name.txt:** "Visio Mobile"

**subtitle.txt (FR, ≤30 chars):** "Visioconférence souveraine"
**subtitle.txt (EN, ≤30 chars):** "Sovereign video meetings"

**keywords.txt (FR, ≤100 chars):**
"visioconférence,réunion,vidéo,appel,La Suite,Meet,souverain,chat,LiveKit"

**keywords.txt (EN, ≤100 chars):**
"video,conferencing,meeting,call,La Suite,Meet,sovereign,chat,LiveKit,open source"

**promotional_text.txt (FR, ≤170 chars):**
"Rejoignez vos réunions La Suite Meet directement depuis votre iPhone ou iPad. Audio, vidéo et chat en temps réel, sans navigateur."

**promotional_text.txt (EN, ≤170 chars):**
"Join your La Suite Meet meetings directly from your iPhone or iPad. Real-time audio, video and chat, no browser needed."

**description.txt:** Same content as Android full_description (adapted slightly for iOS tone).

**privacy_url.txt:** TBD (user to provide)
**support_url.txt:** TBD (user to provide, likely GitHub repo URL)

**release_notes.txt:** "Initial release"

### Release Lane

Add to `ios/fastlane/Fastfile` (mirrors existing `distribute` lane pattern for API key and signing):

```ruby
lane :release do
  setup_ci

  api_key = app_store_connect_api_key(
    key_id: ENV["APP_STORE_CONNECT_API_KEY_ID"],
    issuer_id: ENV["APP_STORE_CONNECT_ISSUER_ID"],
    key_content: ENV["APP_STORE_CONNECT_API_KEY_CONTENT"],
    is_key_content_base64: true
  )

  match(
    type: "appstore",
    app_identifier: "io.visio.mobile",
    git_url: ENV["MATCH_GIT_URL"],
    api_key: api_key,
    readonly: true
  )

  increment_build_number(
    build_number: ENV["GITHUB_RUN_NUMBER"] || "1",
    xcodeproj: "VisioMobile.xcodeproj"
  )

  gym(
    scheme: "VisioMobile",
    project: "VisioMobile.xcodeproj",
    export_method: "app-store",
    output_directory: "build",
    output_name: "VisioMobile.ipa",
    clean: true,
    xcargs: "DEVELOPMENT_TEAM='#{ENV["APPLE_TEAM_ID"]}' CODE_SIGN_STYLE=Manual PROVISIONING_PROFILE_SPECIFIER='match AppStore io.visio.mobile' CODE_SIGN_IDENTITY='iPhone Distribution'",
    export_options: {
      teamID: ENV["APPLE_TEAM_ID"],
      signingStyle: "manual",
      provisioningProfiles: {
        "io.visio.mobile" => "match AppStore io.visio.mobile"
      }
    }
  )

  deliver(
    api_key: api_key,
    ipa: "build/VisioMobile.ipa",
    submit_for_review: false,
    force: true,
    metadata_path: "./fastlane/metadata",
    skip_screenshots: true
  )
end
```

> **Note on screenshots:** `skip_screenshots: true` is set for initial submissions. Apple requires at least one screenshot per device class for the first App Store submission — these must be uploaded manually via App Store Connect or added to `metadata/{locale}/screenshots/` and `skip_screenshots` set to `false`.

Requires: same env vars as existing `distribute` lane (`APP_STORE_CONNECT_API_KEY_ID`, `APP_STORE_CONNECT_ISSUER_ID`, `APP_STORE_CONNECT_API_KEY_CONTENT`, `APPLE_TEAM_ID`, `MATCH_GIT_URL`).

## 7. Dependencies & Prerequisites

### Tools Required (on build machine)
- `sips` (macOS built-in) — image resizing
- `iconutil` (macOS built-in) — .icns generation
- ImageMagick `convert` — .ico generation (install via `brew install imagemagick` if missing)
- `bundle exec fastlane` — Fastlane execution

### Credentials Required (user to configure)
- **Google Play:** Service account JSON key with "Release manager" role
- **App Store Connect:** API key (App Manager role minimum)
- **Match:** Git repo for iOS certificates (already referenced in Fastfile)

### Store Prerequisites (manual steps before first submission)
- **Google Play:** Complete IARC content rating questionnaire, set app category to "Communication", upload feature graphic (1024x500) and screenshots (min 2 phone screenshots), provide privacy policy URL
- **App Store:** Complete age rating form, set app category to "Social Networking" or "Business", upload screenshots per device class (6.7", 6.5", 5.5" iPhone minimum; iPad if universal), provide privacy policy URL (hard requirement)
- **Privacy policy URL:** Must be provided by user before any store submission (`privacy_url.txt` is TBD in metadata)
- **Feature graphic (Google Play):** 1024x500 banner image — not included in current asset pack, needs to be designed or derived from the logo assets

## 8. Summary of Changes

| Area | Files Changed | Files Created |
|------|--------------|---------------|
| Android icons | 10 PNGs replaced + 5 foreground regenerated | — |
| Android store | 1 PNG replaced | — |
| iOS icons | 37 PNGs + Contents.json replaced | — |
| Desktop icons | 6 files replaced | icon.icns, icon.ico regenerated |
| Desktop frontend | 2 files replaced | — |
| README | README.md edited | visio-mobile-banner.png, app_icon.png replaced |
| Android Fastlane | Fastfile modified | metadata/ tree (10 files) |
| iOS Fastlane | Fastfile modified | metadata/ tree (16 files) |
