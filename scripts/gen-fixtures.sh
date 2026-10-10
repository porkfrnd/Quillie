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
# content (§11.4). `drawtext` needs a font; when it is unavailable the
# fixtures still generate, just without a readable timecode burned in.
if "$FFMPEG" -hide_banner -filters 2>/dev/null | grep -q drawtext; then
  MARKER="drawtext=text='%{pts\\:hms}':x=10:y=10:fontsize=48:fontcolor=white:box=1:boxcolor=black@0.6"
else
  echo "warning: no drawtext filter; fixtures will lack per-frame markers" >&2
  MARKER=""
fi

gen() {
  local name="$1" size="$2" rate="$3" seconds="$4" vcodec="$5" extra="${6:-}"
  local out="$OUT/$name"
  # The synthetic source is built per call from this function's arguments.
  # Building it once at script scope would silently reuse the first
  # resolution and rate for every fixture.
  local src="testsrc2=size=${size}:rate=${rate}:duration=$(dur "$seconds")"
  echo "-> $out ($size @ $rate, $(dur "$seconds")s, $vcodec)"
  # shellcheck disable=SC2086
  if ! "$FFMPEG" -hide_banner -loglevel error -y \
      -f lavfi -i "$src" \
      $([ -n "$MARKER" ] && echo "-vf $MARKER") \
      -c:v "$vcodec" $extra \
      -pix_fmt yuv420p \
      -t "$(dur "$seconds")" \
      "$out" 2>/dev/null; then
    echo "   (encoder $vcodec unavailable in this build, falling back to mpeg4)" >&2
    "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "$src" \
      -c:v mpeg4 -q:v 4 -t "$(dur "$seconds")" "$out"
  fi
}

# §16 benchmark scenario 1 wants a 10-minute 1080p, a 10-minute 4K, and a
# 5-minute 1080p60 source. These are the shapes the benchmark exercises.
#
# NOTE on codec names: this sidecar build has NO H.264 or HEVC *encoder*
# (ADR-006 is open; libx264 is GPL and excluded), so the files are named for
# their shape and resolution rather than for a codec they do not contain.
# Passing -c:v libx264 to this sidecar fails with "Unknown encoder", which is
# the correct behaviour: a fixture that silently became a different codec
# would make every benchmark result a lie.
gen "1080p30_10min.mp4"  1920x1080 30 "$(dur 600)" libvpx-vp9 "-b:v 5M -deadline realtime -cpu-used 8"
gen "4k30_10min.mp4"     3840x2160 30 "$(dur 600)" libvpx-vp9 "-b:v 20M -deadline realtime -cpu-used 8"
gen "1080p60_5min.mp4"   1920x1080 60 "$(dur 300)" libvpx-vp9 "-b:v 6M -deadline realtime -cpu-used 8"

# A still-image source for thumbnail and overlay-stream tests.
#
# §7.4 will store thumbnails as WebP, but this build has a WebP *encoder*
# (libwebp_anim) and no WebP *muxer*, so it cannot be written as a standalone
# file here. PNG via the image2 muxer serves the same purpose for the fixture:
# the thumbnail code cares about the decoded pixels and the sprite-strip
# packing, not the container. WebP storage is exercised in Phase 2.
echo "-> $OUT/thumb_source.png"
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=640x360:rate=1:duration=$(dur 10)" \
  -frames:v 1 -c:v png "$OUT/thumb_source.png"

# A 1 kHz tone for audio-only paths (mixer, loudness, peaks).
#
# Mono, not stereo. The synthetic source produces one channel, and converting
# it to stereo needs the aresample filter, which is not in this build (it is
# added in Phase 2 alongside the audio chunk cache). Producing a silent file
# with a failed -ac 2 is worse than producing an honest mono one.
echo "-> $OUT/tone_1khz.wav"
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000:duration=$(dur 10)" \
  "$OUT/tone_1khz.wav"

echo
echo "Fixtures generated in $OUT:"
ls -la "$OUT"
echo
echo "Note: these are intentionally untracked (see .gitignore). Regenerate"
echo "with scripts/gen-fixtures.sh rather than committing media."