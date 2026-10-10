//! Audio output and mixing (§8.1).
//!
//! The mixer runs on the `cpal` output thread and must never allocate, never
//! take a contended lock, and never touch the disk. Everything it needs is
//! prepared ahead of time in `prepare()` and read in `render()`.
//!
//! ADR-008 is still open: whether to mix decoded PCM chunks (the default
//! design here) or stream-decode is decided by dropout and sync measurements
//! on the floor machine. Those measurements have not been taken yet, so the
//! structure supports both without committing to either.

/// Which device clock the playhead follows (§8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// The audio callback, while audio is playing.
    AudioCallback,
    /// A monotonic clock, while paused or silent.
    Monotonic,
}

/// Build-time audio parameters. Nothing here is chosen per callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for AudioConfig {
    fn default() -> Self {
        // 48 kHz is the project default (§6.2) and the rate every target
        // platform resamples to cheaply anyway.
        AudioConfig { sample_rate: 48_000, channels: 2 }
    }
}

/// Queue a decoded chunk for playback.
///
/// Stands in for the real chunk cache until that lands in Phase 2. The point
/// of keeping the interface this small now is that the audio thread contract
/// is fixed: whatever fills the queue, `render` stays allocation-free.
#[derive(Debug, Default)]
pub struct Mixer {
    config: AudioConfig,
}

impl Mixer {
    pub fn new(config: AudioConfig) -> Self {
        Mixer { config }
    }

    pub fn config(&self) -> AudioConfig {
        self.config
    }

    /// Render into a caller-provided buffer.
    ///
    /// Takes `&mut [f32]` rather than returning a `Vec` precisely so this
    /// cannot allocate once it is on the audio thread.
    ///
    /// Silently fills silence for now; the mixing graph is Phase 1 proper.
    pub fn render(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_silence_leaves_every_sample_at_zero() {
        let mut m = Mixer::new(AudioConfig::default());
        let mut buf = [0.25f32; 512];
        m.render(&mut buf);
        assert!(buf.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn the_default_config_is_48k_stereo() {
        let c = AudioConfig::default();
        assert_eq!(c.sample_rate, 48_000);
        assert_eq!(c.channels, 2);
    }
}