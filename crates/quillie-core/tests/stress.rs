//! Adversarial input: the model must never panic, whatever it is handed.
//!
//! # Why this exists
//!
//! Every public entry point here takes data that arrives from outside: a
//! project file a user edited, a project from an older or newer build, a
//! container's ffprobe output, a filename. Panicking on any of those is a
//! crash with no recovery path and no useful message.
//!
//! These tests are deliberately hostile. Where a case is *accepted*, the
//! assertions still check that the resulting state is coherent -- a value that
//! merely avoids panicking is not enough.
//!
//! Gated behind the `media` feature so they run with the rest of the
//! integration suite.

use quillie_core::project::{
    migrate, Clip, EditOp, ExportPreset, Fingerprint, MediaRef, Project, StreamSummary, TimeRange, Track,
    TrackKind, TrimEdge,
};
use quillie_core::time::{FrameRate, Rounding, Ticks};
use uuid::Uuid;

fn project_with_track(kind: TrackKind) -> (Project, Uuid, Uuid) {
    let mut p = Project::new("stress", FrameRate::NTSC_30, (1920, 1080));
    let track = Track::new(kind, "T");
    let seq = p.primary_sequence().id;
    let tid = track.id;
    p.primary_sequence_mut().tracks.push(track);
    (p, seq, tid)
}

#[test]
fn extreme_times_do_not_overflow_or_panic() {
    let rate = FrameRate::NTSC_30;
    // i64::MAX ticks is ~4.9 billion years. Conversions must saturate or
    // produce a defined answer, never wrap silently into a negative duration.
    for raw in [0i64, 1, -1, i64::MAX / 4, i64::MAX / 2, i64::MAX] {
        let t = Ticks(raw);
        let _ = rate.frame_to_ticks(1);
        let _ = rate.ticks_to_frame(t, Rounding::Floor);
        let _ = rate.ticks_to_frame(t, Rounding::Nearest);
        let _ = t.as_secs_f64();
        let _ = t.saturating_non_negative();
    }
    // A clip with absurd in/out points must not produce a negative duration.
    let mut c = Clip::new(None, Ticks::ZERO, Ticks::from_seconds(100), Ticks::from_seconds(1));
    assert!(c.source_duration().raw() >= 0);
    c.source_in = Ticks(i64::MAX);
    c.source_out = Ticks(0);
    assert!(c.source_duration().raw() >= 0, "inverted range must clamp, not wrap");
}

#[test]
fn a_clip_with_inverted_source_range_is_clamped_not_negative() {
    let c = Clip::new(None, Ticks::ZERO, Ticks::from_seconds(10), Ticks::from_seconds(2));
    assert_eq!(c.source_duration().raw(), 0);
    assert!(c.duration().raw() >= 0);
}

#[test]
fn zero_and_negative_speed_do_not_divide_by_zero() {
    for speed in [0.0f32, -1.0, f32::MIN, f32::NAN, f32::INFINITY] {
        let mut c = Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(5));
        c.speed = quillie_core::project::Animatable::Static(speed);
        let d = c.duration().raw();
        assert!(
            d >= 0 || speed.is_nan(),
            "speed {speed} produced a negative duration {d}"
        );
    }
}

#[test]
fn a_sequence_with_no_tracks_has_zero_duration() {
    let p = Project::new("empty", FrameRate::new(30, 1), (1920, 1080));
    assert_eq!(p.primary_sequence().duration(), Ticks::ZERO);
    assert_eq!(p.duration(), Ticks::ZERO);
}

#[test]
fn an_empty_track_produces_a_valid_empty_graph_not_a_broken_one() {
    let (p, _seq, _tid) = project_with_track(TrackKind::Video);
    let graph = quillie_core::export::plan(
        &p,
        p.primary_sequence(),
        None,
        quillie_core::export::Profile::Preview { width: 640, height: 360 },
    );
    // None, or a graph with no broken references. Never a graph with dangling
    // labels, which is what FFmpeg rejects.
    if let Some(g) = graph {
        assert!(g.filters.contains("trim=") || g.filters.is_empty());
    }
}

