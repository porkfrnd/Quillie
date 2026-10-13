//! The Quillie project model (§6.2).
//!
//! This is the **only** source of truth for edits (architecture rule 1). The
//! UI holds a mirror built from patches plus transient interaction state; the
//! renderer holds nothing.
//!
//! Nothing here mutates a `Project` except through
//! [`Project::apply`](crate::project::Project::apply), which is what makes
//! undo total: every change returns an [`Inverse`](crate::project::Inverse)
//! that restores the previous state exactly.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::time::{FrameRate, Ticks};

/// Schema version written into every project file (§6.3).
///
/// Bump this and add a migration in [`schema`] whenever the on-disk shape
/// changes. A project saved at an older version must still open.
pub const SCHEMA_VERSION: u32 = 1;

pub type MediaId = Uuid;
pub type SequenceId = Uuid;
pub type TrackId = Uuid;
pub type ClipId = Uuid;
pub type CaptionId = Uuid;
pub type MarkerId = Uuid;
pub type LinkId = Uuid;
pub type GroupId = Uuid;

/// A path, absolute or relative to the project file.
///
/// Kept as a plain string rather than `PathBuf` so the JSON stays portable
/// between the platforms Quillie ships for.
pub type PathString = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    pub frame_rate: FrameRate,
    pub resolution: (u32, u32),
    pub sample_rate: u32,
    pub media: BTreeMap<MediaId, MediaRef>,
    pub sequences: Vec<Sequence>,
    pub captions: Vec<Caption>,
    pub markers: Vec<Marker>,
    pub snapshots: Vec<NamedSnapshot>,
    pub export_presets: Vec<ExportPreset>,
}

impl Default for Project {
    fn default() -> Self {
        Self::new("Untitled", FrameRate::new(30, 1), (1920, 1080))
    }
}

impl Project {
    pub fn new(name: &str, frame_rate: FrameRate, resolution: (u32, u32)) -> Self {
        let mut project = Project {
            schema_version: SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: name.to_string(),
            frame_rate,
            resolution,
            sample_rate: 48_000,
            media: BTreeMap::new(),
            sequences: Vec::new(),
            captions: Vec::new(),
            markers: Vec::new(),
            snapshots: Vec::new(),
            export_presets: Vec::new(),
        };
        // A project always has at least one sequence (§6.2).
        project.sequences.push(Sequence::new("Main"));
        project
    }

    /// The sequence Quick Edit and the timeline show by default.
    pub fn primary_sequence(&self) -> &Sequence {
        &self.sequences[0]
    }

    pub fn primary_sequence_mut(&mut self) -> &mut Sequence {
        &mut self.sequences[0]
    }

    pub fn sequence(&self, id: SequenceId) -> Option<&Sequence> {
        self.sequences.iter().find(|s| s.id == id)
    }

    /// The combined length of every track, which is the timeline duration.
    pub fn duration(&self) -> Ticks {
        self.sequences
            .iter()
            .map(|s| s.duration())
            .max()
            .unwrap_or(Ticks::ZERO)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,
    pub resolution: (u32, u32),
    pub tracks: Vec<Track>,
    pub transitions: Vec<Transition>,
}

impl Sequence {
    pub fn new(name: &str) -> Self {
        Sequence {
            id: Uuid::new_v4(),
            name: name.to_string(),
            resolution: (1920, 1080),
            tracks: Vec::new(),
            transitions: Vec::new(),
        }
    }

    /// The furthest point any clip reaches.
    pub fn duration(&self) -> Ticks {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end())
            .max()
            .unwrap_or(Ticks::ZERO)
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// Is any track in this sequence soloed?
    ///
    /// Per sequence rather than per project: solo is a mixing decision made
    /// about one arrangement, and a project with several sequences of the same
    /// footage should be able to solo differently in each. Making this global
    /// would mean the answer depends on tracks the user is not looking at.
    pub fn any_solo(&self) -> bool {
        self.tracks.iter().any(|t| t.solo)
    }
}

/// How a track behaves. Determines which clips may live on it and whether it
/// contributes to the audio mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    /// Drawn above the base video track. Composited, never mixed.
    Overlay,
    Audio,
    Caption,
}

