# Rebranding & Store Publication — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all app icons with the new WHITE identity across iOS, Android, and Desktop; set up Fastlane metadata and release lanes for App Store and Google Play; update the README with the new branding.

**Architecture:** File-copy driven — new icon assets from `/Users/mmaudet/work/Logo Visio Mobile/WHITE/` are copied/generated into the correct platform locations. Fastlane metadata directories and release lanes are added alongside existing beta distribution infrastructure.

**Tech Stack:** Fastlane (supply, deliver), sips/iconutil (macOS), ImageMagick (ico), Bash

**Spec:** `docs/superpowers/specs/2026-03-14-rebranding-store-publication-design.md`

---

## Chunk 1: Android Icons

### Task 1: Replace Android launcher icons (all densities)

**Files:**
- Replace: `android/app/src/main/res/mipmap-mdpi/ic_launcher.png`
- Replace: `android/app/src/main/res/mipmap-mdpi/ic_launcher_round.png`
- Replace: `android/app/src/main/res/mipmap-hdpi/ic_launcher.png`
- Replace: `android/app/src/main/res/mipmap-hdpi/ic_launcher_round.png`
- Replace: `android/app/src/main/res/mipmap-xhdpi/ic_launcher.png`
- Replace: `android/app/src/main/res/mipmap-xhdpi/ic_launcher_round.png`
- Replace: `android/app/src/main/res/mipmap-xxhdpi/ic_launcher.png`
- Replace: `android/app/src/main/res/mipmap-xxhdpi/ic_launcher_round.png`
- Replace: `android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png`
- Replace: `android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_round.png`

**Source directory:** `/Users/mmaudet/work/Logo Visio Mobile/WHITE/android/`

- [ ] **Step 1: Copy launcher icons for each density**

```bash
SRC="/Users/mmaudet/work/Logo Visio Mobile/WHITE/android"
DST="/Users/mmaudet/work/visio-mobile-v2/android/app/src/main/res"

cp "$SRC/icon-48x48.png"   "$DST/mipmap-mdpi/ic_launcher.png"
cp "$SRC/icon-48x48.png"   "$DST/mipmap-mdpi/ic_launcher_round.png"
cp "$SRC/icon-72x72.png"   "$DST/mipmap-hdpi/ic_launcher.png"
cp "$SRC/icon-72x72.png"   "$DST/mipmap-hdpi/ic_launcher_round.png"
cp "$SRC/icon-96x96.png"   "$DST/mipmap-xhdpi/ic_launcher.png"
cp "$SRC/icon-96x96.png"   "$DST/mipmap-xhdpi/ic_launcher_round.png"
cp "$SRC/icon-144x144.png" "$DST/mipmap-xxhdpi/ic_launcher.png"
cp "$SRC/icon-144x144.png" "$DST/mipmap-xxhdpi/ic_launcher_round.png"
cp "$SRC/icon-192x192.png" "$DST/mipmap-xxxhdpi/ic_launcher.png"
cp "$SRC/icon-192x192.png" "$DST/mipmap-xxxhdpi/ic_launcher_round.png"
```

- [ ] **Step 2: Verify file sizes match source**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
for density in mdpi hdpi xhdpi xxhdpi xxxhdpi; do
  echo "--- $density ---"
  file "android/app/src/main/res/mipmap-$density/ic_launcher.png"
done
```

Expected: Each file shows correct PNG dimensions (48, 72, 96, 144, 192).

### Task 2: Generate adaptive icon foreground layers

**Files:**
- Replace: `android/app/src/main/res/mipmap-mdpi/ic_launcher_foreground.png`
- Replace: `android/app/src/main/res/mipmap-hdpi/ic_launcher_foreground.png`
- Replace: `android/app/src/main/res/mipmap-xhdpi/ic_launcher_foreground.png`
- Replace: `android/app/src/main/res/mipmap-xxhdpi/ic_launcher_foreground.png`
- Replace: `android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_foreground.png`

Adaptive icons use a 108dp canvas with the logo in the inner 66% (72dp). The source 512x512 logo already has some padding. We place it on a transparent canvas at the adaptive icon sizes with appropriate safe zone.

- [ ] **Step 1: Generate foreground PNGs with safe zone padding**

```bash
SRC="/Users/mmaudet/work/Logo Visio Mobile/WHITE/android/icon-512x512.png"
DST="/Users/mmaudet/work/visio-mobile-v2/android/app/src/main/res"
TMPDIR=$(mktemp -d)