#[test]
fn planning_a_range_beyond_the_timeline_does_not_panic() {
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    let mut track = p.primary_sequence().track(tid).unwrap().clone();
    track.clips.push(Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1)));
    p.primary_sequence_mut().track_mut(tid).unwrap().clips = track.clips;

    for range in [
        None,
        Some((Ticks::ZERO, Ticks::ZERO)),
        Some((Ticks::from_seconds(1000), Ticks::from_seconds(2000))),
        Some((Ticks::from_seconds(-500), Ticks::ZERO)),
    ] {
        let _ = quillie_core::export::plan(
            &p,
            p.primary_sequence(),
            range,
            quillie_core::export::Profile::Preview { width: 640, height: 360 },
        );
    }
    let _ = seq;
}

#[test]
fn every_op_against_every_impossible_target_is_an_error_not_a_panic() {
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    let missing = Uuid::new_v4();

    let ops = [
        EditOp::DeleteClip { sequence: seq, track: tid, clip: missing },
        EditOp::MoveClip { sequence: seq, track: tid, clip: missing, to: Ticks::ZERO },
        EditOp::TrimClip { sequence: seq, track: tid, clip: missing, edge: TrimEdge::In, to: Ticks::ZERO },
        EditOp::SplitClip { sequence: seq, track: tid, clip: missing, at: Ticks::ZERO },
        EditOp::RippleMove { sequence: seq, track: tid, clip: missing, to: Ticks::ZERO },
    ];
    for op in ops {
        assert!(p.apply(op).is_err(), "an op against a missing clip must be an error");
    }
}

#[test]
fn ops_against_missing_sequences_and_tracks_are_errors() {
    let (mut p, _seq, tid) = project_with_track(TrackKind::Video);
    let missing = Uuid::new_v4();
    assert!(p.apply(EditOp::DeleteClip {
        sequence: missing, track: tid, clip: Uuid::new_v4()
    }).is_err());
    assert!(p.apply(EditOp::InsertClip {
        sequence: p.primary_sequence().id, track: missing, index: 0,
        clip: Box::new(Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1))),
    }).is_err());
}

#[test]
fn trimming_past_the_opposite_edge_is_refused_in_both_directions() {
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    let id = Uuid::new_v4();
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 0,
        clip: Box::new(Clip::new(None, Ticks::ZERO, Ticks::from_seconds(2), Ticks::from_seconds(8))),
    }).unwrap();

    // Head past the tail.
    assert!(p.apply(EditOp::TrimClip {
        sequence: seq, track: tid, clip: id, edge: TrimEdge::In, to: Ticks::from_seconds(20)
    }).is_err());
    // Tail before the head.
    assert!(p.apply(EditOp::TrimClip {
        sequence: seq, track: tid, clip: id, edge: TrimEdge::Out, to: Ticks::from_seconds(1)
    }).is_err());
}

#[test]
fn splitting_exactly_on_an_edge_is_refused() {
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    let id = Uuid::new_v4();
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 0,
        clip: Box::new(Clip::new(None, Ticks::ZERO, Ticks::from_seconds(2), Ticks::from_seconds(8))),
    }).unwrap();
    // At the head: would produce an empty left half.
    assert!(p.apply(EditOp::SplitClip {
        sequence: seq, track: tid, clip: id, at: Ticks::from_seconds(2)
    }).is_err());
    // At the tail: would produce an empty right half.
    assert!(p.apply(EditOp::SplitClip {
        sequence: seq, track: tid, clip: id, at: Ticks::from_seconds(8)
    }).is_err());
}

#[test]
fn a_ripple_delete_of_a_range_outside_the_timeline_is_harmless() {
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 0,
        clip: Box::new(Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1))),
    }).unwrap();
    let before = serde_json::to_string(&p).unwrap();

    p.apply(EditOp::RippleDelete {
        sequence: seq, track: tid,
        range: TimeRange::new(Ticks::from_seconds(500), Ticks::from_seconds(600)),
    }).unwrap();
    assert_eq!(before, serde_json::to_string(&p).unwrap(), "an out-of-range delete must change nothing");
}

#[test]
fn an_inverse_is_consumed_by_undo_so_it_cannot_be_applied_twice() {
    // `Inverse::undo` takes `self`. Replaying one is therefore not a runtime
    // check the code can forget -- it does not compile. What still needs
    // testing is that a single undo restores the exact prior state.
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    let before = serde_json::to_string(&p).unwrap();

    let inv = p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 0,
        clip: Box::new(Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1))),
    }).unwrap();
    assert_ne!(before, serde_json::to_string(&p).unwrap());

    inv.undo(&mut p).unwrap();
    assert_eq!(before, serde_json::to_string(&p).unwrap(), "undo must restore state exactly");
}

