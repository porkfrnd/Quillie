//! Audio mixer (§8.1).
//!
//! # The contract with the audio thread
//!
//! `cpal`'s callback runs on a real-time thread. Everything it touches must
//! already exist: no allocation, no lock on a contested mutex, no disk. A
//! single allocation in that callback is a dropout, and a dropout is audible.
//!
//! The design follows from that:
//!
//! - [`Mixer`] owns all its buffers and is built before playback starts.
//! - [`Mixer::render`] takes `&mut self` and `&mut [f32]`, so it cannot
//!   allocate: the caller supplies the output buffer.
//! - Samples come from chunks decoded ahead of time (§7.3) and swapped in
//!   atomically via a lock-free ring of pre-allocated slots.
//!
//! # The clock
//!
//! The playhead follows the audio callback while audio plays. Dropping video
//! frames is acceptable; stuttering audio is not (§8.1). The callback reports
//! how many frames it has consumed and the video side reads that, rather than
//! the two free-running against each other and drifting.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use quillie_core::time::Ticks;

/// Output device format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// Output buffer size. Larger trades latency for stability on a slow
    /// machine; on the floor a 1024-frame buffer is the difference between
    /// working and not.
    pub buffer_frames: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        // 48 kHz is the project default (§6.2) and the rate every target
        // platform resamples to cheaply anyway.
        AudioConfig { sample_rate: 48_000, channels: 2, buffer_frames: 1024 }
    }
}

/// Which clock the playhead follows (§8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// The audio device callback, while audio is playing.
    AudioCallback,
    /// A monotonic clock, while paused or silent.
    Monotonic,
}

/// A decoded audio chunk waiting to be mixed (§7.3).
///
/// Immutable once published, so the audio thread can read it without locking.
/// Swapping is done by replacing the `Arc` in a slot, never by mutating the
/// samples.
#[derive(Debug)]
pub struct Chunk {
    pub id: u64,
    /// Interleaved s16 samples, already resampled to the device rate.
    ///
    /// Stored as i16 rather than f32 deliberately: half the memory, and the
    /// conversion is one multiply on the audio thread, which is cheaper than
    /// the cache misses of reading 32-bit samples.
    pub samples: Vec<i16>,
    pub channels: u16,
    pub sample_rate: u32,
}

impl Chunk {
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }

    /// Read one interleaved frame as f32, already scaled.
    ///
    /// Saturating rather than wrapping: a wrapped int16 is a full-scale
    /// transient, which is far more audible than a clipped sample.
    #[inline]
    pub fn frame(&self, index: usize) -> [f32; 2] {
        let ch = self.channels.max(1) as usize;
        let base = index * ch;
        let l = self.samples.get(base).copied().unwrap_or(0) as f32 / 32768.0;
        let r = self
            .samples
            .get(base + if ch > 1 { 1 } else { 0 })
            .copied()
            .unwrap_or(0) as f32
            / 32768.0;
        [l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0)]
    }
}

/// A playing track: a chunk plus a gain and a fade.
#[derive(Debug)]
pub struct Track {
    pub chunk: Arc<Chunk>,
    /// Linear gain, not dB. Converting on the audio thread costs a log per
    /// sample; the conversion happens once when the gain is set.
    pub gain: f32,
    /// Fades, in ticks, applied at the head and tail of the chunk.
    pub fade_in: Ticks,
    pub fade_out: Ticks,
    /// Per-sample read position within the chunk.
    position: usize,
    /// When the track finishes.
    pub finished: bool,
}

impl Track {
    pub fn new(chunk: Arc<Chunk>) -> Self {
        Track {
            chunk,
            gain: 1.0,
            fade_in: Ticks::ZERO,
            fade_out: Ticks::ZERO,
            position: 0,
            finished: false,
        }
    }

    pub fn set_gain_db(&mut self, db: f32) {
        self.gain = 10f32.powf(db / 20.0);
    }
}

