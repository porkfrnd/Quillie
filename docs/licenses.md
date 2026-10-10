# Licences

Every dependency and sidecar Quillie ships, with its licence and how it is
distributed. A new dependency gets an entry here **before** it is added, and the
licence must be compatible with how Quillie is distributed.

## The position that matters

**FFmpeg is built from source with `--disable-autodetect`.** The build enables
only the components listed in [`ffmpeg-build.md`](ffmpeg-build.md) and the
script fails if `-buildconf` shows any GPL or nonfree component. As built
today the sidecar is **LGPL 2.1 or later**.

That is a *deliberate intermediate position*, not a final answer.
[ADR-006](adr/ADR-006-ffmpeg-build-and-h264.md) is still open: the H.264
**encoder** decision is what determines the final distribution obligations, and
nothing may ship before it is settled. What is settled is that nothing GPL is
currently linked, so there is no accidental exposure.

## Quillie

`MIT OR Apache-2.0` — see `Cargo.toml`. Not yet a published release; the
identifier is in place so the tooling is correct from the first tag.

## Rust dependencies

These are the `normal` (not `dev`, not `build`) edges of both crates.

| Crate | Version | Licence | Notes |
|---|---|---|---|
| serde | 1.x | MIT OR Apache-2.0 | |
| serde_json | 1.x | MIT OR Apache-2.0 | |
| thiserror | 2.x | MIT OR Apache-2.0 | |
| uuid | 1.x | MIT OR Apache-2.0 | v4 ids only, no `getrandom` backend beyond the OS |
| tempfile | 3.x | MIT OR Apache-2.0 | atomic save |
| cpal | 0.15 | MIT OR Apache-2.0 | **planned**, Phase 1 audio |

Transitive: `itoa`, `memchr`, `zmij`, `serde_core`, `proc-macro2`, `quote`,
`syn`, `unicode-ident`, `equivalent`, `hashbrown` — all MIT OR Apache-2.0.

Development-only (not shipped): `proptest` (MIT OR Apache-2.0), Rust test and
benchmark infrastructure.

All Rust dependencies are dual MIT/Apache-2.0, which is compatible with every
distribution model and places no obligation on Quillie's own licence.

## UI dependencies

| Package | Version | Licence | Notes |
|---|---|---|---|
| pixi.js | 8.x | MIT | timeline and preview rendering |
| svelte | 5.x | MIT | |
| vite | 6.x | MIT | build only, not shipped |
| typescript | 5.x | Apache-2.0 | build only |
| @sveltejs/vite-plugin-svelte | 5.x | MIT | build only |
| svelte-check | 4.x | MIT | build only |
| vitest | 2.x | MIT | build only |
| eslint, typescript-eslint | 9.x / 8.x | MIT | build only |
| prettier, prettier-plugin-svelte | 3.x | MIT / MIT | build only |
| @tauri-apps/api | 2.x | MIT OR Apache-2.0 | IPC |
| @types/node | 22.x | MIT | build only |

Only `pixi.js`, `svelte`, `@tauri-apps/api` and their runtime closure reach the
shipped bundle; everything else is build-time.

### PixiJS

MIT, and the package ships a Canvas2D fallback path. §14.4 requires the
timeline to degrade to Canvas2D when WebGL is unavailable or software-rendered
(llvmpipe), which PixiJS supports — so the degraded mode does not mean dropping
the renderer.

## Sidecars

| Sidecar | Version | Licence | Notes |
|---|---|---|---|
| ffmpeg | 9.0.2 (built from source) | **LGPL 2.1 or later** | see below |
| ffprobe | 9.0.2 (built from source) | **LGPL 2.1 or later** | see below |
| whisper.cpp | *optional, Phase 3* | MIT | not bundled; downloaded on explicit user action |

### FFmpeg's components

The FFmpeg build links these external libraries:

| Library | Licence | Purpose |
|---|---|---|
| libvpx | BSD-3-Clause | VP9 decode/encode |
| libopus | BSD-3-Clause | Opus decode/encode |
| dav1d | BSD-2-Clause | AV1 decode |
| libvorbis | BSD-3-Clause | Vorbis decode/encode |
| libwebp | BSD-3-Clause | WebP decode/encode |
| zlib | Zlib | PNG, compressed streams |
| libogg | BSD-3-Clause | Ogg container (vorbis dependency) |
| libsharpyuv | BSD-2-Clause | libwebp dependency |

All permissive. None is copyleft, none is a patent-encumbered encoder.

### What is NOT linked

- **libx264 / libx265** — GPL. Excluded. This is the subject of ADR-006.
- **libfdk-aac** — nonfree. Excluded permanently.
- **libsvtav1** — BSD-3, but has no distribution package and would require a
  second pinned source build. Deferred; not a licensing blocker.

### Consequence for distribution

LGPL-2.1-or-later FFmpeg can be redistributed in any Quillie package provided
that:

1. Quillie's own licence is compatible (MIT/Apache-2.0 is).
2. Users can relink FFmpeg against a modified version — satisfied in practice
   because Quillie ships the sidecar as a separate executable and does not link
   it into the Quillie binary at all. This is the cleanest possible
   arrangement: the LGPL's relinking requirement is met trivially.
3. The FFmpeg licence and source offer accompany the package.

Point 2 is why building FFmpeg ourselves and shipping it as a sidecar is
preferable to linking it statically: the LGPL's most awkward obligation simply
does not arise.

### GPL

If ADR-006 later selects libx264, the distributed Quillie package becomes
GPL-covered and Quillie itself must be GPL-compatible. That is a
once-irreversible consequence of shipping, which is why the decision is
recorded as open and why no package is published before it is made.

## Transitive font and asset licences

`resources/fonts/` ships one variable UI font plus a small set of caption
fonts. **Not yet chosen.** The chosen face and its licence get an entry here
before it is committed. Candidates must be OFL or equivalent, and the licence
file ships with the package.

## Icons

One open-source icon set, inlined as an SVG sprite (§17, no emoji icons).
**Not yet chosen**; same rule — licence recorded before the asset is committed.

## How to check this file

```sh
cargo tree -e normal          # Rust, shipped edges
npm ls --omit=dev             # UI, shipped edges
scripts/build-sidecar.sh      # prints the FFmpeg -buildconf audit
```

A dependency added without a row here is a bug in this file, not just in the
build.