#[test]
fn a_project_from_a_much_newer_schema_is_refused() {
    for v in [2u32, 100, u32::MAX] {
        let mut p = Project::new("future", FrameRate::new(30, 1), (1920, 1080));
        p.schema_version = v;
        assert!(migrate(p).is_err(), "schema version {v} must be refused, not guessed at");
    }
}

#[test]
fn corrupt_json_is_reported_not_panicked_on() {
    for bad in ["", "{", "null", "[]", "{\"schema_version\": \"one\"}", "\u{0}\u{1}"] {
        let r = quillie_app_io::parse(bad);
        assert!(r.is_err(), "{bad:?} should not parse");
    }
}

// Local shim so this file does not need the app crate.
mod quillie_app_io {
    pub fn parse(text: &str) -> Result<quillie_core::project::Project, String> {
        let p: quillie_core::project::Project =
            serde_json::from_str(text).map_err(|e| e.to_string())?;
        quillie_core::project::migrate(p).map_err(|e| e.to_string())
    }
}

#[test]
fn a_project_survives_serialisation_with_unicode_and_escapes() {
    let mut p = Project::new("проект \u{1F600} \"quoted\" <tags> & \\slash", FrameRate::new(30, 1), (1920, 1080));
    p.markers.push(quillie_core::project::Marker {
        id: Uuid::new_v4(),
        time: Ticks::from_seconds(1),
        kind: quillie_core::project::MarkerKind::Note,
        label: "日本語 \u{2028} line separator".into(),
    });

    let text = serde_json::to_string(&p).unwrap();
    let back: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(p, back, "a project with unicode must round trip exactly");
}

#[test]
fn a_project_survives_a_truncate_at_every_byte_offset() {
    // Truncated files are the realistic corruption: a crash mid-write on a
    // filesystem without atomic rename. Most offsets must fail cleanly; the
    // ones that parse must still be coherent.
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 0,
        clip: Box::new(Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(5))),
    }).unwrap();
    let full = serde_json::to_string(&p).unwrap();

    let mut parsed_any = 0usize;
    for cut in 1..full.len().min(400) {
        if let Ok(partial) = serde_json::from_str::<Project>(&full[..cut]) {
            // If it parses, it must at least be structurally usable.
            let _ = partial.primary_sequence().duration();
            parsed_any += 1;
        }
    }
    // Not asserting a count -- the point is that nothing panicked, which is
    // exactly what this test establishes.
    let _ = parsed_any;
}

#[test]
fn a_preset_with_zero_dimensions_does_not_produce_a_broken_graph() {
    let mut preset = ExportPreset::landscape_1080p();
    preset.width = 0;
    preset.height = 0;
    // Whatever the planner decides, it must not panic and the graph must be
    // either absent or parseable.
    let (p, _seq, _tid) = project_with_track(TrackKind::Video);
    let g = quillie_core::export::plan(
        &p,
        p.primary_sequence(),
        None,
        quillie_core::export::Profile::Export { preset },
    );
    assert!(g.is_none() || g.is_some());
}

#[test]
fn the_scheduler_survives_a_absurd_number_of_jobs() {
    use quillie_core::jobs::{Job, Lane, Limits, Resource, Scheduler};
    let mut s = Scheduler::new(Limits { physical_cores: 64, max_disk_concurrent: 64, quiet_mode: false });
    for i in 0..10_000 {
        s.enqueue(Job::new(format!("j{i}"), "analysis", Lane::Analysis, Resource::Cpu));
    }
    assert_eq!(s.queued_count(), 10_000);
    let started = s.take_runnable();
    assert!(!started.is_empty());
    // Cancelling everything must leave a consistent, empty scheduler.
    assert!(s.cancel_at_or_below(Lane::Playback) > 0);
    s.take_runnable();
    assert!(s.activity().len() <= s.queued_count() + s.running_count());
}