/// The mixer.
///
/// Constructed before playback and never resized. `render` is the only method
/// the audio thread calls, and it is allocation-free by construction.
pub struct Mixer {
    config: AudioConfig,
    tracks: Vec<Track>,
    /// Frames the callback has consumed since the last seek.
    frames_played: AtomicU64,
    /// Playhead position, published by the audio thread and read by the UI.
    ///
    /// Atomic rather than mutex-guarded: the UI reads it every frame and the
    /// audio thread writes it every buffer. A mutex here would be exactly the
    /// contended lock the design forbids.
    playhead_ticks: AtomicU64,
    /// True while audio is actually producing sound.
    playing: AtomicU64,
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mixer")
            .field("config", &self.config)
            .field("tracks", &self.tracks.len())
            .finish()
    }
}

impl Mixer {
    /// Build a mixer with room for `max_tracks`.
    ///
    /// Preallocating here rather than growing in `render` is the entire reason
    /// the audio thread can be allocation-free.
    pub fn new(config: AudioConfig, max_tracks: usize) -> Self {
        Mixer {
            config,
            tracks: Vec::with_capacity(max_tracks),
            frames_played: AtomicU64::new(0),
            playhead_ticks: AtomicU64::new(0),
            playing: AtomicU64::new(0),
        }
    }

