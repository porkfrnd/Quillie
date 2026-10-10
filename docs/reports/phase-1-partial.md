# Phase 1 (partial) report

Covers the first eight tasks of the build plan: the Phase 0 scaffold and the
first slice of the editor core.

**Status: incomplete.** Phase 1 acceptance is not met. The gaps are listed
below rather than left to be discovered.

---

## What changed

### Workspace and host

| File | Purpose |
|---|---|
| `Cargo.toml` | Workspace, release profile (`lto=fat`, `codegen-units=1`, `panic=abort`, `strip`) |
| `crates/quillie-core/` | Pure logic: time base, project model, edit ops, media parsing. No Tauri, no process spawning, no I/O |
| `crates/quillie-app/` | Host: atomic save, process supervision, audio scaffolding, Tauri IPC (behind `tauri-host`) |
| `ui/` | Svelte 5 + Vite + strict TypeScript. Shell, three lazily-loaded views, PixiJS timeline |
| `scripts/build-sidecar.sh` | Slim FFmpeg from pinned, PGP-verified source, with a licence audit that fails the build |
| `scripts/gen-fixtures.sh` | Media fixtures from `lavfi` sources, untracked |
| `bench/coldstart.ts` | Cold-start and idle harness |

### Rust core

- **`time.rs`** — `Ticks(i64)` at 1/120000 s, `FrameRate` as an exact rational.
  12 tests covering every required rate.
- **`project/model.rs`** — the v1 schema: project, sequences, tracks, clips,
  media refs, captions, markers, snapshots, export presets.
- **`project/edit.rs`** — `EditOp` (insert, delete, move, trim, split, ripple
  delete, ripple move), total `Inverse`, and `Patch` derived as a pure function
  of `(op, inverse)`.
- **`project/schema.rs`** — versioned forward migrations. Refuses a project from
  a newer build rather than guessing.
- **`media/probe.rs`** — ffprobe JSON parsing. Nothing is guessed.
- **`media/fingerprint.rs`** — first/last-MiB window hash.

### UI

- Shell with a skeleton that paints before anything else loads.
- Quick Edit, Timeline and Motion Studio as separate lazy chunks.
- PixiJS loaded only when a timeline opens.
- Three retained layers with dirty flags; the render gate stops
  `requestAnimationFrame` entirely when idle.
- Typed command registry; typed IPC wrapper.

---

## Test results

Commands and counts, as run:

```
cargo test --workspace
  quillie-core lib        27 passed
  quillie-app lib          8 passed
  crash_injection          7 passed
  model_invariants        13 passed
  (property test: 512 cases)

cd ui && npx vitest run
  2 files, 16 tests passed

cargo clippy --workspace --all-targets --features tauri-host -- -D warnings
  clean
cd ui && npx svelte-check --tsconfig ./tsconfig.json --threshold warning
  0 errors, 0 warnings
cd ui && npx eslint .
  0 errors, 0 warnings
```

**55 Rust tests, 16 UI tests, 71 total. All passing.**

### Defects the tests found

The property test (random ops, then undo, must restore byte-identical state)
found three real bugs, each of which only appeared for certain edit orderings:

1. **Trim undo** computed its shift against the already-updated `source_in`, so
   undoing a head trim left the clip at the wrong timeline position.
2. **Split undo** searched for its halves by contiguity, which breaks the moment
   a later ripple move displaces them. Now stores the halves it created.
3. **Split** gave the left half a new id, losing link groups and the UI
   selection's reference to the clip. The left half now keeps the original id,
   because it *is* the clip the user was looking at, just shortened.

Separately, a serde round-trip test caught that `Animatable<f32>` could not
serialise at all: serde cannot internally tag a newtype variant containing a
bare float.

---

## Performance

### Measured

| Metric | Result | Budget | Verdict |
|---|---|---|---|
| Initial JS shell (gzip) | **24.7 KiB** | 250 KiB | pass |
| Total CSS (gzip) | **2.6 KiB** | 40 KiB | pass |
| FFmpeg sidecar, ffmpeg | **6.8 MB** | — | recorded |
| FFmpeg sidecar, ffprobe | **6.6 MB** | — | recorded |

The shell number improved from 164.0 KiB to 24.7 KiB during this phase. Vite
was emitting a `modulepreload` link for PixiJS, so the browser fetched 139 KiB
of it at startup despite the dynamic import. That is precisely the failure
code-splitting exists to prevent, and it hid from a naive size check because the
chunk was correctly labelled lazy while still being fetched eagerly.

### Not measured

Everything in §16 that requires a running application.

- **Cold start, window shell, idle CPU** — the harness exists and runs, but the
  Tauri binary has not been built on this machine, so there is no number. The
  first harness run reported 121 ms cold start and 5% idle CPU; both were
  measuring `rustc` compiling rather than Quillie, because it ran `cargo run`
  and timed from process spawn. That result was discarded, and the harness now
  builds first and launches the binary directly.
- **Scrub latency p95, dropped frames, audio dropouts** — no preview pipeline
  yet.