# For each density: canvas_size = 108 * scale_factor
# mdpi=108, hdpi=162, xhdpi=216, xxhdpi=324, xxxhdpi=432
# Logo occupies inner 66% → logo_size = canvas * 0.66
declare -A SIZES=( [mdpi]=108 [hdpi]=162 [xhdpi]=216 [xxhdpi]=324 [xxxhdpi]=432 )

for density in "${!SIZES[@]}"; do
  canvas=${SIZES[$density]}
  logo_size=$(echo "$canvas * 0.66" | bc | cut -d. -f1)
  offset=$(( (canvas - logo_size) / 2 ))

  # Create transparent canvas, composite logo centered
  convert -size ${canvas}x${canvas} xc:transparent \
    \( "$SRC" -resize ${logo_size}x${logo_size} \) \
    -gravity center -composite \
    "$DST/mipmap-$density/ic_launcher_foreground.png"
done
```

- [ ] **Step 2: Verify foreground dimensions**

```bash
DST="/Users/mmaudet/work/visio-mobile-v2/android/app/src/main/res"
for density in mdpi hdpi xhdpi xxhdpi xxxhdpi; do
  file "$DST/mipmap-$density/ic_launcher_foreground.png"
done
```

Expected: 108x108, 162x162, 216x216, 324x324, 432x432.

### Task 3: Verify adaptive icon background is white

**Files:**
- Verify: `android/app/src/main/res/values/ic_launcher_background.xml`

- [ ] **Step 1: Confirm background color is #FFFFFF**

Read `android/app/src/main/res/values/ic_launcher_background.xml` and verify it contains `#FFFFFF`. (Current value is already `#FFFFFF` — no change needed.)

### Task 4: Replace Play Store icon

**Files:**
- Replace: `android/app/src/main/assets/icons/icon-playstore-512.png`

- [ ] **Step 1: Copy Play Store icon**

```bash
cp "/Users/mmaudet/work/Logo Visio Mobile/WHITE/Google Play Market/icon-512x512.png" \
   "/Users/mmaudet/work/visio-mobile-v2/android/app/src/main/assets/icons/icon-playstore-512.png"
```

- [ ] **Step 2: Verify**

```bash
file /Users/mmaudet/work/visio-mobile-v2/android/app/src/main/assets/icons/icon-playstore-512.png
```

Expected: PNG image data, 512 x 512.

### Task 5: Commit Android icons

- [ ] **Step 1: Commit**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
git add android/app/src/main/res/mipmap-*/ic_launcher*.png \
        android/app/src/main/assets/icons/icon-playstore-512.png
git commit -m "rebrand: replace Android launcher icons with new WHITE identity"
```

---

## Chunk 2: iOS Icons

### Task 6: Replace iOS AppIcon.appiconset

**Files:**
- Delete + recreate: `ios/VisioMobile/Assets.xcassets/AppIcon.appiconset/` (all PNGs + Contents.json)

The current appiconset uses naming convention `icon_20_2x.png` while the new assets use `icon-40x40.png`. The new `Contents.json` from `WHITE/ios/` maps to the new naming. Clean replacement is safest.

- [ ] **Step 1: Remove all existing icon PNGs from appiconset**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
rm -f ios/VisioMobile/Assets.xcassets/AppIcon.appiconset/*.png
```

- [ ] **Step 2: Copy all 37 new PNGs + Contents.json**

```bash
SRC="/Users/mmaudet/work/Logo Visio Mobile/WHITE/ios"
DST="/Users/mmaudet/work/visio-mobile-v2/ios/VisioMobile/Assets.xcassets/AppIcon.appiconset"

cp "$SRC"/*.png "$DST/"
cp "$SRC/Contents.json" "$DST/Contents.json"
```

- [ ] **Step 3: Verify Contents.json references exist**

```bash
DST="/Users/mmaudet/work/visio-mobile-v2/ios/VisioMobile/Assets.xcassets/AppIcon.appiconset"
# Extract filenames from Contents.json and check they exist
python3 -c "
import json, os, sys
with open('$DST/Contents.json') as f:
    data = json.load(f)
missing = []
for img in data['images']:
    fn = img.get('filename', '')
    if fn and not os.path.exists(os.path.join('$DST', fn)):
        missing.append(fn)
if missing:
    print('MISSING:', missing)
    sys.exit(1)
else:
    print(f'OK: all {len([i for i in data[\"images\"] if i.get(\"filename\")])} referenced files exist')
"
```