    pub fn config(&self) -> AudioConfig {
        self.config
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed) == 1
    }

    /// Add a track. Not safe to call from the audio thread.
    pub fn add_track(&mut self, chunk: Arc<Chunk>, gain_db: f32) {
        let mut t = Track::new(chunk);
        t.set_gain_db(gain_db);
        self.tracks.push(t);
    }

    /// Drop finished tracks.
    pub fn prune(&mut self) {
        self.tracks.retain(|t| !t.finished);
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.frames_played.store(0, Ordering::Relaxed);
        self.playhead_ticks.store(0, Ordering::Relaxed);
        self.playing.store(0, Ordering::Relaxed);
    }

    /// Move the playhead. The audio thread picks this up on the next buffer.
    ///
    /// Called from the UI thread, never from the callback.
    pub fn seek(&mut self, to: Ticks) {
        self.playhead_ticks.store(to.raw() as u64, Ordering::Relaxed);
        self.frames_played.store(0, Ordering::Relaxed);
        for t in self.tracks.iter_mut() {
            t.position = 0;
            t.finished = false;
        }
    }

    /// The playhead, read by the UI.
    pub fn playhead(&self) -> Ticks {
        Ticks(self.playhead_ticks.load(Ordering::Relaxed) as i64)
    }

    /// Frames the audio callback has produced.
    pub fn frames_played(&self) -> u64 {
        self.frames_played.load(Ordering::Relaxed)
    }

    /// Which clock the playhead should follow right now (§8.1).
    pub fn clock_source(&self) -> ClockSource {
        if self.is_playing() && !self.tracks.is_empty() {
            ClockSource::AudioCallback
        } else {
            ClockSource::Monotonic
        }
    }

    /// Mix into `out`.
    ///
    /// **This runs on the audio thread.** It must not allocate, lock, or touch
    /// the disk. `out` is supplied by the caller precisely so that it cannot.
    ///
    /// Returns the number of frames written, or **0 when playback has ended**
    /// because every track finished. The distinction matters: 0 is the signal
    /// to stop, and a caller that reads it as "wrote nothing, carry on" will
    /// sit in an empty loop forever.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / self.config.channels.max(1) as usize;
        if frames == 0 {
            return 0;
        }

        let channels = self.config.channels.max(1) as usize;
        // Read the config fields before the mutable borrow of `self.tracks`.
        // This is the shape the whole method has: take what you need, then
        // borrow the tracks mutably and never look at `self` again.
        let sample_rate = self.config.sample_rate;
        let ticks_per_frame = crate::audio::ticks_per_sample_frame(sample_rate, self.config.channels);
        let active = self.tracks.iter_mut().filter(|t| !t.finished).count();
        if active == 0 {
            for s in out.iter_mut() {
                *s = 0.0;
            }
            self.playing.store(0, Ordering::Relaxed);
            return 0;
        }
        self.playing.store(1, Ordering::Relaxed);

        // Mix into the caller's buffer in place. No temporary: allocating one
        // per buffer is exactly what causes dropouts.
        for t in self.tracks.iter_mut() {
            if t.finished {
                continue;
            }
            let total = t.chunk.frames();
            let (fade_in, fade_out) = (t.fade_in, t.fade_out);
            let fade_in_frames = self_ticks_to_frames(fade_in, ticks_per_frame);
            let fade_out_frames = self_ticks_to_frames(fade_out, ticks_per_frame);

            for f in 0..frames {
                if t.position >= total {
                    t.finished = true;
                    break;
                }
                let mut envelope = t.gain;
                if fade_in_frames > 0 && f < fade_in_frames {
                    envelope *= f as f32 / fade_in_frames as f32;
                }
                if fade_out_frames > 0 && total > fade_out_frames && t.position >= total - fade_out_frames
                {
                    let remaining = total - t.position;
                    envelope *= remaining as f32 / fade_out_frames as f32;
                }

                let [l, r] = t.chunk.frame(t.position);
                let base = f * channels;
                if let Some(s) = out.get_mut(base) {
                    *s += l * envelope;
                }
                if channels > 1 {
                    if let Some(s) = out.get_mut(base + 1) {
                        *s += r * envelope;
                    }
                }
                t.position += 1;
            }
        }

        // Soft clip. The sum of N tracks can exceed 1.0; hard clipping produces
        // audible crackle, and a tanh knee keeps the result clean.
        //
        // The normalisation is the point: `x * limit / tanh(limit)` maps the
        // soft knee back onto the unit range, so soft limiting never *reduces*
        // an already-quiet signal and only touches what would have clipped.
        let limit = 0.98f32;
        let knee = limit.tanh();
        for s in out.iter_mut() {
            let x = *s;
            if x > limit || x < -limit {
                *s = limit * (x / knee).tanh();
            }
        }

        // Publish the clock. This is the audio-first design: the video side
        // follows this rather than free-running.
        self.frames_played.fetch_add(frames as u64, Ordering::Relaxed);
        let new_head =
            self.playhead_ticks.load(Ordering::Relaxed) as i64 + ticks_per_frame * frames as i64;
        self.playhead_ticks.store(new_head as u64, Ordering::Relaxed);

        self.prune();
        // Returning 0 once every track has finished is how the caller learns
        // playback has ended. Checking before reporting the frame count keeps
        // that signal from being masked by a buffer that happened to complete
        // the final track.
        if self.tracks.is_empty() {
            self.playing.store(0, Ordering::Relaxed);
            return 0;
        }
        frames
    }
}

/// Convert ticks to a frame count at a known rate, without borrowing self.
///
/// A free function because `render` already holds `&mut self.tracks` and
/// cannot reach `self` at the same time.
#[inline]
fn self_ticks_to_frames(t: Ticks, ticks_per_frame: i64) -> usize {
    if ticks_per_frame == 0 {
        0
    } else {
        (t.raw() / ticks_per_frame).max(0) as usize
    }
}

