#!/usr/bin/env bash
# Build a slim FFmpeg/ffprobe for Quillie from a pinned, PGP-verified source release.
#
# Why from source and not a distro package or a prebuilt download:
#   - Distro FFmpeg packages on Debian/MX Linux are GPL builds (they include
#     libx264/libx265). Bundling one would drag GPL obligations onto every
#     Quillie package without anyone deciding that (see ADR-006).
#   - A random prebuilt tarball is unauditable; --disable-autodetect below is
#     only meaningful if we control the whole configure line.
#
# Verification is two-stage (see docs/ffmpeg-build.md):
#   1. Source tarball: PGP signature against the FFmpeg release key published
#      on ffmpeg.org/download.html. This is the real upstream check -- ffmpeg.org
#      publishes no .sha256 file, only .asc signatures.
#   2. Binary: SHA256 recorded in resources/sidecars/manifest.json. This does
#      NOT verify upstream; it lets CI detect when a rebuild drifts.
#
# Usage: scripts/build-sidecar.sh [--no-verify]
set -euo pipefail

FFMPEG_VERSION="9.0.2"
FFMPEG_TARBALL_SHA256="8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
FFMPEG_PGP_FPR="FCF986EA15E6E293A5644F10B4322F04D67658D8"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${QUILLIE_SIDECAR_WORK:-/tmp/opencode}"
PREFIX="$WORK/prefix"
SRC="$WORK/ffmpeg-$FFMPEG_VERSION"
OUT="$WORK/ffbuild"
TARGET_TRIPLE="${QUILLIE_TARGET_TRIPLE:-x86_64-unknown-linux-gnu}"
# Parallel compile jobs. FFmpeg's largest translation units (libavfilter,
# libavcodec) are memory-hungry; on a 4 GB machine with a desktop running, -j2
# is already near the limit and -j1 is safer if the build is being OOM-killed.
JOBS="${QUILLIE_JOBS:-2}"

export PKG_CONFIG_PATH="$PREFIX/usr/lib/x86_64-linux-gnu/pkgconfig"
export PKG_CONFIG_LIBDIR="$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="$PREFIX/usr/lib/x86_64-linux-gnu:$PREFIX/lib/x86_64-linux-gnu"

log() { printf '\n=== %s ===\n' "$1"; }

# --- 1. fetch + verify the source tarball -------------------------------
log "1/4 source tarball verification"
TARBALL="$WORK/ffmpeg-$FFMPEG_VERSION.tar.xz"
if [ ! -f "$TARBALL" ]; then
  curl -fsSL -o "$TARBALL" "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz"
  curl -fsSL -o "$TARBALL.asc" "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz.asc"
fi

if [ "${1:-}" != "--no-verify" ]; then
  echo "--> SHA256"
  echo "$FFMPEG_TARBALL_SHA256  $TARBALL" | sha256sum -c -
  echo "--> PGP signature"
  export GNUPGHOME="$WORK/gnupg"
  mkdir -p "$GNUPGHOME"; chmod 700 "$GNUPGHOME"
  [ -f "$WORK/ffmpeg-devel.asc" ] || curl -fsSL -o "$WORK/ffmpeg-devel.asc" https://ffmpeg.org/ffmpeg-devel.asc
  gpg --batch --import "$WORK/ffmpeg-devel.asc" 2>/dev/null || true
  gpg --verify "$TARBALL.asc" "$TARBALL" 2>&1 | grep -q "Good signature" \
    || { echo "PGP VERIFICATION FAILED" >&2; exit 1; }
  gpg --verify "$TARBALL.asc" "$TARBALL" 2>&1 | grep -q "$FFMPEG_PGP_FPR" \
    || { echo "PGP KEY FINGERPRINT MISMATCH" >&2; exit 1; }
  echo "PGP signature GOOD, key $FFMPEG_PGP_FPR"
fi

