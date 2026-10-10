# Quillie

A motion-first desktop video editor.

Quillie has one project model and three views over it:

- **Quick Edit** — a guided path for a first video.
- **Timeline** — full nonlinear editing.
- **Motion Studio** — tracking, animated text, subject-follow framing.

Switching views never discards or reinterprets data.

## Design principle: pay as you go

Quillie does no work the user did not ask for and the screen does not need.
Idle cost is zero. Heavy features cost resources only while they run, and they
run on cached, cancelable, low-priority jobs. Weak hardware gets slower previews
and slower background work — never a worse editor.

## Target machine

The **floor** is the weakest machine a user plausibly owns: 4 GB RAM, 2 physical
cores, no AVX2, integrated graphics, a 5400 RPM mechanical disk. Every basic
operation must work there. See [`docs/perf/floor.md`](docs/perf/floor.md).

## Stack

| Layer | Choice |
|---|---|
| Shell | Tauri v2 |
| Backend | Rust (stable), Cargo workspace |
| UI | Svelte + TypeScript (strict), Vite |
| Timeline / preview | PixiJS v8 on WebGL |
| Audio | Rust host via `cpal` (ADR-008) |
| Media | FFmpeg + ffprobe, built slim from pinned source |
| Transcription | whisper.cpp, optional sidecar |

## Repository layout

```
crates/quillie-core/   pure Rust: model, time, media, analysis. No Tauri, no spawning.
crates/quillie-app/    Tauri host: commands, I/O, process supervision, audio.
ui/                    Svelte app
resources/             recipes, looks, kits, fonts, sidecar manifest
scripts/               sidecar build + fixture generation
tests/                 generated fixtures and golden references
bench/                 benchmark harness and committed result files
docs/                  ADRs, perf, format, licenses, phase reports
```

`quillie-core` builds and tests without Tauri and without FFmpeg installed.

## Building

```sh
cargo test -p quillie-core          # model and time base, no sidecars needed
cargo clippy --all-targets -- -D warnings
```

FFmpeg sidecars are built from a pinned, PGP-verified source release:

```sh
scripts/build-sidecar.sh
```

See [`docs/ffmpeg-build.md`](docs/ffmpeg-build.md) for why this is built rather
than downloaded, and [`docs/licenses.md`](docs/licenses.md) for the license
position.

## Status

Phases are tracked in [`docs/reports/`](docs/reports/). Performance claims are
only made with a committed result file in `bench/results/` behind them.