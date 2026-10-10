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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    pub clips: Vec<Clip>,
    pub muted: bool,
    pub locked: bool,
    pub hidden: bool,
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
            height_px,
        }
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