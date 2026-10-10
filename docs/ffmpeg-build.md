# Building the FFmpeg sidecars

Quillie bundles `ffmpeg` and `ffprobe` as sidecars. They are **built from
source**, not downloaded, and not taken from the distribution.

## Why not a distro package

FFmpeg packages on Debian and MX Linux are **GPL builds**: they link `libx264`
and `libx265`. Bundling one would attach GPL obligations to every Quillie
package before anyone has decided the H.264 encoder question (ADR-006). Building
ourselves means the configure line is the complete, auditable list of what gets
linked.

## Why not a prebuilt download

Same reason, plus verifiability. A prebuilt tarball is a binary from an unknown
build configuration. `--disable-autodetect` is only meaningful if we control the
whole configure line — otherwise the build silently picks up whatever libraries
it finds on the build machine, which is precisely how a GPL component gets in
without anyone noticing.

## Verification: two hashes, two different jobs

These are genuinely different things and the build document's framing is right
to keep them separate.

**1. Source tarball — verifies upstream.**
Checked against the FFmpeg release **PGP signature**. This is the real check.
ffmpeg.org publishes **no `.sha256` file** (verified: `ffmpeg-9.0.2.tar.xz.sha256`
returns HTTP 404); it publishes `.asc` signatures signed with the release key
whose fingerprint is listed on the download page. We verify the signature *and*
that it came from that exact fingerprint, because "good signature" from an
untrusted key proves nothing on its own.

```
fingerprint: FCF9 86EA 15E6 E293 A564  4F10 B432 2F04 D676 58D8
tarball:     8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
```

Verified good signature, 2026-10-10.

**2. Binary — does not verify upstream.**
Recorded in `resources/sidecars/manifest.json`. This exists so CI can tell a
clean rebuild from a drifted one. It says nothing about whether the build was
correct, which is why it is not the upstream check.

## Build dependencies without root

The build needs libvpx, libopus, dav1d, libvorbis, libwebp and zlib. Where
these are not installed and no root is available, they are unpacked into a
local prefix:

```sh
mkdir -p /tmp/opencode/debs /tmp/opencode/prefix
cd /tmp/opencode/debs
apt-get download libvpx-dev libopus-dev libdav1d-dev libvorbis-dev \
                  libwebp-dev zlib1g-dev libogg-dev libsharpyuv-dev
# plus the matching *runtime* packages -- the -dev packages ship only
# symlinks to versioned .so files that live in the runtime packages.
for f in *.deb; do dpkg-deb -x "$f" ../prefix; done
```

Two gotchas worth recording, both of which cost time:

1. **The `-dev` packages are not enough.** They provide `libfoo.so` as a
   symlink to `libfoo.so.N`, but the target lives in the runtime package
   (`libvorbis0a`, `libopus0`, `libwebp7`, …). Without those, every link fails
   with `cannot find -lfoo`.

2. **The extracted `.pc` files hardcode `prefix=/usr`**, so pkg-config emits
   `-I/usr/include/...` for headers that are actually in the local prefix. The
   fix is to rewrite the prefix:

   ```sh
   sed -i 's|^prefix=/usr$|prefix=/tmp/opencode/prefix/usr|' *.pc
   ```

Static `.a` archives are also renamed aside so the linker uses shared objects;
the static libopus pulls in `-lm`, which pkg-config does not report under
`--disable-autodetect`.

## Build

```sh
scripts/build-sidecar.sh
```

Takes roughly 15–30 minutes with `make -j2`. Outputs land in
`crates/quillie-app/sidecars/ffmpeg-<triple>` and `ffprobe-<triple>`.

## License audit

The script audits itself and **fails the build** if `-buildconf` shows any of:

```
enable-gpl  enable-nonfree  libx264  libx265  libfdk
```

Current build: **LGPL version 2.1 or later**. Clean.

## What is enabled

Deliberately small. `--disable-everything` then opt in:

- **protocols:** file, pipe
- **demuxers:** mov, matroska, mpegts, avi, mp3, wav, flac, ogg, image2
- **muxers:** mp4, webm, matroska, wav, null, image2, adts, rawvideo
- **decoders:** h264, hevc, av1, vp9, aac, mp3, opus, vorbis, flac, pcm_s16le,
  mjpeg, png, webp
- **encoders:** libvpx_vp9, libopus, aac, libvorbis, libwebp_anim, pcm_s16le,
  png, mjpeg, flac
- **parsers:** h264, hevc, av1, vp9, aac, opus
- **filters:** scale, crop, overlay, format, trim, atrim, setpts, asetpts,
  concat, amix, volume, afade, fps, pad, hflip, vflip, rotate, transpose

Note there is **no H.264 encoder** — see ADR-006. Decoding is unaffected.

`libdav1d` (BSD-2) is linked for faster AV1 decode. `libsvtav1` is absent: it
has no Debian 13 package, so using it would mean a second pinned source build
with its own verification, and it is deferred until the encoder decision.

## Windows

Built under MSYS2 (MINGW64) with the same configure line, output renamed to
`ffmpeg-x86_64-pc-windows-msvc.exe`. A MinGW build runs fine as a Windows
sidecar. CI builds it on a Windows runner so no Windows machine is needed.