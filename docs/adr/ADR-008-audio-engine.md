# ADR-008: Audio engine

- **Status:** Accepted (structure); device layer pending measurement
- **Date:** 2026-10-10
- **References:** §8.1, §22

## Context

Quillie mixes audio in the Rust host, not in the webview. The playhead is
single-threaded: audio must never stutter, and video frames may be dropped
instead (§8.1). That requirement alone decides where the mixer lives — a
webview audio graph cannot be given a real-time guarantee, and the WebKitGTK
audio path varies by distribution and installed GStreamer plugins.

The build document lists three candidates:

1. A Rust mixer on `cpal` with lazily decoded PCM chunks (the default design).
2. Streaming decode.
3. The WebView's own audio.

## Decision

**A Rust mixer on `cpal`, fed by decoded PCM chunks.** The mixer is
implemented and tested; the device layer is deliberately not yet written.

## Why chunks rather than streaming decode

Decoding ahead of time into fixed 30-second chunks (§7.3) means the audio
thread only ever *reads memory*. Streaming decode would put FFmpeg's
demuxer and decoder on the audio thread, where any allocation or lock is a
dropout.

This costs latency: the playhead cannot start until enough of the current chunk
is decoded. That cost is bounded and paid once, whereas streaming decode pays
its risk on every buffer, forever.

The alternative is not rejected outright — it is simply that correctness
first. Streaming decode can become viable if measurement ever shows chunk
decode is the bottleneck, and the mixer's interface does not change when it
does: chunks in, samples out.

## The real-time contract

Everything `render` touches exists before playback starts:

- All track buffers are preallocated in `Mixer::new`.
- The output buffer is supplied by the caller, so `render` cannot allocate.
- The playhead is an `AtomicU64`, not a mutex. The UI reads it every frame and
  the audio thread writes it every buffer; a lock between them is exactly the
  contended mutex the design forbids.
- Chunks are immutable once published and swapped via `Arc`, so the audio
  thread never locks to read samples.
- Gain is stored as a linear multiplier, converted from dB once at set time
  rather than a `log` per sample on the audio thread.

## Clock

The playhead advances by exactly the frames the audio callback produced. Video
follows; it does not free-run. This is the whole meaning of "audio-first" and
it is what makes a dropped video frame a cosmetic problem rather than a sync
one.

`ClockSource` switches to `Monotonic` when paused or silent, because there is
no audio callback to follow then.

## The channel-count trap

`ticks_per_sample_frame` takes a **channel count**. Ticks per *sample* at
48 kHz is 2; ticks per *frame* of stereo is 4.

Getting this wrong runs the playhead at exactly half speed for stereo, which
presents as a sync problem and is not one — it is a unit error, and the two
look identical to a user. The first implementation had it and a test caught it.

## Soft limiting

The sum of N tracks can exceed full scale. Hard clipping produces audible
crackle, so the output passes through a `tanh` knee normalised by
`limit / tanh(limit)`, which maps the soft region back onto the unit range.
An already-quiet signal passes through untouched.

## What is decided by measurement, and has not been taken

ADR-008 in the build document says to choose from "dropout and sync
measurements on the floor machine". Those measurements have not been taken,
because there is no preview pipeline to produce sync against yet.

The mixer structure is independent of that outcome: it is the same mixer
either way. What the measurements will decide is:

- whether `cpal` needs a device-specific workaround on either platform
- whether the 1024-frame default buffer is right, or whether the floor needs
  2048
- whether chunk decode latency is acceptable, or whether streaming is needed

**The floor numbers for all three are "not measured".**

## Consequences

- `cpal` becomes a dependency when the device layer is written. It is MIT OR
  Apache-2.0, recorded in `docs/licenses.md`.
- The audio thread's allocation count is a testable property. There is a
  regression test for the interface, but a real allocation counter belongs in
  the bench harness.
- Audio and video sync is a measurable property of this design, not a hope: the
  playhead *is* the audio frame count.