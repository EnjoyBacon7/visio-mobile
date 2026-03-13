#!/usr/bin/env bash
# Download a test video for E2E bot usage.
# Requires: yt-dlp, ffmpeg
#
# Usage: ./scripts/download-test-media.sh [youtube-url]
#   Default: https://www.youtube.com/watch?v=EKWx-87CoVc

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
ASSETS_DIR="$ROOT_DIR/test-assets"

URL="${1:-https://www.youtube.com/watch?v=EKWx-87CoVc}"
OUTPUT="$ASSETS_DIR/test-video.mp4"

command -v yt-dlp >/dev/null 2>&1 || { echo "yt-dlp required: brew install yt-dlp"; exit 1; }
command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg required: brew install ffmpeg"; exit 1; }

mkdir -p "$ASSETS_DIR"

if [ -f "$OUTPUT" ]; then
    echo "test-video.mp4 already exists, skipping download."
    echo "Delete $OUTPUT to re-download."
else
    echo "Downloading test video..."
    yt-dlp -f "bestvideo[height<=480]+bestaudio/best[height<=480]" \
        --merge-output-format mp4 \
        -o "$OUTPUT" \
        "$URL"
    echo "Downloaded: $OUTPUT"
fi

# Show info
echo ""
ffprobe -hide_banner -show_entries format=duration,size -show_entries stream=codec_type,width,height,sample_rate,channels \
    -of compact "$OUTPUT" 2>/dev/null || true
echo ""
echo "Usage: cargo run -p visio-bot -- --media-file $OUTPUT --url ws://localhost:7880 --room test"
