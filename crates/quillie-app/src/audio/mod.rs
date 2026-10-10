//! Audio output and mixing (§8.1).
//!
//! The mixer is the part that must be right regardless of platform: it runs on
//! the audio thread under a real-time constraint, so it owns its buffers,
//! allocates nothing during `render`, and takes no contested locks.
//!
//! The device layer is separate and optional. `cpal` is a Phase 1 dependency
//! but the mixer is testable without it, which is deliberate — a mixer that can
//! only be tested by making noise is a mixer whose bugs are found by users.

pub mod mixer;

pub use mixer::{ticks_per_sample_frame, AudioConfig, Chunk, ClockSource, Mixer, Track};

/// What the UI needs from the audio subsystem (§8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackState {
    pub playhead_ticks: i64,
    pub playing: bool,
    pub clock: ClockSource,
}

impl Mixer {
    /// A snapshot for the UI thread.
    ///
    /// Reads only atomics, so it is safe to call while the audio thread is
    /// rendering.
    pub fn state(&self) -> PlaybackState {
        PlaybackState {
            playhead_ticks: self.playhead().raw(),
            playing: self.is_playing(),
            clock: self.clock_source(),
        }
    }
}