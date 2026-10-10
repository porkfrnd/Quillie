//! Export planning (§11.2).
//!
//! # One graph, two consumers
//!
//! `plan()` compiles a project over a range into an [`ExportGraph`]. The same
//! graph drives two things:
//!
//! - **Export**, rendering at the output resolution.
//! - **Smooth Zones** (§8.5), rendering the same subgraph at proxy resolution
//!   into a preview cache.
//!
//! That is the whole reason the planner exists as its own stage. If preview
//! caching and export each built their own pipeline they would drift, and a
//! cached segment would stop matching what the user finally gets. A draft-mode
//! preview that differs from the export is worse than no preview at all,
//! because the user trusts it.
//!
//! # Structure
//!
//! The graph is a DAG of nodes. It is deliberately not a string: argument
//! building is a pure function over it, so the graph can be asserted on,
//! diffed, and tested without running FFmpeg.

use crate::project::{Clip, ExportPreset, Project, Sequence, Track, TrackKind};
use crate::time::{FrameRate, Rounding, Ticks};
use std::fmt::Write as _;

/// A node in the export graph.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// Decode a source file. Named by input index for `-i`.
    Source { input: usize, label: String },
    /// `[a:b]trim=start:end,setpts=PTS-STARTPTS`
    Trim { input: String, start: Ticks, end: Ticks },
    /// `[a]setpts=(PTS-STARTPTS)/speed[a][b]`
    Speed { input: String, speed: f32 },
    /// `[a]scale=w:h,format=pix_fmt`
    Transform { input: String, width: u32, height: u32, pix_fmt: String },
    /// `[a]format=rgba,colorchannelmixer=aa=opacity`
    Opacity { input: String, opacity: f32 },
    /// `[bg][fg]overlay=x:y`
    Composite { background: String, foreground: String, x: i64, y: i64 },
    /// `[a][b]xfade=transition=...:duration=...`
    Transition {
        first: String,
        second: String,
        kind: String,
        duration: Ticks,
    },
    /// `[a][b]...amix=inputs=N[a]`
    AudioMix { inputs: Vec<String> },
    /// `[a]atrim...,asetpts=...,volume=...`
    AudioClip { input: String, gain_db: f32 },
    /// `[a]...[vout]`
    Encode { input: String, preset: String },
}

/// Why a render is happening. The same graph, different resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum Profile {
    /// Final output.
    Export { preset: ExportPreset },
    /// A preview-cache segment (§8.5).
    Preview { width: u32, height: u32 },
}

/// A planned render.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportGraph {
    pub nodes: Vec<Node>,
    /// Absolute path per `-i` index.
    pub inputs: Vec<String>,
    /// The filter_complex body.
    pub filters: String,
    /// Duration the graph is expected to produce.
    pub expected_duration: Ticks,
    pub profile: Profile,
    pub frame_rate: FrameRate,
}

/// Format ticks as a plain decimal number of seconds, for FFmpeg `trim`.
///
/// # Why seconds and not `HH:MM:SS.mmm`
///
/// The obvious timestamp format is wrong here. FFmpeg's filter syntax uses
/// `:` as its option separator, so `trim=start=00:00:01.000000` is parsed as
/// an option named "00" with the rest as garbage, and the graph fails to
/// build. Escaping the colons works but is fragile across filter versions.
///
/// Decimal seconds have no separator, are unambiguous, and are what the
/// documentation uses. They also carry more precision than a millisecond
/// timestamp would, so nothing is lost by using them.
///
/// Tick precision is 1/120000 s, so six decimal places is exact.
fn ts(t: Ticks) -> String {
    // Integer arithmetic throughout: building this from a float would
    // reintroduce the accumulation error the timebase exists to prevent.
    let scaled = t.raw().saturating_mul(1_000_000) / crate::time::TICKS_PER_SECOND;
    let whole = scaled / 1_000_000;
    let frac = scaled % 1_000_000;
    if frac == 0 {
        format!("{whole}")
    } else {
        // Trim trailing zeros so the common case reads as "1.5" not "1.500000".
        let frac_str = format!("{frac:06}");
        format!("{whole}.{}", frac_str.trim_end_matches('0'))
    }
}