Expected: "OK: all 18 referenced files exist" (Contents.json references 18 entries, some filenames shared across idioms).

### Task 7: Replace AppLogo imageset

**Files:**
- Replace: `ios/VisioMobile/Assets.xcassets/AppLogo.imageset/logo.png`

- [ ] **Step 1: Generate resized logo from transparent dark icon**

```bash
sips -z 256 256 "/Users/mmaudet/work/Logo Visio Mobile/SVG/transparent Dark.png" \
  --out "/Users/mmaudet/work/visio-mobile-v2/ios/VisioMobile/Assets.xcassets/AppLogo.imageset/logo.png"
```

- [ ] **Step 2: Verify**

```bash
file /Users/mmaudet/work/visio-mobile-v2/ios/VisioMobile/Assets.xcassets/AppLogo.imageset/logo.png
```

Expected: PNG image data, 256 x 256.

### Task 8: Commit iOS icons

- [ ] **Step 1: Commit**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
git add ios/VisioMobile/Assets.xcassets/AppIcon.appiconset/
git add ios/VisioMobile/Assets.xcassets/AppLogo.imageset/logo.png
git commit -m "rebrand: replace iOS app icons with new WHITE identity"
```

---

## Chunk 3: Desktop Icons

### Task 9: Replace desktop PNG icons

**Files:**
- Replace: `crates/visio-desktop/icons/icon.png`
- Replace: `crates/visio-desktop/icons/32x32.png`
- Replace: `crates/visio-desktop/icons/128x128.png`
- Replace: `crates/visio-desktop/icons/128x128@2x.png`
- Replace: `crates/visio-desktop/frontend/public/icon.png`

- [ ] **Step 1: Copy web icons to desktop locations**

```bash
SRC="/Users/mmaudet/work/Logo Visio Mobile/WHITE/web"
DST="/Users/mmaudet/work/visio-mobile-v2/crates/visio-desktop"

cp "$SRC/icon-256x256.png" "$DST/icons/icon.png"
cp "$SRC/icon-32x32.png"   "$DST/icons/32x32.png"
cp "$SRC/icon-128x128.png" "$DST/icons/128x128.png"
cp "$SRC/icon-256x256.png" "$DST/icons/128x128@2x.png"
cp "$SRC/icon-128x128.png" "$DST/frontend/public/icon.png"
```

### Task 10: Generate icon.icns (macOS)

**Files:**
- Replace: `crates/visio-desktop/icons/icon.icns`

- [ ] **Step 1: Create iconset and generate .icns**

```bash
SRC="/Users/mmaudet/work/Logo Visio Mobile/WHITE/App Store/icon-1024x1024.png"
ICONSET=$(mktemp -d)/icon.iconset
mkdir -p "$ICONSET"

# Generate all required sizes for .icns
sips -z 16 16     "$SRC" --out "$ICONSET/icon_16x16.png"
sips -z 32 32     "$SRC" --out "$ICONSET/icon_16x16@2x.png"
sips -z 32 32     "$SRC" --out "$ICONSET/icon_32x32.png"
sips -z 64 64     "$SRC" --out "$ICONSET/icon_32x32@2x.png"
sips -z 128 128   "$SRC" --out "$ICONSET/icon_128x128.png"
sips -z 256 256   "$SRC" --out "$ICONSET/icon_128x128@2x.png"
sips -z 256 256   "$SRC" --out "$ICONSET/icon_256x256.png"
sips -z 512 512   "$SRC" --out "$ICONSET/icon_256x256@2x.png"
sips -z 512 512   "$SRC" --out "$ICONSET/icon_512x512.png"
sips -z 1024 1024 "$SRC" --out "$ICONSET/icon_512x512@2x.png"

iconutil --convert icns "$ICONSET" \
  --output "/Users/mmaudet/work/visio-mobile-v2/crates/visio-desktop/icons/icon.icns"
```

- [ ] **Step 2: Verify .icns was generated**

```bash
file /Users/mmaudet/work/visio-mobile-v2/crates/visio-desktop/icons/icon.icns
```

Expected: "Mac OS X icon, ..."

### Task 11: Generate icon.ico (Windows)

**Files:**
- Replace: `crates/visio-desktop/icons/icon.ico`

- [ ] **Step 1: Generate .ico with multiple sizes using ImageMagick**

```bash
SRC="/Users/mmaudet/work/Logo Visio Mobile/WHITE/App Store/icon-1024x1024.png"
DST="/Users/mmaudet/work/visio-mobile-v2/crates/visio-desktop/icons/icon.ico"