impl TrackKind {
    /// The switches that mean anything on a track of this kind.
    ///
    /// A video track has no audio to mute, so "mute" on it would be a control
    /// that cannot do anything — the decoy-control failure this project treats
    /// as a defect. Its mute is "hide" instead, which is what every mainstream
    /// editor calls it. Solo is an audio concept too: no mainstream NLE offers
    /// it on a video track, because soloing one video track does not select a
    /// subset of anything — it just hides the rest.
    ///
    /// Driven from the kind rather than hard-coded per header row in the UI, so
    /// the header cannot disagree with the model about which switches exist.
    pub fn controls(self) -> &'static [TrackFlag] {
        const AUDIO_CONTROLS: &[TrackFlag] =
            &[TrackFlag::Muted, TrackFlag::Solo, TrackFlag::Locked];
        const VISUAL_CONTROLS: &[TrackFlag] = &[TrackFlag::Hidden, TrackFlag::Locked];
        match self {
            TrackKind::Audio => AUDIO_CONTROLS,
            TrackKind::Video | TrackKind::Overlay | TrackKind::Caption => VISUAL_CONTROLS,
        }
    }
}

/// One of the per-track switches a user can flip.
///
/// An enum rather than four separate booleans passed around by name because it
/// makes "which switches exist" a closed set the compiler can check. Adding a
/// fifth flag then fails to compile everywhere a flag is read or written,
/// rather than silently reading `false` for the one nobody remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackFlag {
    Muted,
    Solo,
    Locked,
    Hidden,
}

impl TrackFlag {
    /// Read this flag off a track.
    ///
    /// Exhaustive so that adding a variant is a compile error here rather than
    /// a flag that reads `false` on every track forever.
    pub fn get(self, track: &Track) -> bool {
        match self {
            TrackFlag::Muted => track.muted,
            TrackFlag::Solo => track.solo,
            TrackFlag::Locked => track.locked,
            TrackFlag::Hidden => track.hidden,
        }
    }

    /// Write this flag on a track.
    ///
    /// The single place any flag is assigned. An edit that sets a flag goes
    /// through here, so "which code can change a track flag" has one answer.
    pub fn set(self, track: &mut Track, value: bool) {
        match self {
            TrackFlag::Muted => track.muted = value,
            TrackFlag::Solo => track.solo = value,
            TrackFlag::Locked => track.locked = value,
            TrackFlag::Hidden => track.hidden = value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub clips: Vec<Clip>,
    pub muted: bool,
    pub locked: bool,
    pub hidden: bool,
    /// Added after schema v1 shipped, so a v1 file without the key still loads
    /// and reads as not soloed — which is what every project written before
    /// solo existed means. `#[serde(default)]` is what keeps that true; without
    /// it, opening an older project would fail outright.
    #[serde(default)]
    pub solo: bool,
    pub height_px: f32,
}

impl Track {
    pub fn new(kind: TrackKind, name: &str) -> Self {
        let height_px = match kind {
            TrackKind::Video => 80.0,
            TrackKind::Overlay => 60.0,
            TrackKind::Audio => 56.0,
            TrackKind::Caption => 40.0,
        };
        Track {
            id: Uuid::new_v4(),
            kind,
            name: name.to_string(),
            clips: Vec::new(),
            muted: false,
            locked: false,
            hidden: false,
            solo: false,
            height_px,
        }
    }

    /// Does this track contribute to what the user hears?
    ///
    /// Solo is stored per track but its *effect* is derived here rather than
    /// written into `muted`. The tempting alternative — soloing a track
    /// rewrites every other track's `muted`, and un-soloing rewrites them
    /// back — needs somewhere to keep the flags it overwrote, and that
    /// somewhere has to be restored exactly. Deriving instead makes clearing
    /// the last solo restore every `muted` flag to its stored value for free,
    /// because nothing was ever overwritten.
    ///
    /// An explicit `muted` beats `solo`: silence is a stronger statement than
    /// "play this one", so a track that is both muted and soloed stays silent.
    pub fn audible(&self, any_solo_in_sequence: bool) -> bool {
        !self.muted && (!any_solo_in_sequence || self.solo)
    }

    /// Is this track drawn?
    ///
    /// Hidden is a user preference about the timeline, not about the export —
    /// whether a hidden track reaches the output is decided where the export
    /// graph is built, not here.
    pub fn visible(&self) -> bool {
        !self.hidden
    }
}

/// A clip on the timeline.
///
/// `media` is `None` for generated clips (title, solid, shape) — those have no
/// source file and are rendered by the overlay stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub media: Option<MediaId>,
    pub timeline_start: Ticks,
    pub source_in: Ticks,
    pub source_out: Ticks,
    pub speed: Animatable<f32>,
    pub transform: Transform,
    pub opacity: Animatable<f32>,
    pub audio: ClipAudio,
    pub effects: Vec<EffectInstance>,
    pub link: Option<LinkId>,
    pub group: Option<GroupId>,
}

impl Clip {
    pub fn new(media: Option<MediaId>, timeline_start: Ticks, source_in: Ticks, source_out: Ticks) -> Self {
        Clip {
            id: Uuid::new_v4(),
            media,
            timeline_start,
            source_in,
            source_out,
            speed: Animatable::Static(1.0),
            transform: Transform::default(),
            opacity: Animatable::Static(1.0),
            audio: ClipAudio::default(),
            effects: Vec::new(),
            link: None,
            group: None,
        }
    }