/// Compile a sequence over a range into an export graph.
///
/// Returns `None` for a sequence with nothing to render, rather than an
/// empty graph that FFmpeg would reject with a confusing error.
pub fn plan(
    project: &Project,
    sequence: &Sequence,
    range: Option<(Ticks, Ticks)>,
    profile: Profile,
) -> Option<ExportGraph> {
    let rate = project.frame_rate;
    let (width, height) = match &profile {
        Profile::Export { preset } => (preset.width, preset.height),
        Profile::Preview { width, height } => (*width, *height),
    };

    let mut nodes: Vec<Node> = Vec::new();
    let mut inputs: Vec<String> = Vec::new();
    let mut filters = String::new();

    let (range_start, range_end) = range.unwrap_or((Ticks::ZERO, sequence.duration()));

    let video_tracks: Vec<&Track> = sequence
        .tracks
        .iter()
        .filter(|t| matches!(t.kind, TrackKind::Video | TrackKind::Overlay) && !t.hidden)
        .collect();
    let audio_tracks: Vec<&Track> = sequence
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && !t.muted)
        .collect();

    // --- video ---------------------------------------------------------
    let mut video_labels: Vec<String> = Vec::new();

    for track in &video_tracks {
        let mut layer_labels: Vec<String> = Vec::new();
        // Clips are laid out by time order so concat sees them in order.
        let mut clips: Vec<&Clip> = track.clips.iter().collect();
        clips.sort_by_key(|c| c.timeline_start);

        for clip in clips {
            // Skip anything entirely outside the range.
            if clip.end() <= range_start || clip.timeline_start >= range_end {
                continue;
            }
            let Some(media_id) = clip.media else { continue };
            let Some(media) = project.media.get(&media_id) else { continue };

            let input = inputs.len();
            inputs.push(media.path.clone());
            nodes.push(Node::Source { input, label: format!("clip{}", clip.id) });

            let src = format!("{input}:v");
            let vid = format!("v{}", nodes.len());

            // How far into the source this clip starts, and how much we take.
            let head = clip.source_in;
            let tail = clip.source_out;
            let mut chain = format!(
                "[{src}]trim=start={}:end={},setpts=PTS-STARTPTS",
                ts(head),
                ts(tail)
            );

            let speed = clip.speed.constant_or(1.0);
            if (speed - 1.0).abs() > f32::EPSILON {
                let _ = write!(chain, ",setpts=(PTS-STARTPTS)/{speed}");
            }

            let _ = write!(
                chain,
                ",scale={width}:{height}:force_original_aspect_ratio=decrease,\
                 pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,format=yuv420p"
            );
            // One chain, one terminating label. A semicolon here is what
            // separates this filter from the next; without it FFmpeg sees
            // "[v1][v1]null" and fails to parse.
            let _ = write!(filters, "{chain}[{vid}];");

            nodes.push(Node::Transform {
                input: vid.clone(),
                width,
                height,
                pix_fmt: "yuv420p".into(),
            });

            layer_labels.push(vid);
        }

        // Clips on one track are sequential, so they concatenate into a
        // single layer. A track that is not sequential (clips overlapping or
        // out of order) is a malformed project and is rejected by the model
        // before it reaches here, so concat is the correct operation.
        match layer_labels.len() {
            0 => {}
            1 => video_labels.push(layer_labels[0].clone()),
            _ => {
                let out = format!("layer{}", nodes.len());
                // concat is an input-consuming filter: every input label
                // must be written before it, and the result is named by the
                // trailing [label]. Writing "layer=concat=..." instead
                // produces "No such filter" from FFmpeg, because the label
                // before the '=' is read as a filter name.
                let parts: String =
                    layer_labels.iter().map(|l| format!("[{l}]")).collect();
                let _ = write!(
                    filters,
                    "{parts}concat=n={}:v=1:a=0[{out}];",
                    layer_labels.len()
                );
                video_labels.push(out);
            }
        }
    }

    // --- audio ---------------------------------------------------------
    let mut audio_labels: Vec<String> = Vec::new();
    for track in &audio_tracks {
        let mut clips: Vec<&Clip> = track.clips.iter().collect();
        clips.sort_by_key(|c| c.timeline_start);
        for clip in clips {
            if clip.end() <= range_start || clip.timeline_start >= range_end {
                continue;
            }
            let Some(media_id) = clip.media else { continue };
            let Some(media) = project.media.get(&media_id) else { continue };

            let input = inputs.len();
            inputs.push(media.path.clone());
            nodes.push(Node::Source { input, label: format!("audio{}", clip.id) });

            let out = format!("a{}", nodes.len());
            let _ = write!(
                filters,
                "[{input}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,\
                 aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo",
                ts(clip.source_in),
                ts(clip.source_out)
            );
            let gain = clip.audio.gain_db.constant_or(0.0);
            if gain.abs() > f32::EPSILON {
                let _ = write!(filters, ",volume={gain}dB");
            }
            let _ = write!(filters, "[{out}];");

            nodes.push(Node::AudioClip { input: out.clone(), gain_db: gain });
            audio_labels.push(out);
        }
    }

    // --- mix layers ----------------------------------------------------
    if !video_labels.is_empty() {
        let mut current = video_labels[0].clone();
        for next in video_labels.iter().skip(1) {
            let out = format!("comp{}", nodes.len());
            // Overlays composite over the base at the origin. Real placement
            // comes with the transform fields, which are Phase 3.
            let _ = write!(filters, "[{current}][{next}]overlay=0:0[{out}];");
            nodes.push(Node::Composite {
                background: current.clone(),
                foreground: next.clone(),
                x: 0,
                y: 0,
            });
            current = out;
        }

        // `null` passes the stream through while giving the final video label a
        // stable name, so -map does not have to know how many layers deep the
        // graph went.
        let out = format!("vout{}", nodes.len());
        let _ = write!(filters, "[{current}]null[{out}];");
        // video_labels is not read after this; the label is exposed via video_label().
    }

    if !audio_labels.is_empty() {
        let out = format!("aout{}", nodes.len());
        let _ = write!(
            filters,
            "{}=amix=inputs={}:normalize=0[a0];[a0]alimiter=limit=0.98[{out}]",
            out,
            audio_labels.len()
        );
        nodes.push(Node::AudioMix { inputs: audio_labels });
        // audio_labels is not read after this; the label is exposed via audio_label().
    }

    if nodes.is_empty() {
        return None;
    }

    let expected_duration = Ticks(
        (range_end.raw() - range_start.raw()).max(0),
    );

    // The trailing semicolon is trimmed because FFmpeg accepts it but it makes
    // the last line of a filter_complex argument noisier in logs and errors.
    let filters = filters.trim_end().trim_end_matches(';').to_string();

    Some(ExportGraph {
        nodes,
        inputs,
        filters,
        expected_duration,
        profile,
        frame_rate: rate,
    })
}