convert "$SRC" \
  \( -clone 0 -resize 16x16 \) \
  \( -clone 0 -resize 32x32 \) \
  \( -clone 0 -resize 48x48 \) \
  \( -clone 0 -resize 64x64 \) \
  \( -clone 0 -resize 128x128 \) \
  \( -clone 0 -resize 256x256 \) \
  -delete 0 "$DST"
```

- [ ] **Step 2: Verify .ico**

```bash
file /Users/mmaudet/work/visio-mobile-v2/crates/visio-desktop/icons/icon.ico
```

Expected: "MS Windows icon resource"

### Task 12: Replace frontend logo

**Files:**
- Replace: `crates/visio-desktop/frontend/public/logo.png`

- [ ] **Step 1: Resize transparent light banner to ~400px wide**

```bash
sips -Z 400 "/Users/mmaudet/work/Logo Visio Mobile/SVG/transparent light.png" \
  --out "/Users/mmaudet/work/visio-mobile-v2/crates/visio-desktop/frontend/public/logo.png"
```

(`-Z` resizes to fit within a box preserving aspect ratio.)

### Task 13: Commit desktop icons

- [ ] **Step 1: Commit**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
git add crates/visio-desktop/icons/ crates/visio-desktop/frontend/public/icon.png crates/visio-desktop/frontend/public/logo.png
git commit -m "rebrand: replace desktop icons (PNG, ICNS, ICO) with new WHITE identity"
```

---

## Chunk 4: README Update

### Task 14: Create banner and update app_icon

**Files:**
- Create: `docs/screenshots/visio-mobile-banner.png`
- Replace: `docs/screenshots/app_icon.png`

- [ ] **Step 1: Generate README banner from transparent light**

```bash
sips -Z 1000 "/Users/mmaudet/work/Logo Visio Mobile/SVG/transparent light.png" \
  --out "/Users/mmaudet/work/visio-mobile-v2/docs/screenshots/visio-mobile-banner.png"
```

- [ ] **Step 2: Replace app_icon.png with new logo**

```bash
sips -z 256 256 "/Users/mmaudet/work/Logo Visio Mobile/SVG/transparent Dark.png" \
  --out "/Users/mmaudet/work/visio-mobile-v2/docs/screenshots/app_icon.png"
```

### Task 15: Update README.md header

**Files:**
- Modify: `README.md` (lines 1-3)

- [ ] **Step 1: Replace the header image tag**

Replace:
```markdown
<p align="center">
  <img src="docs/screenshots/app_icon.png" alt="Visio Mobile" width="128" />
</p>
```

With:
```markdown
<p align="center">
  <img src="docs/screenshots/visio-mobile-banner.png" alt="Visio Mobile" width="500" />
</p>
```

### Task 16: Commit README changes

- [ ] **Step 1: Commit**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
git add docs/screenshots/visio-mobile-banner.png docs/screenshots/app_icon.png README.md
git commit -m "rebrand: update README with new logo banner"
```

---

## Chunk 5: Fastlane Metadata — Android

### Task 17: Create Android metadata directory structure

**Files:**
- Create: `android/fastlane/metadata/android/fr-FR/title.txt`
- Create: `android/fastlane/metadata/android/fr-FR/short_description.txt`
- Create: `android/fastlane/metadata/android/fr-FR/full_description.txt`
- Create: `android/fastlane/metadata/android/fr-FR/changelogs/default.txt`
- Create: `android/fastlane/metadata/android/en-US/title.txt`
- Create: `android/fastlane/metadata/android/en-US/short_description.txt`
- Create: `android/fastlane/metadata/android/en-US/full_description.txt`
- Create: `android/fastlane/metadata/android/en-US/changelogs/default.txt`

- [ ] **Step 1: Create directory structure**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
mkdir -p android/fastlane/metadata/android/fr-FR/changelogs
mkdir -p android/fastlane/metadata/android/en-US/changelogs
```

- [ ] **Step 2: Write FR metadata files**

`android/fastlane/metadata/android/fr-FR/title.txt`:
```
Visio Mobile
```

`android/fastlane/metadata/android/fr-FR/short_description.txt`:
```
Visioconférence souveraine — client natif pour La Suite Meet
```