    /// Source duration, before speed is applied.
    pub fn source_duration(&self) -> Ticks {
        Ticks((self.source_out.raw() - self.source_in.raw()).max(0))
    }

    /// Length on the timeline. Speed is applied here, which is why a speed ramp
    /// changes how long a clip occupies without changing its source range.
    ///
    /// Uses `constant_or(1.0)` rather than `constant()`: a keyframed speed must
    /// not panic the duration computation, and treating it as 1x is the safe
    /// assumption until the motion engine can evaluate it.
    pub fn duration(&self) -> Ticks {
        let base = self.speed.constant_or(1.0).max(f32::MIN_POSITIVE);
        Ticks((self.source_duration().raw() as f32 / base).round() as i64)
    }

    /// Where the clip ends on the timeline.
    pub fn end(&self) -> Ticks {
        Ticks(self.timeline_start.raw() + self.duration().raw())
    }

    pub fn overlaps(&self, other: &Clip) -> bool {
        self.timeline_start < other.end() && other.timeline_start < self.end()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipAudio {
    pub gain_db: Animatable<f32>,
    pub fade_in: Ticks,
    pub fade_out: Ticks,
}

impl Default for ClipAudio {
    fn default() -> Self {
        ClipAudio {
            gain_db: Animatable::Static(0.0),
            fade_in: Ticks::ZERO,
            fade_out: Ticks::ZERO,
        }
    }
}

/// A value that can be constant, keyframed, or driven by analysis data.
///
/// Keyframes and `Driven` are Phase 4 work. The variants exist now so the
/// on-disk shape does not need a migration when they land.
///
/// # Serialization shape
///
/// This uses **adjacent** tagging, not internal, because serde cannot
/// internally tag a newtype variant containing a bare float (`Static(f32)`
/// is not a map). Adjacent tagging writes:
///
/// ```json
/// {"kind": "static", "value": 1.0}
/// {"kind": "keyframed", "keyframes": [...]}
/// {"kind": "driven", "source": {...}, "map": {...}}
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Animatable<T> {
    Static(T),
    Keyframed(Vec<Keyframe<T>>),
    Driven { source: DataSource, map: Mapping },
}

impl<T: Copy> Animatable<T> {
    /// The constant value.
    ///
    /// # Panics
    /// Panics when this is keyframed or driven. Use this only where a
    /// non-animated layout decision has to be made before the motion engine
    /// exists; never on a path that evaluates a renderable property.
    pub fn constant(&self) -> T {
        match self {
            Animatable::Static(v) => *v,
            _ => unreachable!("constant() on a non-static Animatable"),
        }
    }

    /// The constant value, or `fallback` when animated.
    ///
    /// The form to use in the export planner and anywhere else that runs
    /// before the motion engine. Silently returning a default from
    /// [`constant`](Self::constant) would hide a keyframed property behind a
    /// plausible-looking number.
    pub fn constant_or(&self, fallback: T) -> T {
        match self {
            Animatable::Static(v) => *v,
            _ => fallback,
        }
    }

    /// Is this a plain constant?
    pub fn is_static(&self) -> bool {
        matches!(self, Animatable::Static(_))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe<T> {
    pub time: Ticks,
    pub value: T,
    pub interp: Interp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Interp {
    Hold,
    Linear,
    Bezier { x1: f32, y1: f32, x2: f32, y2: f32 },
    Named(EasingId),
    Spring(SpringParams),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpringParams {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub initial_velocity: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EasingId(pub String);

/// What an audio-reactive or tracking-driven value reads from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataSource {
    Tracker { track_id: String, channel: String },
    AudioEnvelope { media: MediaId, band: String, smoothing: f32 },
    BeatPulse { marker_set: String, decay: f32 },
}

/// How a `Driven` source value becomes a property value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    pub input_min: f32,
    pub input_max: f32,
    pub output_min: f32,
    pub output_max: f32,
    pub curve: f32,
    pub smoothing: Ticks,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: Animatable<f32>,
    pub y: Animatable<f32>,
    pub scale_x: Animatable<f32>,
    pub scale_y: Animatable<f32>,
    pub rotation_deg: Animatable<f32>,
    pub anchor_x: Animatable<f32>,
    pub anchor_y: Animatable<f32>,
}

impl Default for Transform {
    fn default() -> Self {
        use Animatable::Static;
        Transform {
            x: Static(0.0),
            y: Static(0.0),
            scale_x: Static(1.0),
            scale_y: Static(1.0),
            rotation_deg: Static(0.0),
            anchor_x: Static(0.5),
            anchor_y: Static(0.5),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectInstance {
    pub id: Uuid,
    /// Matches an `EffectSpec` id in the effect catalogue.
    pub effect: String,
    pub enabled: bool,
    /// Params are stored untyped and validated against the spec at load time,
    /// so a params change does not need a project migration.
    pub params: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub id: Uuid,
    pub track: TrackId,
    pub from_clip: ClipId,
    pub to_clip: ClipId,
    pub duration: Ticks,
    pub kind: String,
    pub params: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaRef {
    pub id: MediaId,
    pub path: PathString,
    pub fingerprint: Fingerprint,
    pub duration: Ticks,
    pub streams: StreamSummary,
    pub rotation: u16,
    pub vfr: bool,
}

/// Cheap identity for a source file (§7.1).
///
/// Size, mtime, and a hash of the first and last MiB. Deliberately **not** a
/// whole-file hash: on a mechanical disk that would turn every import into a
/// multi-minute stall, and the point of a fingerprint is to notice that a file
/// *changed*, not to prove its contents forever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub size_bytes: u64,
    pub mtime_unix: i64,
    pub head_hash: [u8; 16],
    pub tail_hash: [u8; 16],
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamSummary {
    pub video: Option<VideoStreamInfo>,
    pub audio: Option<AudioStreamInfo>,
    pub has_subtitles: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoStreamInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: Option<String>,
    /// Bits per sample, for 10-bit sources. Drives the proxy decision (§7.2).
    pub bits_per_raw_sample: Option<u16>,
    pub avg_frame_rate: Option<(u32, u32)>,
    pub nb_frames: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamInfo {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub channel_layout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub id: CaptionId,
    pub start: Ticks,
    pub end: Ticks,
    pub text: String,
    pub words: Option<Vec<WordTiming>>,
    pub style: String,
    pub origin: CaptionOrigin,
    /// Set when a human edits the text. Re-transcription must never silently
    /// overwrite an edited caption (§12).
    pub edited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionOrigin {
    Auto,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordTiming {
    pub start: Ticks,
    pub end: Ticks,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub time: Ticks,
    pub kind: MarkerKind,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    Note,
    Chapter,
    Beat,
    Scene,
    Silence,
}

/// A small named copy of the project, for restore points and compare.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedSnapshot {
    pub id: Uuid,
    pub name: String,
    pub created_at_unix: i64,
    pub project: Box<Project>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    pub id: Uuid,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    pub video_bitrate_bps: u64,
    pub audio_codec: String,
    pub audio_bitrate_bps: u64,
}

impl ExportPreset {
    /// The codec name ffprobe will report for this preset's video.
    ///
    /// The preset stores an FFmpeg *encoder* name (`libvpx_vp9`) while ffprobe
    /// reports a *codec* name (`vp9`). Conflating them means either a
    /// validation that never passes or one that never runs.
    ///
    /// Note the encoder name uses an underscore while FFmpeg 9 expects a
    /// hyphen (`libvpx-vp9`); `ffmpeg_video_args` translates it at the point of
    /// use rather than storing a name that only one of the two tools accepts.
    pub fn expected_codec(&self) -> &str {
        match self.video_codec.as_str() {
            v if v.contains("vp9") => "vp9",
            v if v.contains("av1") => "av1",
            v if v.contains("x264") => "h264",
            v if v.contains("x265") => "hevc",
            other => other,
        }
    }

    pub fn landscape_1080p() -> Self {
        ExportPreset {
            id: Uuid::new_v4(),
            name: "1080p".into(),
            width: 1920,
            height: 1080,
            video_codec: "libvpx_vp9".into(),
            video_bitrate_bps: 8_000_000,
            audio_codec: "aac".into(),
            audio_bitrate_bps: 192_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sequence holding one track of each kind that differs in its controls.
    fn sequence_with(kinds: &[(TrackKind, bool)]) -> Sequence {
        let mut seq = Sequence::new("Main");
        for (kind, muted) in kinds {
            let mut track = Track::new(*kind, "t");
            track.muted = *muted;
            seq.tracks.push(track);
        }
        seq
    }

    #[test]
    fn with_nothing_soloed_each_track_is_audible_unless_it_is_muted() {
        let seq = sequence_with(&[(TrackKind::Audio, false), (TrackKind::Audio, true)]);
        assert!(!seq.any_solo(), "nothing is soloed, so the question is mute alone");
        assert!(seq.tracks[0].audible(seq.any_solo()));
        assert!(!seq.tracks[1].audible(seq.any_solo()));
    }

    #[test]
    fn soloing_one_track_inaudibles_every_other_track_regardless_of_its_own_mute() {
        let mut seq = sequence_with(&[(TrackKind::Audio, false), (TrackKind::Audio, false)]);
        seq.tracks[0].solo = true;

        assert!(seq.any_solo());
        assert!(seq.tracks[0].audible(seq.any_solo()));
        // The second track's `muted` is false. It is still inaudible, because
        // something else in the sequence is soloed. This is the case that the
        // stored-mute design would have handled by overwriting this flag.
        assert!(!seq.tracks[1].muted);
        assert!(!seq.tracks[1].audible(seq.any_solo()));
    }

    #[test]
    fn clearing_the_last_solo_restores_every_stored_mute_because_none_was_ever_overwritten() {
        let mut seq = sequence_with(&[
            (TrackKind::Audio, true),  // explicitly muted before soloing
            (TrackKind::Audio, false),
            (TrackKind::Audio, true),
        ]);
        let before: Vec<bool> = seq.tracks.iter().map(|t| t.muted).collect();

        seq.tracks[1].solo = true;
        assert!(!seq.tracks[0].audible(seq.any_solo()));
        assert!(!seq.tracks[2].audible(seq.any_solo()));

        seq.tracks[1].solo = false;
        assert!(!seq.any_solo());

        // Byte-for-byte: the flags a user set by hand are exactly what they
        // were. There is no "pre-solo mute state" to have been lost.
        let after: Vec<bool> = seq.tracks.iter().map(|t| t.muted).collect();
        assert_eq!(before, after);
        assert!(!seq.tracks[0].audible(seq.any_solo()), "still muted by hand");
        assert!(seq.tracks[1].audible(seq.any_solo()), "back to audible");
        assert!(!seq.tracks[2].audible(seq.any_solo()), "still muted by hand");
    }

    #[test]
    fn an_explicit_mute_beats_solo_on_the_same_track() {
        let mut track = Track::new(TrackKind::Audio, "A1");
        track.muted = true;
        track.solo = true;
        // Silence is a stronger statement than "play this one".
        assert!(!track.audible(true));
    }

    #[test]
    fn visible_is_the_negation_of_hidden_and_says_nothing_about_audibility() {
        let mut track = Track::new(TrackKind::Video, "V1");
        assert!(track.visible());
        TrackFlag::Hidden.set(&mut track, true);
        assert!(!track.visible());
        // Hiding a video track is a timeline preference. It must not be
        // reported as "this track is silent", which is a different question.
        assert!(track.audible(false));
    }

    #[test]
    fn flags_round_trip_through_get_and_set_for_every_flag() {
        let mut track = Track::new(TrackKind::Audio, "A1");
        for flag in [
            TrackFlag::Muted,
            TrackFlag::Solo,
            TrackFlag::Locked,
            TrackFlag::Hidden,
        ] {
            assert!(!flag.get(&track), "{flag:?} starts false");
            flag.set(&mut track, true);
            assert!(flag.get(&track), "{flag:?} did not take the write");
            flag.set(&mut track, false);
            assert!(!flag.get(&track), "{flag:?} did not take the clear");
        }
    }

    #[test]
    fn only_audio_offers_mute_and_solo_and_every_kind_offers_lock() {
        assert_eq!(
            TrackKind::Audio.controls(),
            &[TrackFlag::Muted, TrackFlag::Solo, TrackFlag::Locked]
        );
        for kind in [TrackKind::Video, TrackKind::Overlay, TrackKind::Caption] {
            assert_eq!(
                kind.controls(),
                &[TrackFlag::Hidden, TrackFlag::Locked],
                "{kind:?} offers hide and lock"
            );
        }
    }

    #[test]
    fn a_track_serialised_without_a_solo_key_loads_as_not_soloed() {
        // A v1 project file predates this field. The JSON is written by hand
        // rather than by deleting the key from a round trip, so this pins the
        // on-disk shape an old file actually has.
        let v1 = r#"{
            "id": "0192b3f4-0000-7000-8000-000000000001",
            "kind": "Audio",
            "name": "A1",
            "clips": [],
            "muted": false,
            "locked": false,
            "hidden": false,
            "height_px": 56.0
        }"#;
        let track: Track = serde_json::from_str(v1).expect("a v1 track must still parse");
        assert!(!track.solo);
        assert_eq!(track.name, "A1");
    }
}