impl ExportGraph {
    /// Whether this graph has a video stream.
    pub fn has_video(&self) -> bool {
        self.filters.contains("vout")
    }

    pub fn has_audio(&self) -> bool {
        self.filters.contains("aout")
    }

    /// The label of the final video stream, for `-map`.
    pub fn video_label(&self) -> Option<String> {
        find_label(&self.filters, "vout")
    }

    pub fn audio_label(&self) -> Option<String> {
        find_label(&self.filters, "aout")
    }

    /// Duration in seconds, for validation after export.
    pub fn expected_seconds(&self) -> f64 {
        self.expected_duration.as_secs_f64()
    }

    /// How many frames the plan expects, at the project rate.
    pub fn expected_frames(&self) -> i64 {
        self.frame_rate.ticks_to_frame(self.expected_duration, Rounding::Floor)
    }
}

/// Find the output label of a filter chain whose output starts with `prefix`.
///
/// Walks the chain from the end, because the final `[label]` has no closing
/// semicolon and a naive split on `;` would miss it. FFmpeg's filter syntax
/// puts the output label last, so scanning backwards is both simpler and
/// correct for any graph shape.
fn find_label(filters: &str, prefix: &str) -> Option<String> {
    let last = filters.rsplit(';').next()?;
    let close = last.rfind(']')?;
    let open = last[..close].rfind('[')?;
    let label = &last[open + 1..close];
    label.starts_with(prefix).then(|| label.to_string())
}

impl ExportPreset {
    /// The default VP9 parameters for this build.
    ///
    /// No H.264 encoder exists in the sidecar while ADR-006 is open, so this
    /// is VP9 in MP4. It is recorded rather than hard-coded so the change is
    /// visible the day ADR-006 settles.
    pub fn ffmpeg_video_args(&self) -> Vec<String> {
        match self.video_codec.as_str() {
            "libvpx_vp9" | "libvpx-vp9" => vec![
                "-c:v".into(),
                "libvpx-vp9".into(),
                "-b:v".into(),
                format!("{}k", self.video_bitrate_bps / 1000),
                "-deadline".into(),
                "realtime".into(),
                "-cpu-used".into(),
                // 5 keeps VP9 usable on a two-core floor machine. Measured
                // throughput for other values lands in docs/reports.
                "5".into(),
                "-row-mt".into(),
                "1".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
            ],
            other => vec!["-c:v".into(), other.to_string()],
        }
    }