`android/fastlane/metadata/android/fr-FR/full_description.txt`:
```
Visio Mobile est le client natif de visioconférence pour La Suite Meet (meet.numerique.gouv.fr). Rejoignez des salles de réunion directement depuis votre appareil, sans navigateur.

Fonctionnalités :
- Appels audio et vidéo en temps réel
- Chat intégré pendant les réunions
- Liste des participants
- Partage de lien de salle
- Authentification OIDC / ProConnect
- Création de salles (publiques, de confiance, restreintes)

Visio Mobile est un logiciel libre (open source) construit sur le SDK LiveKit.
```

`android/fastlane/metadata/android/fr-FR/changelogs/default.txt`:
```
Version initiale
```

- [ ] **Step 3: Write EN metadata files**

`android/fastlane/metadata/android/en-US/title.txt`:
```
Visio Mobile
```

`android/fastlane/metadata/android/en-US/short_description.txt`:
```
Sovereign video conferencing — native client for La Suite Meet
```

`android/fastlane/metadata/android/en-US/full_description.txt`:
```
Visio Mobile is the native video conferencing client for La Suite Meet (meet.numerique.gouv.fr). Join meeting rooms directly from your device, no browser needed.

Features:
- Real-time audio and video calls
- In-meeting chat
- Participant list
- Room link sharing
- OIDC / ProConnect authentication
- Room creation (public, trusted, restricted)

Visio Mobile is free and open-source software built on the LiveKit SDK.
```

`android/fastlane/metadata/android/en-US/changelogs/default.txt`:
```
Initial release
```

### Task 18: Add Android release lane

**Files:**
- Modify: `android/fastlane/Fastfile`

- [ ] **Step 1: Add release lane after existing distribute lane**

Append before the final `end`:

```ruby
  desc "Build AAB and upload to Google Play (internal track)"
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

The full file should be:

```ruby
default_platform(:android)

platform :android do
  desc "Build release APK and distribute via Firebase App Distribution"
  lane :distribute do
    gradle(
      task: "assembleRelease",
      project_dir: "."
    )

    firebase_app_distribution(
      app: ENV["FIREBASE_APP_ID"],
      service_credentials_file: ENV["GOOGLE_APPLICATION_CREDENTIALS"],
      groups: "testers",
      release_notes: ENV["RELEASE_NOTES"].to_s.empty? ? "Build #{ENV['GITHUB_RUN_NUMBER'] || 'local'} — #{`git log -1 --pretty=%s`.strip}" : ENV["RELEASE_NOTES"]
    )
  end

  desc "Build AAB and upload to Google Play (internal track)"
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
end
```

### Task 19: Commit Android Fastlane metadata

- [ ] **Step 1: Commit**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
git add android/fastlane/metadata/ android/fastlane/Fastfile
git commit -m "feat: add Android Fastlane metadata (FR/EN) and release lane for Google Play"
```

---

## Chunk 6: Fastlane Metadata — iOS

### Task 20: Create iOS metadata directory structure

**Files:**
- Create: `ios/fastlane/metadata/fr-FR/name.txt`
- Create: `ios/fastlane/metadata/fr-FR/subtitle.txt`
- Create: `ios/fastlane/metadata/fr-FR/description.txt`
- Create: `ios/fastlane/metadata/fr-FR/keywords.txt`
- Create: `ios/fastlane/metadata/fr-FR/promotional_text.txt`
- Create: `ios/fastlane/metadata/fr-FR/privacy_url.txt`
- Create: `ios/fastlane/metadata/fr-FR/support_url.txt`
- Create: `ios/fastlane/metadata/fr-FR/release_notes.txt`
- Create: `ios/fastlane/metadata/en-US/name.txt`
- Create: `ios/fastlane/metadata/en-US/subtitle.txt`
- Create: `ios/fastlane/metadata/en-US/description.txt`
- Create: `ios/fastlane/metadata/en-US/keywords.txt`
- Create: `ios/fastlane/metadata/en-US/promotional_text.txt`
- Create: `ios/fastlane/metadata/en-US/privacy_url.txt`
- Create: `ios/fastlane/metadata/en-US/support_url.txt`
- Create: `ios/fastlane/metadata/en-US/release_notes.txt`