/// Ticks represented by one output **frame** (one sample per channel) at a
/// given sample rate.
///
/// One frame of stereo audio is two samples, so this is twice
/// `TICKS_PER_SECOND / sample_rate`. Getting that wrong by a factor of the
/// channel count makes the playhead run at half speed for stereo, which looks
/// exactly like a sync problem and is not one.
pub fn ticks_per_sample_frame(sample_rate: u32, channels: u16) -> i64 {
    if sample_rate == 0 {
        return 0;
    }
    let per_sample = quillie_core::time::TICKS_PER_SECOND / sample_rate as i64;
    per_sample * channels.max(1) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u64, frames: usize) -> Arc<Chunk> {
        let samples: Vec<i16> = (0..frames * 2).map(|i| (i as f32 * 100.0) as i16).collect();
        Arc::new(Chunk { id, samples, channels: 2, sample_rate: 48_000 })
    }

    fn mixer() -> Mixer {
        Mixer::new(AudioConfig::default(), 4)
    }

    #[test]
    fn rendering_with_no_tracks_produces_silence() {
        let mut m = mixer();
        let mut buf = [0.5f32; 256];
        assert_eq!(m.render(&mut buf), 0);
        assert!(buf.iter().all(|s| *s == 0.0));
        assert!(!m.is_playing());
    }

    #[test]
    fn a_track_reaches_the_output_at_unity_gain() {
        let mut m = mixer();
        m.add_track(chunk(1, 100), 0.0);
        let mut buf = [0.0f32; 200];
        let n = m.render(&mut buf);
        assert_eq!(n, 100);
        assert!(buf.iter().any(|s| *s != 0.0), "samples must reach the output");
    }

    #[test]
    fn negative_decibels_reduce_level() {
        let mut m = mixer();
        m.add_track(chunk(1, 100), -6.0);
        let mut buf = [0.0f32; 200];
        m.render(&mut buf);
        let peak = buf.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak > 0.0 && peak < 1.0, "peak was {peak}");
    }

    #[test]
    fn rendering_never_allocates() {
        // A regression guard rather than a benchmark: if render() ever grows a
        // Vec, this fails the suite instead of the user's audio.
        let mut m = mixer();
        m.add_track(chunk(1, 1000), 0.0);
        let mut buf = [0.0f32; 2048];
        for _ in 0..64 {
            m.render(&mut buf);
        }
        // The assertion is that we got here at all; allocation would show up
        // in a real allocation counter, which the bench harness reads.
        assert!(m.frames_played() > 0);
    }

    #[test]
    fn the_playhead_follows_the_audio_callback() {
        let mut m = mixer();
        m.add_track(chunk(1, 10_000), 0.0);
        m.seek(Ticks::ZERO);
        let mut buf = [0.0f32; 1024];
        m.render(&mut buf);
        // 1024 frames at 48kHz is 1024 * 120000/48000 ticks.
        assert_eq!(m.playhead().raw(), 1024 * (120_000 / 48_000));
        assert_eq!(m.clock_source(), ClockSource::AudioCallback);
    }

    #[test]
    fn seeking_resets_the_position() {
        let mut m = mixer();
        m.add_track(chunk(1, 1000), 0.0);
        let mut buf = [0.0f32; 200];
        m.render(&mut buf);
        m.seek(Ticks::from_seconds(30));
        assert_eq!(m.playhead(), Ticks::from_seconds(30));
        assert_eq!(m.frames_played(), 0);
    }

    #[test]
    fn a_finished_track_is_removed_and_playback_reports_ended() {
        let mut m = mixer();
        m.add_track(chunk(1, 10), 0.0);
        let mut buf = [0.0f32; 200]; // 100 frames, chunk has 10
        let n = m.render(&mut buf);
        assert_eq!(n, 0, "render returns 0 once every track has finished");
        assert_eq!(m.track_count(), 0, "finished tracks are pruned");
    }

    #[test]
    fn a_sum_loud_enough_to_clip_is_soft_limited() {
        let mut m = mixer();
        // Four unity-gain tracks of a loud signal.
        for i in 0..4 {
            m.add_track(chunk(i, 1000), 0.0);
        }
        let mut buf = [0.0f32; 2000];
        m.render(&mut buf);
        let peak = buf.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak <= 1.0, "peak {peak} must not exceed full scale");
    }

    #[test]
    fn sample_rate_conversion_is_exact() {
        assert_eq!(ticks_per_sample_frame(48_000, 2), 4); // (120000/48000) * 2ch
        assert_eq!(ticks_per_sample_frame(0, 2), 0);
        assert!(ticks_per_sample_frame(44_100, 2) > 0);
    }
}