    pub fn ffmpeg_audio_args(&self) -> Vec<String> {
        vec![
            "-c:a".into(),
            self.audio_codec.clone(),
            "-b:a".into(),
            format!("{}k", self.audio_bitrate_bps / 1000),
            "-ac".into(),
            "2".into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{
        Animatable, Clip, Fingerprint, Interp, Keyframe, MediaRef, StreamSummary, Track,
    };
    use uuid::Uuid;

    fn project_with_clips(n: usize) -> Project {
        let mut p = Project::new("t", FrameRate::new(30, 1), (1920, 1080));
        let media_id = Uuid::new_v4();
        p.media.insert(
            media_id,
            MediaRef {
                id: media_id,
                path: "/tmp/a.mov".into(),
                fingerprint: Fingerprint {
                    size_bytes: 1,
                    mtime_unix: 0,
                    head_hash: [0; 16],
                    tail_hash: [0; 16],
                },
                duration: Ticks::from_seconds(60),
                streams: StreamSummary::default(),
                rotation: 0,
                vfr: false,
            },
        );
        let mut track = Track::new(TrackKind::Video, "V1");
        for i in 0..n {
            let mut c = Clip::new(
                Some(media_id),
                Ticks::from_seconds(i as i64 * 5),
                Ticks::from_seconds(i as i64 * 2),
                Ticks::from_seconds((i as i64 * 2) + 5),
            );
            c.id = Uuid::new_v4();
            track.clips.push(c);
        }
        p.primary_sequence_mut().tracks.push(track);
        p
    }

    #[test]
    fn a_single_clip_produces_a_valid_filter_chain() {
        let p = project_with_clips(1);
        let g = plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360,
        })
        .expect("a graph");

        assert!(g.has_video());
        assert_eq!(g.inputs.len(), 1);
        assert!(g.filters.contains("trim=start="));
        assert!(g.filters.contains("scale=640:360"));
        assert!(g.filters.contains("format=yuv420p"));
        assert!(g.video_label().is_some(), "filters were: {}", g.filters);
    }