- [ ] **Step 1: Create directory structure**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
mkdir -p ios/fastlane/metadata/fr-FR
mkdir -p ios/fastlane/metadata/en-US
```

- [ ] **Step 2: Write FR metadata files**

`ios/fastlane/metadata/fr-FR/name.txt`:
```
Visio Mobile
```

`ios/fastlane/metadata/fr-FR/subtitle.txt`:
```
Visioconférence souveraine
```

`ios/fastlane/metadata/fr-FR/description.txt`:
```
Visio Mobile est le client natif de visioconférence pour La Suite Meet (meet.numerique.gouv.fr). Rejoignez des salles de réunion directement depuis votre iPhone ou iPad, sans navigateur.

Fonctionnalités :
- Appels audio et vidéo en temps réel
- Chat intégré pendant les réunions
- Liste des participants
- Partage de lien de salle
- Authentification OIDC / ProConnect
- Création de salles (publiques, de confiance, restreintes)

Visio Mobile est un logiciel libre (open source) construit sur le SDK LiveKit.
```

`ios/fastlane/metadata/fr-FR/keywords.txt`:
```
visioconférence,réunion,vidéo,appel,La Suite,Meet,souverain,chat,LiveKit
```

`ios/fastlane/metadata/fr-FR/promotional_text.txt`:
```
Rejoignez vos réunions La Suite Meet directement depuis votre iPhone ou iPad. Audio, vidéo et chat en temps réel, sans navigateur.
```

`ios/fastlane/metadata/fr-FR/privacy_url.txt`:
```
```
(Empty — TBD, must be provided before first App Store submission)

`ios/fastlane/metadata/fr-FR/support_url.txt`:
```
```
(Empty — TBD, likely GitHub repo URL)

`ios/fastlane/metadata/fr-FR/release_notes.txt`:
```
Version initiale
```

- [ ] **Step 3: Write EN metadata files**

`ios/fastlane/metadata/en-US/name.txt`:
```
Visio Mobile
```

`ios/fastlane/metadata/en-US/subtitle.txt`:
```
Sovereign video meetings
```

`ios/fastlane/metadata/en-US/description.txt`:
```
Visio Mobile is the native video conferencing client for La Suite Meet (meet.numerique.gouv.fr). Join meeting rooms directly from your iPhone or iPad, no browser needed.

Features:
- Real-time audio and video calls
- In-meeting chat
- Participant list
- Room link sharing
- OIDC / ProConnect authentication
- Room creation (public, trusted, restricted)

Visio Mobile is free and open-source software built on the LiveKit SDK.
```

`ios/fastlane/metadata/en-US/keywords.txt`:
```
video,conferencing,meeting,call,La Suite,Meet,sovereign,chat,LiveKit,open source
```

`ios/fastlane/metadata/en-US/promotional_text.txt`:
```
Join your La Suite Meet meetings directly from your iPhone or iPad. Real-time audio, video and chat, no browser needed.
```

`ios/fastlane/metadata/en-US/privacy_url.txt`:
```
```
(Empty — TBD)

`ios/fastlane/metadata/en-US/support_url.txt`:
```
```
(Empty — TBD)

`ios/fastlane/metadata/en-US/release_notes.txt`:
```
Initial release
```

### Task 21: Add iOS release lane

**Files:**
- Modify: `ios/fastlane/Fastfile`

- [ ] **Step 1: Add release lane after existing distribute lane**

The full file should be:

```ruby
default_platform(:ios)

platform :ios do
  desc "Build and upload to TestFlight"
  lane :distribute do
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

    pilot(
      api_key: api_key,
      ipa: "build/VisioMobile.ipa",
      distribute_external: true,
      groups: ["Beta Testers"],
      changelog: ENV["RELEASE_NOTES"].to_s.empty? ? "Build #{ENV['GITHUB_RUN_NUMBER'] || 'local'} — #{`git log -1 --pretty=%s`.strip}" : ENV["RELEASE_NOTES"]
    )
  end

  desc "Build and upload to App Store Connect (metadata + binary)"
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
      app_identifier: "io.visio.mobile",
      submit_for_review: false,
      force: true,
      metadata_path: "./fastlane/metadata",
      skip_screenshots: true
    )
  end
end
```

### Task 22: Commit iOS Fastlane metadata

- [ ] **Step 1: Commit**

```bash
cd /Users/mmaudet/work/visio-mobile-v2
git add ios/fastlane/metadata/ ios/fastlane/Fastfile
git commit -m "feat: add iOS Fastlane metadata (FR/EN) and release lane for App Store"
```
