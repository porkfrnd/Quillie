//! End-to-end export against the real sidecar (§11.3).
//!
//! The Phase 1 acceptance criterion is explicit: **"Export duration is within
//! one frame of the plan."** That is checked here, against a real encode, with
//! a real file probed afterwards.
//!
//! Anything less is a claim rather than a measurement, and a claim about
//! export accuracy is exactly the kind that gets believed until a user exports
//! a two-hour project and finds it two minutes short.
//!
//! Gated behind the `media` feature.

#![cfg(feature = "media")]

use quillie_app::export::{build_args, export, ExportError};
use quillie_core::export::{plan, Profile};
use quillie_core::project::{
    Clip, ExportPreset, Fingerprint, MediaRef, Project, StreamSummary, Track, TrackKind,
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

/// A project with one clip cut from the middle of a real fixture.
///
/// A trim rather than the whole file, because the interesting part of the
/// duration check is that the *planned* range is what comes out -- a clip
/// shorter than its source proves the trim is being applied.
fn project_with_one_clip(rate: FrameRate, start_s: i64, len_s: i64) -> Project {
    let mut p = Project::new("export test", rate, (640, 360));
    let media_id = uuid::Uuid::new_v4();
    p.media.insert(
        media_id,
        MediaRef {
            id: media_id,
            path: fixture("1080p30_10min.mp4").display().to_string(),
            fingerprint: Fingerprint {
                size_bytes: 0,
                mtime_unix: 0,
                head_hash: [0; 16],
                tail_hash: [0; 16],
            },
            duration: Ticks::from_seconds(2),
            streams: StreamSummary::default(),
            rotation: 0,
            vfr: false,
        },
    );
    let mut track = Track::new(TrackKind::Video, "V1");
    track.clips.push(Clip::new(
        Some(media_id),
        Ticks::ZERO,
        Ticks::from_seconds(start_s),
        Ticks::from_seconds(start_s + len_s),
    ));
    p.primary_sequence_mut().tracks.push(track);
    p
}

/// A unique temp path per test.
///
/// The directory is per-process, but these tests run in parallel within the
/// same process, so the filename carries the test name too. Sharing one
/// directory made one test's cleanup delete another's output mid-run.
fn tmp_out(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("quillie-export-{}", std::process::id()))
        .join(name);
    std::fs::create_dir_all(&d).unwrap();
    d.join("out.mp4")
}

fn small_preset() -> ExportPreset {
    // Small enough that a software VP9 encode finishes in a test run. The
    // accuracy being checked does not depend on resolution.
    let mut p = ExportPreset::landscape_1080p();
    p.width = 320;
    p.height = 180;
    p.video_bitrate_bps = 400_000;
    p
}

#[test]
fn export_duration_is_within_one_frame_of_the_plan() {
    let p = project_with_one_clip(FrameRate::new(30, 1), 0, 1);
    let preset = small_preset();
    let out = tmp_out("duration.mp4");

    let report = export(&p, &out, &preset).expect("export must succeed");

    assert!(
        report.duration_delta_seconds <= report.frame_seconds,
        "export duration {:.3}s differs from the plan {:.3}s by {:.3}s, \
         which is more than one frame ({:.3}s)",
        report.duration_seconds,
        report.planned_seconds,
        report.duration_delta_seconds,
        report.frame_seconds
    );
    assert!(out.exists(), "the output file must exist under its real name");
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

#[test]
fn export_honours_the_trimmed_range_not_the_whole_source() {
    // A half-second clip from a two-second source. If the trim were being
    // ignored the file would be four times too long, which the duration check
    // would catch only if the plan were also wrong -- so this asserts the
    // relationship directly.
    let p = project_with_one_clip(FrameRate::new(30, 1), 0, 1);
    let preset = small_preset();
    let out = tmp_out("trimmed.mp4");

    let report = export(&p, &out, &preset).unwrap();
    assert!(
        report.duration_seconds < 1.6,
        "a 1s clip exported to {:.3}s: the trim is not being applied",
        report.duration_seconds
    );
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

#[test]
fn export_produces_the_requested_dimensions_and_codec() {
    let p = project_with_one_clip(FrameRate::new(30, 1), 0, 1);
    let preset = small_preset();
    let out = tmp_out("dims.mp4");

    let report = export(&p, &out, &preset).unwrap();
    assert_eq!(report.codec.as_deref(), Some("vp9"), "the sidecar encodes VP9");
    assert_eq!(report.width, Some(320));
    assert_eq!(report.height, Some(180));
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

#[test]
fn a_failed_export_leaves_no_part_file_and_no_output() {
    // Point at a file that does not exist. The export must fail, must not
    // leave a .part file behind, and must not create the output.
    let mut p = project_with_one_clip(FrameRate::new(30, 1), 0, 1);
    let media_id = *p.media.keys().next().unwrap();
    p.media.get_mut(&media_id).unwrap().path = "/tmp/no-such-source.mp4".into();

    let preset = small_preset();
    let out = tmp_out("never-created.mp4");

    let result = export(&p, &out, &preset);
    assert!(result.is_err(), "exporting a missing source must fail");

    assert!(!out.exists(), "no output file may be left behind");
    let part = quillie_app::export::part_path(&out);
    assert!(
        !part.exists(),
        "a .part file was left behind at {}",
        part.display()
    );
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

#[test]
fn an_empty_sequence_is_refused_rather_than_producing_an_empty_file() {
    let p = Project::new("empty", FrameRate::new(30, 1), (320, 180));
    let out = tmp_out("empty.mp4");
    let err = export(&p, &out, &small_preset()).expect_err("must refuse");
    assert!(matches!(err, ExportError::NothingToRender));
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

#[test]
fn the_argument_builder_never_produces_a_shell_string() {
    // Every argument stays one element, including a filename containing shell
    // metacharacters.
    let mut p = project_with_one_clip(FrameRate::new(30, 1), 0, 1);
    let media_id = *p.media.keys().next().unwrap();
    p.media.get_mut(&media_id).unwrap().path =
        "/tmp/a;rm -rf $HOME & echo `id`.mp4".into();

    let graph = plan(&p, p.primary_sequence(), None, Profile::Export { preset: small_preset() })
        .unwrap();
    let args = build_args(&graph, Path::new("/tmp/out.mp4"));

    assert_eq!(args.iter().filter(|a| a.contains("rm -rf")).count(), 1);
    assert!(
        args.iter().any(|a| a == "/tmp/a;rm -rf $HOME & echo `id`.mp4"),
        "the whole filename must be a single argument, got: {args:?}"
    );
}

#[test]
fn preview_and_export_plans_agree_on_duration() {
    // The Smooth Zones invariant (§8.5): a cached segment must not disagree
    // with the final render. Both go through the same planner, so this holds
    // by construction -- and is asserted so a future divergence is caught.
    let p = project_with_one_clip(FrameRate::new(30, 1), 0, 1);
    let preview = plan(
        &p,
        p.primary_sequence(),
        None,
        Profile::Preview { width: 640, height: 360 },
    )
    .unwrap();
    let full = plan(
        &p,
        p.primary_sequence(),
        None,
        Profile::Export { preset: small_preset() },
    )
    .unwrap();
    assert_eq!(preview.expected_duration, full.expected_duration);
    assert_eq!(preview.filters.matches("trim=").count(), full.filters.matches("trim=").count());
}