- **Peak memory** — measured as 425.8 MiB, but against the compiler, so it is
  not a number about Quillie.
- **Proxy generation, analysis, export throughput** — none of these features
  exist yet.

No result file has been committed to `bench/results/`, because no valid
measurement has been taken. An empty results directory is the honest state.

---

## The floor machine gap

The development machine is **mid tier, not floor**:

| Property | Dev machine | Floor |
|---|---|---|
| CPU | i5-6200U, 2 physical / 4 logical | 2 physical cores — **matches** |
| RAM | 7.8 GB | 4 GB — **does not match** |
| AVX2 | present | absent — **does not match** |
| Disk | SSD | 5400 RPM mechanical — **does not match** |

It matches the floor on core count, which is closer than the earlier reading of
it. It does not match on RAM, SIMD or disk, and those are the three that most
affect the numbers the floor exists to bound: memory ceilings, CPU-heavy
concurrency, and everything disk-bound.

§2 requires this gap to be recorded rather than substituted, so every
floor-specific acceptance criterion in this phase is marked **not measured**.

### Recommendation

Constrain this machine rather than hunting for another one:

- cgroup memory cap of 4 GB — faithful, easy
- `cpulimit` to 2 cores — already matches
- disk throttling to approximate a 5400 RPM spindle — **not faithful**; disk
  numbers taken this way will be optimistic and must be labelled as such

The memory cap alone would make most of the §16 targets meaningfully
measurable. The disk numbers cannot be made honest by emulation.

---

## Decisions

| ADR | Status | Note |
|---|---|---|
| [ADR-001](../adr/ADR-001-time-base.md) | Accepted | 1/120000 s, verified against all nine rates. Records an inverted-ratio error that briefly looked like a defect in the document |
| [ADR-006](../adr/ADR-006-ffmpeg-build-and-h264.md) | **Open** | H.264 encoder undecided by instruction. Sidecar currently LGPL 2.1+, no H.264 encoder |

Other decisions recorded inline rather than as ADRs: JSON project format
(default from §22, ADR-007 left at its default), adjacent serde tagging for
`Animatable`.

---

## Known gaps and risks

### Not built, and therefore not shipped

Placeholder UI was avoided per §24. What exists is a working shell; what does
not exist is not drawn.

- **No export path at all.** Phase 1's single-track re-encode export is not
  started, so "export duration within one frame of the plan" is untested.
- **No preview rendering.** Path A (`<video>` element) is not implemented. The
  PixiJS layer exists and is wired to the model through ops and patches, but
  draws no video.
- **No audio mixer.** `audio::Mixer` renders silence. No `cpal` dependency yet,
  so ADR-008 is untouched.
- **No scheduler.** §9 is not started; `quillie-core::jobs` does not exist.
- **No capability probe or calibration.** §13 not started.
- **No imports, no media panel, no markers, no snapping to markers.** The model
  has the fields; nothing populates them from a real file.
- **No commands module beyond open/save.**

### The current export situation

There is **no H.264 encoder in the sidecar**. libx264 is GPL and excluded while
ADR-006 is open; OpenH264 is a separate Cisco-licensed binary. VP9 is available
via libvpx, and libsvtav1 for AV1 is absent (no Debian 13 package).

This means any Phase 1 export will be VP9-in-MP4 or AV1, not H.264. That is a
real change in output compatibility, and it is the visible cost of leaving
ADR-006 open. **This is the single most consequential thing to settle before
Phase 1 continues**, because export presets, realtime-factor measurements and
the default codec all depend on it.

### Risk: the build script has produced three broken binaries

`scripts/build-sidecar.sh` linked and passed its licence audit three times
before producing a binary that could execute a single command: missing
`lavfi`, `libvpx_vp9` named wrong, and `wrapped_avframe` enabled as an encoder
but not a decoder.

All three were found only by *running* the binary. A licence audit is not a
functional test, and neither is a successful link. The build should gain a
smoke test — encode a 1-second clip and probe it — so a future missing component
fails the build instead of the user.

### Risk: property test coverage is narrower than it looks

The 512-case property test exercises eight op kinds across two tracks, but the
op set is enumerated from a fixed candidate list per state. It does not cover
ordering permutations of edits on the same clip, and it does not cover the
undo-of-undo (redo) path, which is not implemented.

### UI has no integration tests

`svelte-check` and ESLint are clean, and the pure functions (time, viewport,
layers) are unit tested. The components themselves are untested, and nothing
exercises the PixiJS lifecycle. On a headless machine a WebGL context will not
exist, so this needs a degraded-mode test (§14.4) before the timeline can be
trusted.

---

## Next

1. Settle ADR-006. It blocks export, and export is most of what remains in
   Phase 1.
2. Add the smoke test to the sidecar build script.
3. Implement ffprobe import end to end, using the built sidecar against real
   generated fixtures.
4. Implement preview path A and the re-encode export, then measure export
   duration against the plan.
5. Implement the audio mixer and the audio-first clock (ADR-008), which needs
   floor measurements that cannot be taken until there is something to measure.