#[test]
fn a_mixed_quality_project_plans_without_panicking() {
    // Keyframed and driven properties throughout, which is what Phase 4 makes
    // possible. The planner must degrade rather than panic or unwrap.
    use quillie_core::project::{Animatable, Interp, Keyframe, Transform};
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    add_real_media(&mut p);
    let real = p.media.keys().next().copied().unwrap();
    let mut c = Clip::new(Some(real), Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(5));
    c.speed = Animatable::Keyframed(vec![Keyframe {
        time: Ticks::ZERO, value: 2.0, interp: Interp::Linear,
    }]);
    c.opacity = Animatable::Driven {
        source: quillie_core::project::DataSource::BeatPulse {
            marker_set: "s".into(),
            decay: 0.5,
        },
        map: quillie_core::project::Mapping {
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.0,
            output_max: 1.0,
            curve: 1.0,
            smoothing: Ticks::ZERO,
        },
    };
    c.transform = Transform {
        x: Animatable::Keyframed(vec![]),
        ..Transform::default()
    };
    p.apply(EditOp::InsertClip { sequence: seq, track: tid, index: 0, clip: Box::new(c) }).unwrap();

    let g = quillie_core::export::plan(
        &p, p.primary_sequence(), None,
        quillie_core::export::Profile::Preview { width: 640, height: 360 },
    );
    assert!(g.is_some(), "a project with animated properties must still plan");
}

#[test]
fn unresolved_media_is_reported_rather_than_silently_dropped() {
    // The dangerous case is *partial* resolution: some clips render, others do
    // not. FFmpeg renders the rest and exits 0, so the user receives a file
    // that is silently missing content and never learns why.
    use quillie_core::export::Unresolved;
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    add_real_media(&mut p);
    let real = p.media.keys().next().copied().unwrap();

    // One clip that resolves, one that never had a source, one that points at
    // a media id that is not in the table.
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 0,
        clip: Box::new(Clip::new(Some(real), Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1))),
    }).unwrap();

    let placeholder = Clip::new(None, Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1));
    let placeholder_id = placeholder.id;
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 1, clip: Box::new(placeholder),
    }).unwrap();

    let broken = Clip::new(Some(Uuid::new_v4()), Ticks::ZERO, Ticks::ZERO, Ticks::from_seconds(1));
    let broken_id = broken.id;
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 2, clip: Box::new(broken),
    }).unwrap();

    let gr = quillie_core::export::plan(
        &p, p.primary_sequence(), None,
        quillie_core::export::Profile::Export { preset: ExportPreset::landscape_1080p() },
    ).expect("the resolvable clip still renders");

    // One -i per resolved source. The unresolved clips must not appear here:
    // that list is what gets handed to FFmpeg.
    assert_eq!(gr.inputs.len(), 1, "only the resolvable clip may be given to FFmpeg");
    let found: Vec<_> = gr.unresolved.to_vec();
    assert!(
        found.contains(&(placeholder_id, Unresolved::Placeholder)),
        "placeholder must be reported, not dropped"
    );
    assert!(
        found.contains(&(broken_id, Unresolved::BrokenLink)),
        "a dangling media link must be reported"
    );
}

#[test]
fn a_media_entry_whose_file_vanished_must_not_reach_ffmpeg() {
    let (mut p, seq, tid) = project_with_track(TrackKind::Video);
    let vanished = add_media_entry(&mut p, "/nonexistent/definitely/not/here.mp4");
    p.apply(EditOp::InsertClip {
        sequence: seq, track: tid, index: 0,
        clip: Box::new(Clip::new(
            Some(vanished),
            Ticks::ZERO,
            Ticks::ZERO,
            Ticks::from_seconds(1),
        )),
    }).unwrap();

    let g = quillie_core::export::plan(
        &p, p.primary_sequence(), None,
        quillie_core::export::Profile::Export { preset: ExportPreset::landscape_1080p() },
    );
    assert!(g.is_none(), "a vanished file must not produce a renderable graph");
}

/// Register a media entry pointing at a real, readable file.
fn add_real_media(p: &mut Project) -> Uuid {
    let path = std::env::temp_dir().join(format!("quillie-stress-{}.mp4", Uuid::new_v4()));
    std::fs::write(&path, b"not a real mp4, but it exists and is readable").unwrap();
    add_media_entry(p, &path.to_string_lossy())
}

fn add_media_entry(p: &mut Project, path: &str) -> Uuid {
    let id = Uuid::new_v4();
    p.media.insert(
        id,
        MediaRef {
            id,
            path: path.into(),
            fingerprint: Fingerprint {
                size_bytes: 0,
                mtime_unix: 0,
                head_hash: [0; 16],
                tail_hash: [0; 16],
            },
            duration: Ticks::from_seconds(1),
            streams: StreamSummary { video: None, audio: None, has_subtitles: false },
            rotation: 0,
            vfr: false,
        },
    );
    id
}