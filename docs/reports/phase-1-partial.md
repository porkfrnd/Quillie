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
| `crates/quillie-core/` | Pure logic: time base, project model, edit ops, media parsing, export planner, job scheduler, capability model. No Tauri, no process spawning, no I/O |
| `crates/quillie-app/` | Host: atomic save, process supervision, export execution, capability probe, audio scaffolding, Tauri IPC (behind `tauri-host`) |
| `ui/` | Svelte 5 + Vite + strict TypeScript. Shell, three lazily-loaded views, PixiJS timeline |
| `scripts/build-sidecar.sh` | Slim FFmpeg from pinned, PGP-verified source, with a licence audit and a smoke test that both fail the build |
| `scripts/gen-fixtures.sh` | Media fixtures from `lavfi` sources, untracked |
| `scripts/make-ico.mjs` | Multi-size Windows .ico from the app SVG |
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
- **`export/graph.rs`** — the `ExportGraph` IR and FFmpeg filter builder, shared
  by export and Smooth Zones.
- **`jobs/scheduler.rs`** — six-lane priority scheduler, pure policy.
- **`capability.rs`** — tier derivation, feature gating, memory budgets.

### Rust host

- **`io/project_store.rs`** — atomic save, backup, autosave journal.
- **`process/mod.rs`** — sidecar argument building, spawn, stderr ring,
  progress parsing, stall watchdog.
- **`export/mod.rs`** — one-invocation export with mandatory output validation.
- **`capability_probe.rs`** — hardware, FFmpeg, battery and disk probing.

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
cargo test --workspace --features media
  quillie-core lib        61 passed
  quillie-app lib         14 passed
  crash_injection          7 passed
  export                   7 passed
  media_import             9 passed
  model_invariants        13 passed

cd ui && npx vitest run
  2 files, 16 tests passed

cargo clippy --workspace --all-targets --features tauri-host,media -- -D warnings
  clean
cd ui && npx svelte-check --tsconfig ./tsconfig.json --threshold warning
  0 errors, 0 warnings
cd ui && npx eslint .
  0 errors, 0 warnings