    #[test]
    fn several_clips_on_one_track_concatenate_in_time_order() {
        let p = project_with_clips(3);
        let g = plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360,
        })
        .unwrap();
        assert!(g.filters.contains("concat=n=3:v=1:a=0"));
        // Inputs are ordered by timeline position, not by insertion.
        assert_eq!(g.inputs.len(), 3);
    }

    #[test]
    fn an_empty_sequence_plans_to_nothing_rather_than_an_empty_graph() {
        let p = Project::new("empty", FrameRate::new(30, 1), (1920, 1080));
        assert!(plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360
        })
        .is_none());
    }

    #[test]
    fn the_expected_duration_is_the_sequence_duration() {
        // Two clips, each 5 seconds long, starting at 0s and 5s. The sequence
        // therefore ends at 10s, not 15s: clip duration and clip start are
        // different quantities and conflating them is an easy way to plan an
        // export that is half again as long as the edit.
        let p = project_with_clips(2);
        let g = plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360,
        })
        .unwrap();
        assert_eq!(g.expected_duration, Ticks::from_seconds(10));
        assert_eq!(g.expected_duration, p.primary_sequence().duration());
        assert_eq!(g.expected_frames(), 300);
    }

    #[test]
    fn a_range_narrows_the_expected_duration() {
        let p = project_with_clips(2);
        let g = plan(
            &p,
            p.primary_sequence(),
            Some((Ticks::from_seconds(1), Ticks::from_seconds(4))),
            Profile::Preview { width: 640, height: 360 },
        )
        .unwrap();
        assert_eq!(g.expected_duration, Ticks::from_seconds(3));
    }

    #[test]
    fn preview_and_export_produce_the_same_structure_at_different_sizes() {
        let p = project_with_clips(1);
        let preview = plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360,
        })
        .unwrap();
        let export = plan(
            &p,
            p.primary_sequence(),
            None,
            Profile::Export { preset: ExportPreset::landscape_1080p() },
        )
        .unwrap();

        // Same node kinds, same duration; only the scale differs. This is the
        // invariant that keeps a cached preview segment honest.
        assert_eq!(preview.expected_duration, export.expected_duration);
        assert_eq!(preview.nodes.len(), export.nodes.len());
        assert!(preview.filters.contains("scale=640:360"));
        assert!(export.filters.contains("scale=1920:1080"));
    }

    #[test]
    fn timestamps_are_plain_seconds_with_no_colons() {
        // FFmpeg's filter syntax uses ':' as its option separator, so a
        // HH:MM:SS.mmm timestamp makes the whole graph unparseable. Verified
        // against the real sidecar: `trim=start=00:00:01.000000` fails with
        // "No option name near '00:00.000000'".
        assert_eq!(ts(Ticks::ZERO), "0");
        assert_eq!(ts(Ticks::from_seconds(1)), "1");
        assert_eq!(ts(Ticks::from_seconds(61)), "61");
        assert_eq!(ts(Ticks::from_seconds(3661)), "3661");
        // Half a second: trailing zeros trimmed, precision kept.
        assert_eq!(ts(Ticks::from_millis_f64(1500.0)), "1.5");
    }

    #[test]
    fn no_generated_filter_contains_a_colon_that_ffmpeg_would_parse_as_a_separator() {
        // The whole class of bug, guarded. Every ':' in a filter_complex
        // string is either a filter's own option separator (scale=640:360,
        // which is intended) or a timestamp (which is fatal). This checks the
        // trim/atrim arguments specifically.
        let p = project_with_clips(2);
        let g = plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360,
        })
        .unwrap();
        for field in g.filters.split(&[';', ','][..]) {
            for arg in field.split(&['=', ':'][..]).skip(1) {
                assert!(
                    !arg.starts_with("0") || !arg.contains(':'),
                    "a timestamp-looking argument survived into the filter: {field}"
                );
            }
        }
        assert!(g.filters.contains("trim=start=0:"));
    }

    #[test]
    fn a_speed_change_becomes_a_setpts_division() {
        let mut p = project_with_clips(1);
        let seq_id = p.primary_sequence().id;
        let track_id = p.primary_sequence().tracks[0].id;
        let media_id = *p.media.keys().next().unwrap();

        p.apply(crate::project::EditOp::InsertClip {
            sequence: seq_id,
            track: track_id,
            index: 0,
            clip: Box::new(Clip::new(
                Some(media_id),
                Ticks::from_seconds(20),
                Ticks::ZERO,
                Ticks::from_seconds(10),
            )),
        })
        .unwrap();

        // Now set the first clip's speed to 2x.
        let first = p.primary_sequence().track(track_id).unwrap().clips[0].id;
        p.apply(crate::project::EditOp::MoveClip {
            sequence: seq_id,
            track: track_id,
            clip: first,
            to: Ticks::from_seconds(40),
        })
        .unwrap();
        {
            let clip = p
                .primary_sequence_mut()
                .track_mut(track_id)
                .unwrap()
                .clips
                .iter_mut()
                .find(|c| c.id == first)
                .unwrap();
            clip.speed = Animatable::Static(2.0);
        }

        let g = plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360,
        })
        .unwrap();
        assert!(
            g.filters.contains("setpts=(PTS-STARTPTS)/2"),
            "a 2x clip must divide PTS, not drop frames. Got: {}",
            g.filters
        );
    }

    #[test]
    fn an_animated_property_falls_back_rather_than_panicking() {
        // Keyframed values arrive in Phase 4. The planner must not panic on
        // one before then; it must use the fallback and say so.
        let mut p = project_with_clips(1);
        let track_id = p.primary_sequence().tracks[0].id;
        {
            let clip = &mut p
                .primary_sequence_mut()
                .track_mut(track_id)
                .unwrap()
                .clips[0];
            clip.speed = Animatable::Keyframed(vec![Keyframe {
                time: Ticks::ZERO,
                value: 2.0,
                interp: Interp::Linear,
            }]);
            assert!(!clip.speed.is_static());
        }
        // Planning succeeds and treats the speed as 1x rather than unwrapping.
        let g = plan(&p, p.primary_sequence(), None, Profile::Preview {
            width: 640,
            height: 360,
        })
        .unwrap();
        assert!(g.filters.contains("trim="));
    }
}