[ -d "$SRC" ] || tar -C "$WORK" -xf "$TARBALL"

# --- 2. configure -------------------------------------------------------
# --disable-autodetect is the guard that stops the build from picking up any
# library it happens to find on the build machine. Everything we link is
# listed explicitly below.
#
# NOTE ON CODECS: there is deliberately no H.264 *encoder* here. libx264 is
# GPL and OpenH264 is a separate Cisco-licensed binary; ADR-006 is still open,
# so we ship VP9 (libvpx) + AAC and let export use those until ADR-006 settles.
# H.264 *decoding* is fully present -- that is what matters for import and
# preview. SVT-AV1 is absent for the same reason (no distribution package).
log "2/4 configure"
cd "$SRC"
make distclean >/dev/null 2>&1 || true

./configure \
  --prefix="$OUT" \
  --disable-autodetect \
  --disable-everything \
  --disable-doc \
  --disable-network \
  --enable-small \
  --enable-ffmpeg \
  --enable-ffprobe \
  --enable-protocol=file,pipe \
  --enable-demuxer=mov,matroska,mpegts,avi,mp3,wav,flac,ogg,image2 \
  --enable-muxer=mp4,webm,matroska,wav,null,image2,adts,rawvideo \
  --enable-decoder=h264,hevc,av1,vp9,aac,mp3,opus,vorbis,flac,pcm_s16le,mjpeg,png,webp \
  --enable-encoder=libvpx_vp9,libopus,aac,libvorbis,libwebp_anim,pcm_s16le,png,mjpeg,flac \
  --enable-parser=h264,hevc,av1,vp9,aac,opus \
  --enable-filter=scale,crop,overlay,format,trim,atrim,setpts,asetpts,concat,amix,volume,afade,fps,pad,hflip,vflip,rotate,transpose \
  --enable-libvpx --enable-libopus --enable-libdav1d --enable-libvorbis --enable-libwebp --enable-zlib \
  --extra-cflags="-I$PREFIX/usr/include" \
  --extra-ldflags="-L$PREFIX/usr/lib/x86_64-linux-gnu -L$PREFIX/usr/lib -lwebp -lvpx -lopus -lvorbis -lvorbisenc -ldav1d -lz -logg -lsharpyuv -lm" \
  2>&1 | tail -25

# --- 3. build -----------------------------------------------------------
log "3/4 make -j${JOBS}"
# Keep the full log; the summary at the end is what you read, but a killed
# build leaves no error otherwise.
make -j"${JOBS}" > "$WORK/make.log" 2>&1 || {
  echo "make failed; last 40 lines:" >&2
  tail -40 "$WORK/make.log" >&2
  exit 1
}
make install 2>&1 | tail -3
strip "$OUT/bin/ffmpeg" "$OUT/bin/ffprobe"

# --- 4. audit + record --------------------------------------------------
log "4/4 license audit"
# This MUST print nothing. A match means a GPL/nonfree component leaked in.
if "$OUT/bin/ffmpeg" -buildconf 2>&1 | grep -E "enable-gpl|enable-nonfree|libx264|libx265|libfdk"; then
  echo "AUDIT FAILED: GPL/nonfree component present" >&2
  exit 1
fi
echo "CLEAN: no GPL or nonfree components linked"

DEST="$REPO_ROOT/crates/quillie-app/sidecars"
mkdir -p "$DEST"
cp "$OUT/bin/ffmpeg" "$DEST/ffmpeg-$TARGET_TRIPLE"
cp "$OUT/bin/ffprobe" "$DEST/ffprobe-$TARGET_TRIPLE"

log "result"
"$DEST/ffmpeg-$TARGET_TRIPLE" -version 2>&1 | head -2
( cd "$DEST" && sha256sum "ffmpeg-$TARGET_TRIPLE" "ffprobe-$TARGET_TRIPLE" )
echo
echo "Record these in resources/sidecars/manifest.json"