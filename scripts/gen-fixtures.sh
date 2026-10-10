#!/usr/bin/env bash
# Generate the benchmark fixtures with FFmpeg's lavfi sources (§16).
#
# Media is generated, never committed: a 10-minute 4K HEVC file would be
# several gigabytes and would bloat every clone for no benefit. The scripts
# below are deterministic, so the same fixture comes back on every machine.
#
# Usage: scripts/gen-fixtures.sh [outdir] [--small]
#   --small  Generate short proxies of each fixture (2s) for quick test runs.
set -euo pipefail

OUT="${1:-tests/fixtures/generated}"
SMALL=0
[ "${2:-}" = "--small" ] && SMALL=1

FFMPEG="${FFMPEG:-ffmpeg}"
command -v "$FFMPEG" >/dev/null 2>&1 || { echo "ffmpeg not found" >&2; exit 1; }

mkdir -p "$OUT"

# Duration helper: full fixtures are long, --small makes 2-second versions so
# the harness can be exercised without waiting.
dur() { [ "$SMALL" = "1" ] && echo 2 || echo "$1"; }

# A per-frame marker pattern so golden tests can identify a frame by its
# content (§11.4). `drawtext` needs a font; fall back to a moving box when
# none is available so the fixtures still generate on a bare container.
if "$FFMPEG" -hide_banner -filters 2>/dev/null | grep -q drawtext; then
  MARKER="drawtext=text='%{pts\\:hms}':x=10:y=10:fontsize=48:fontcolor=white:box=1:boxcolor=black@0.6"
  SRC="testsrc2=size=${2:-1920x1080}:rate=${3:-30}:duration=${4}"
else
  echo "warning: no drawtext filter; fixtures will lack per-frame markers" >&2
  MARKER=""
  SRC="testsrc2=size=${2:-1920x1080}:rate=${3:-30}:duration=${4}"
fi

gen() {
  local name="$1" size="$2" rate="$3" seconds="$4" vcodec="$5" extra="${6:-}"
  local out="$OUT/$name"
  echo "-> $out ($size @ $rate, $(dur "$seconds")s, $vcodec)"
  # shellcheck disable=SC2086
  "$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "$SRC" \
    $([ -n "$MARKER" ] && echo "-vf $MARKER") \
    -c:v "$vcodec" $extra \
    -pix_fmt yuv420p \
    -t "$(dur "$seconds")" \
    "$out" 2>&1 | sed 's/^/   /' || {
      echo "   (encoder $vcodec unavailable, falling back to mpeg4)" >&2
      "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$SRC" \
        -c:v mpeg4 -q:v 4 -t "$(dur "$seconds")" "$out"
    }
}

# §16 benchmark scenario 1: a 10-minute 1080p H.264, a 10-minute 4K HEVC, and
# a 5-minute 1080p60. These are the sources the whole benchmark runs against.
gen "1080p_h264_10min.mp4"  1920x1080 30 "$(dur 600)" libx264  "-preset veryfast -crf 23"
gen "4k_hevc_10min.mp4"     3840x2160 30 "$(dur 600)" libx265  "-preset ultrafast -crf 32"
gen "1080p60_h264_5min.mp4" 1920x1080 60 "$(dur 300)" libx264  "-preset veryfast -crf 22"

# A still-image source for thumbnail and overlay-stream tests.
echo "-> $OUT/thumb_source.mkv"
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=640x360:rate=1:duration=$(dur 10)" \
  -c:v ffv1 "$OUT/thumb_source.mkv"

# A 1 kHz tone for audio-only paths (mixer, loudness, peaks).
echo "-> $OUT/tone_1khz.wav"
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000:duration=$(dur 10)" \
  -ac 2 "$OUT/tone_1khz.wav"

echo
echo "Fixtures generated in $OUT:"
ls -la "$OUT"
echo
echo "Note: these are intentionally untracked (see .gitignore). Regenerate"
echo "with scripts/gen-fixtures.sh rather than committing media."