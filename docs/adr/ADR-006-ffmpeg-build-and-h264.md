# ADR-006: FFmpeg build and the H.264 encoder

- **Status:** **Open** — deliberately unresolved at the start of Phase 0
- **Date:** 2026-10-10
- **References:** build prompt §7.2, §11.2, §22

## Context

FFmpeg is bundled as a sidecar. Which H.264 **encoder** it contains determines
the license obligations that ship with every Quillie binary, so this is the one
packaging decision that cannot be made casually or reversed late.

The four candidate positions:

1. **`libx264`** — best quality per bitrate, most mature. But `libx264` is GPL,
   so linking it makes the distributed Quillie package GPL-covered.
2. **`OpenH264`** — Cisco-licensed, royalty-free when using their unmodified
   prebuilt binary. Lower quality at equal bitrate, and basic profile coverage
   only.
3. **No H.264 encoder** — export AV1 or VP9 only. Cleanest licensing, but
   software AV1/VP9 encode is slow on a floor machine and some target platforms
   expect H.264.
4. **OS-provided encoder** — Windows Media Foundation, Linux VA-API. The OS or
   chip vendor carries the licensing; Quillie falls back to (3) when absent.

**H.264 *decoding* is a separate question and is not affected.** Every option
keeps full H.264 decode, which is what import and preview actually need. Only
encoding is at stake.

## Current state

**Undecided by explicit instruction.** The project owner was asked to settle
this before Phase 0 ships and chose to defer, noting the constraint only binds
at ship time, not at build time.

What is done now so this is not a blocker later:

- The sidecar build script uses `--disable-autodetect`, so exactly the
  components listed in `scripts/build-sidecar.sh` are linked. Nothing sneaks in
  from the build machine.
- The script **audits its own output** and fails if `-buildconf` shows
  `enable-gpl`, `enable-nonfree`, `libx264`, `libx265` or `libfdk`. As built
  today the build is clean and the license is **LGPL 2.1 or later**.
- **No H.264 encoder is currently enabled.** The encode set is `libvpx_vp9`,
  `libopus`, `aac`, `libvorbis`, `libwebp_anim`, `pcm_s16le`, `png`, `mjpeg`,
  `flac`.
- Phase 1 export must therefore use VP9 or AV1 in MP4, not H.264. This is a
  real, temporary limitation and is called out in the phase report.

Adding `libx264` later is a one-line change to the configure line plus a
rebuild; it does not touch application code. Deciding after Phase 0 is cheap
provided nothing is *shipped* before the decision.

## What this decision still blocks

- Choosing the default export preset codec (§11.2 currently says software H.264
  at veryfast; that line is provisional).
- Publishing any Quillie package to a distribution channel.
- Finalising `docs/licenses.md` as a shipped document.

## Note on `libsvtav1`

The intended configure line enabled `libsvtav1` for AV1 encoding. SVT-AV1 has
no Debian 13 package, so using it means building it from a separately pinned
source release with its own verification step. It is omitted for now to keep
the sidecar build reproducible from the script alone.

## Note on `libdav1d`

`libdav1d` (the dav1d AV1 decoder) is enabled for AV1 *decode* performance and
is BSD-2-Clause, so it carries no licensing cost. It is in the build.

## Consequences for now

- The sidecar is LGPL 2.1+, which is the safest position to build and test
  under while the encoder question is open.
- Export presets and their realtime factors must be re-measured once the
  encoder is chosen; the Phase 1 export measurement is therefore not final.