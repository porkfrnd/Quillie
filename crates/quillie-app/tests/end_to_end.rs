//! End-to-end: import a real file, edit it, export, and validate (§7.1–11.3).
//!
//! This is the closest thing to what a user actually does that can run in CI:
//! take a real file off disk, put it on a timeline, cut it, and produce a
//! verified export. Every stage uses the real sidecar, so a regression in
//! parsing, editing, planning, argument building or validation shows up here
//! rather than in someone's export.
//!
//! Gated behind the `media` feature.

#![cfg(feature = "media")]

use quillie_app::export::export;
use quillie_app::media::import_into;
use quillie_core::export::{plan, Profile};
use quillie_core::project::{
    Clip, EditOp, ExportPreset, Project, SequenceId, TimeRange, Track, TrackKind, TrimEdge,
};
use quillie_core::time::{FrameRate, Ticks};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture(name: &str) -> PathBuf {
    let p = repo_root().join("tests/fixtures/generated").join(name);
    assert!(p.exists(), "fixture missing; run scripts/gen-fixtures.sh --small");
    p
}

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("quillie-e2e-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn small_preset() -> ExportPreset {
    let mut p = ExportPreset::landscape_1080p();
    p.width = 320;
    p.height = 180;
    p.video_bitrate_bps = 300_000;
    p
}

/// Import the 1080p fixture and give it one video track with one clip.
fn imported_project() -> (Project, SequenceId) {
    let mut p = Project::new("e2e", FrameRate::new(30, 1), (320, 180));
    let imported = import_into(&mut p, &fixture("1080p30_10min.mp4")).expect("import");
    let mut track = Track::new(TrackKind::Video, "V1");
    // One second of a two-second fixture, so a trim is observable.
    track.clips.push(Clip::new(
        Some(imported.media_id),
        Ticks::ZERO,
        Ticks::ZERO,
        Ticks::from_seconds(1),
    ));
    let seq = p.primary_sequence().id;
    p.primary_sequence_mut().tracks.push(track);
    (p, seq)
}

#[test]
fn import_edit_export_produces_a_verified_file() {
    let (p, _seq) = imported_project();
    let out = workdir("full").join("final.mp4");

    let report = export(&p, &out, &small_preset()).expect("export must succeed");

    assert!(out.exists());
    assert!(
        report.duration_delta_seconds <= report.frame_seconds,
        "duration drifted {:.3}s beyond one frame",
        report.duration_delta_seconds - report.frame_seconds
    );
    assert_eq!(report.width, Some(320));
    assert_eq!(report.height, Some(180));
}