```

**84 Rust tests, 16 UI tests, 100 total. All passing.**

### Phase 1 acceptance criteria

| Criterion | Status |
|---|---|
| Round-trip and rate tests pass | **met** — 12 time tests, property round-trip |
| Random ops + undo returns identical state | **met** — 512-case property test, byte-identical |
| Crash-injection at each save step leaves a valid project | **met** — 7 crash points, all loadable and complete |
| Export duration within one frame of the plan | **met** — real encode, real probe, measured delta |
| Audio plays 5 minutes with zero dropouts | **not met** — no mixer yet |
| Clock stays in sync with video | **not met** — no preview yet |
| New user cuts a three-clip edit in under 10 minutes | **not done** — requires a usable editor |

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

Testing the generated FFmpeg filter string against the **real sidecar** rather
than only against expected substrings found a fourth: FFmpeg uses `:` as its
filter option separator, so `trim=start=00:00:01.000000` parses as an option
named `00` and the entire graph fails to build. Every unit test passed; only
running it caught it. Timestamps are now plain decimal seconds.

A serde round-trip test caught that `Animatable<f32>` could not serialise at
all: serde cannot internally tag a newtype variant containing a bare float.

A real-ffprobe integration test found that per-stream `duration` arrives as the
string `"2.000000"` even where the format-level one parses as a number. The
tolerant string-or-number handling was applied to one and not the other.

---

## Performance

### Measured

| Metric | Result | Budget | Verdict |
|---|---|---|---|
| Initial JS shell (gzip) | **24.7 KiB** | 250 KiB | pass |
| Total CSS (gzip) | **2.6 KiB** | 40 KiB | pass |
| FFmpeg sidecar, ffmpeg | **6.84 MB** | — | recorded |
| FFmpeg sidecar, ffprobe | **6.67 MB** | — | recorded |
| Export duration error | **< 1 frame** | ≤ 1 frame | pass |
| FFmpeg sidecar licence audit | **clean** | no GPL/nonfree | pass |
| Sidecar smoke test (encode + probe) | **pass** | — | pass |
| H.264 decode via sidecar | **pass** | required for import | pass |

The shell number improved from 164.0 KiB to 24.7 KiB during this phase. Vite
was emitting a `modulepreload` link for PixiJS, so the browser fetched 139 KiB
of it at startup despite the dynamic import. That is precisely the failure
code-splitting exists to prevent, and it hid from a naive size check because the
chunk was correctly labelled lazy while still being fetched eagerly.

### Not measured

Everything in §16 that requires the full running application.

- **Cold start, window shell, idle CPU** — the harness exists and runs, but the
  Tauri binary has not been built on this machine, so there is no number. The
  first harness run reported 121 ms cold start and 5% idle CPU; both were
  measuring `rustc` compiling rather than Quillie, because it ran `cargo run`
  and timed from process spawn. That result was discarded, and the harness now
  builds first and launches the binary directly.
- **Scrub latency p95, dropped frames, audio dropouts** — no preview pipeline
  yet.
- **Peak memory of the whole process tree under a real 10-minute project** — the
  harness's PSS accounting works but has never been pointed at a running app.
- **Proxy generation, analysis, export throughput as a realtime factor** — the
  scheduler exists but nothing is scheduled yet. The export tests use a 1-second
  clip and a 320x180 preset precisely so they run fast; that says nothing about
  a 10-minute 1080p export.

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

- **No preview rendering.** Path A (`<video>` element) is not implemented. The
  PixiJS layer exists and is wired to the model through ops and patches, but
  draws no video. This blocks the scrub-latency and dropped-frame criteria.
- **No audio mixer.** `audio::Mixer` renders silence. No `cpal` dependency yet,
  so ADR-008 is untouched. This blocks the dropout and clock-sync criteria.
- **No preview cache, prefetch, GOP cache or frame cache** (§8.4). The dirty
  flag layer system exists; the caches do not.
- **No import command.** Parsing and fingerprinting work against real ffprobe,
  but nothing wires a file path to a `MediaRef` and an `EditOp`.
- **No markers, snapping to analysis data, or in/out marks** in the UI. The
  model has the fields; nothing populates them.
- **No scheduler executor.** The policy module is complete and tested;
  `quillie-app` does not yet spawn jobs it selects.
- **No calibration.** Capability probing works; the 3-second micro-benchmark
  that turns capabilities into cost-model coefficients (§13) does not exist.
- **No commands beyond open/save.**

### The current export situation

There is **no H.264 encoder in the sidecar**. libx264 is GPL and excluded while
ADR-006 is open; OpenH264 is a separate Cisco-licensed binary. VP9 is available
via libvpx, and libsvtav1 for AV1 is absent (no Debian 13 package).

Export works end to end and is validated — but it produces **VP9 in MP4**, not
H.264. That is a real change in output compatibility and it is the visible cost
of leaving ADR-006 open. It is not a blocker for reading media: **H.264 decoding
is verified working** through the sidecar, which is what import and preview
depend on.

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

### Risk: the sidecar is a moving target

`scripts/build-sidecar.sh` produced three binaries that linked, passed the
licence audit, and still could not execute a single command. Each is now
guarded by a smoke test, but the underlying cause — a configure line written
without running anything — is still a live risk as the codec list grows.

---

## Next

1. **Wire import**: file path → ffprobe → `MediaRef` → fingerprint → an
   `EditOp`. Everything it needs exists and is tested against real media; only
   the command is missing.
2. **Implement preview path A** (`<video>` element) and the audio-first clock.
   This unblocks the scrub-latency, dropped-frame and clock-sync criteria, which
   are three of the five remaining acceptance items.
3. **Add the `cpal` mixer** and settle ADR-008 from floor measurements.
4. **Add the scheduler executor** so the tested policy actually runs jobs.
5. **Settle ADR-006** before any package is published. It does not block
   reading media — H.264 decode is verified — but it does determine what
   Quillie can export.