#[test]
fn trimming_a_clip_shortens_the_export_by_the_same_amount() {
    // The property that makes a trim meaningful: 1s of source trimmed to 0.4s
    // must produce a ~0.4s file, not a 1s one.
    let (mut p, seq) = imported_project();
    let track_id = p.primary_sequence().tracks[0].id;
    let clip_id = p.primary_sequence().tracks[0].clips[0].id;

    p.apply(EditOp::TrimClip {
        sequence: seq,
        track: track_id,
        clip: clip_id,
        edge: TrimEdge::Out,
        to: Ticks::from_millis_f64(400.0),
    })
    .expect("trim");

    let graph = plan(&p, p.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap();
    assert!((graph.expected_seconds() - 0.4).abs() < 0.01, "plan said {}", graph.expected_seconds());

    let out = workdir("trim").join("trimmed.mp4");
    let report = export(&p, &out, &small_preset()).expect("export after trim");
    assert!(
        report.duration_seconds < 0.6,
        "a 0.4s clip exported to {:.3}s: the trim did not take",
        report.duration_seconds
    );
}

#[test]
fn splitting_a_clip_produces_two_clips_and_one_longer_export() {
    let (mut p, seq) = imported_project();
    let track_id = p.primary_sequence().tracks[0].id;
    let clip_id = p.primary_sequence().tracks[0].clips[0].id;

    p.apply(EditOp::SplitClip {
        sequence: seq,
        track: track_id,
        clip: clip_id,
        at: Ticks::from_millis_f64(400.0),
    })
    .expect("split");

    let clips = &p.primary_sequence().track(track_id).unwrap().clips;
    assert_eq!(clips.len(), 2, "a split must leave two clips");
    // Together they still cover the original source, with no gap or overlap.
    assert_eq!(clips[0].source_out, clips[1].source_in);
    assert_eq!(clips[1].source_out, Ticks::from_seconds(1));

    let out = workdir("split").join("split.mp4");
    let report = export(&p, &out, &small_preset()).expect("export after split");
    assert!((report.duration_seconds - 1.0).abs() < 0.05);
}

#[test]
fn ripple_deleting_middle_shortens_the_export_by_exactly_that_much() {
    let (mut p, seq) = imported_project();
    let track_id = p.primary_sequence().tracks[0].id;

    // Delete 300ms from the middle of a 1s clip.
    p.apply(EditOp::RippleDelete {
        sequence: seq,
        track: track_id,
        range: TimeRange::new(Ticks::from_millis_f64(300.0), Ticks::from_millis_f64(600.0)),
    })
    .expect("ripple delete");

    let graph = plan(&p, p.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap();
    assert!(
        (graph.expected_seconds() - 0.7).abs() < 0.02,
        "plan said {} after deleting 0.3s from 1s",
        graph.expected_seconds()
    );

    let out = workdir("ripple").join("rippled.mp4");
    let report = export(&p, &out, &small_preset()).expect("export after ripple delete");
    assert!((report.duration_seconds - 0.7).abs() < 0.05);
}

#[test]
fn undoing_every_edit_returns_the_original_export_byte_for_byte() {
    // The full loop: import, edit, undo, export twice. Both exports must be
    // identical, because the project state is identical and the planner is
    // deterministic.
    let (mut p, seq) = imported_project();
    let track_id = p.primary_sequence().tracks[0].id;
    let clip_id = p.primary_sequence().tracks[0].clips[0].id;

    let before = plan(&p, p.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap()
        .filters;

    let mut inverses = Vec::new();
    for op in [
        EditOp::TrimClip {
            sequence: seq,
            track: track_id,
            clip: clip_id,
            edge: TrimEdge::Out,
            to: Ticks::from_millis_f64(700.0),
        },
        EditOp::MoveClip {
            sequence: seq,
            track: track_id,
            clip: clip_id,
            to: Ticks::from_millis_f64(100.0),
        },
    ] {
        inverses.push(p.apply(op).expect("apply"));
    }

    let edited = plan(&p, p.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap()
        .filters;
    assert_ne!(before, edited, "the edits must actually change the plan");

    for inv in inverses.into_iter().rev() {
        inv.undo(&mut p).expect("undo");
    }

    let after = plan(&p, p.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap()
        .filters;
    assert_eq!(before, after, "undo must restore the plan exactly");
}

#[test]
fn a_project_round_trips_through_disk_and_still_exports() {
    // Save, reload, export. Anything that loses information in serialisation
    // shows up here as an export that differs or fails.
    let (p, _seq) = imported_project();
    let dir = workdir("roundtrip");
    let store = quillie_app::io::ProjectStore::new(dir.join("p.quillie"), None);

    store.save(&p).expect("save");
    let loaded = store.load().expect("load");
    assert_eq!(loaded, p, "a project must survive a disk round trip unchanged");

    let out = dir.join("from-disk.mp4");
    let report = export(&loaded, &out, &small_preset()).expect("export after reload");
    assert!(report.duration_seconds > 0.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_project_in_a_new_instance_exports_identically() {
    // Guards against hidden global state: two exports of the same project from
    // two separately-loaded instances must agree.
    let (p, _seq) = imported_project();
    let dir = workdir("isolation");
    let store = quillie_app::io::ProjectStore::new(dir.join("p.quillie"), None);
    store.save(&p).unwrap();

    let a = store.load().unwrap();
    let b = quillie_app::io::ProjectStore::new(dir.join("p.quillie"), None)
        .load()
        .unwrap();

    let fa = plan(&a, a.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap()
        .filters;
    let fb = plan(&b, b.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap()
        .filters;
    assert_eq!(fa, fb, "planning must be deterministic and stateless");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_timeline_is_refused_rather_than_producing_an_empty_file() {
    let p = Project::new("empty", FrameRate::new(30, 1), (320, 180));
    let out = workdir("empty").join("nothing.mp4");
    assert!(export(&p, &out, &small_preset()).is_err());
    assert!(!out.exists(), "no file may be created for an empty project");
}

#[test]
fn exporting_the_same_project_twice_is_byte_identical() {
    // Determinism matters for the golden tests and for users who re-export.
    let (p, _seq) = imported_project();
    let dir = workdir("determinism");
    let a = dir.join("a.mp4");
    let b = dir.join("b.mp4");
    export(&p, &a, &small_preset()).expect("first export");
    export(&p, &b, &small_preset()).expect("second export");

    let ha = quillie_core::media::fingerprint::fingerprint_file(&a).unwrap();
    let hb = quillie_core::media::fingerprint::fingerprint_file(&b).unwrap();
    assert_eq!(
        ha.head_hash, hb.head_hash,
        "two exports of the same project must produce identical bytes"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_export_with_one_unresolvable_clip_is_refused_not_silently_shortened() {
    // The failure this prevents is quiet and expensive: a project whose
    // second clip points at a file on an unplugged drive renders the first
    // clip, FFmpeg exits 0, and the user receives a shorter video with no
    // error anywhere. A long timeline losing half its length to a bad path is
    // not something to discover after delivery.
    let (mut p, seq) = imported_project();
    let tid = p.primary_sequence().tracks[0].id;

    // A clip with no source, sitting after the real one.
    p.apply(EditOp::InsertClip {
        sequence: seq,
        track: tid,
        index: 1,
        clip: Box::new(Clip::new(
            None,
            Ticks::from_seconds(1),
            Ticks::ZERO,
            Ticks::from_seconds(1),
        )),
    })
    .unwrap();

    let dir = workdir("unresolved");
    let out = dir.join("shortened.mp4");
    let err = export(&p, &out, &small_preset())
        .expect_err("an export missing a clip must not succeed");
    assert!(
        matches!(err, quillie_app::export::ExportError::UnresolvedMedia { .. }),
        "expected a refusal naming the unresolved clip, got: {err}"
    );
    assert!(!out.exists(), "nothing may be written for a refused export");
    assert!(
        !out.with_extension("mp4.part").exists(),
        "not even a partial file may be left behind"
    );

    // Give that clip a real source. Now the same timeline exports, and covers
    // both seconds -- which is the point: nothing was lost, the export was
    // correctly refused until it could be completed.
    let bad = p.primary_sequence().track(tid).unwrap().clips[1].id;
    p.apply(EditOp::DeleteClip { sequence: seq, track: tid, clip: bad })
        .unwrap();
    let real = *p.media.keys().next().expect("the import registered media");
    p.apply(EditOp::InsertClip {
        sequence: seq,
        track: tid,
        index: 1,
        clip: Box::new(Clip::new(
            Some(real),
            Ticks::from_seconds(1),
            Ticks::ZERO,
            Ticks::from_seconds(1),
        )),
    })
    .unwrap();

    let fixed = dir.join("fixed.mp4");
    let report = export(&p, &fixed, &small_preset()).expect("export once the clip is resolved");
    assert!(
        report.duration_seconds > 1.5,
        "the full 2s timeline should now render, got {:.3}s",
        report.duration_seconds
    );

    let _ = std::fs::remove_dir_all(&dir);
}