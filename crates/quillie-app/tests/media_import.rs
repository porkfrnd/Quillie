//! ffprobe import against **real** sidecar output.
//!
//! The unit tests in `media::probe` parse captured JSON. This suite runs the
//! actual ffprobe binary against actual generated media and checks that the
//! parsed values match the file that exists on disk.
//!
//! Parsed-by-handwritten-JSON is not the same as parsed-from-the-thing-that-
//! ships. ffprobe's exact field shapes, its string-typed numbers, its rotation
//! handling and its VFR reporting only show up here.
//!
//! Gated behind the `media` feature so `cargo test` still works on a machine
//! with no sidecar and no FFmpeg:
//!
//! ```sh
//! cargo test -p quillie-core --features media
//! ```

#![cfg(feature = "media")]

use quillie_app::process::SidecarCommand;
use quillie_core::media::probe::{parse_probe_output, probe_duration, rotation};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Workspace root, from this crate's manifest dir.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn sidecar(name: &str) -> PathBuf {
    // The sidecar carries a target-triple suffix; find it rather than
    // hard-coding, so the same test works on Linux and Windows.
    let dir = repo_root().join("crates/quillie-app/sidecars");
    let wanted = format!("{name}-");
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
        panic!(
            "no sidecars in {}. Build them with scripts/build-sidecar.sh first. ({e})",
            dir.display()
        )
    });
    for e in entries.flatten() {
        let p = e.path();
        let fname = p.file_name().unwrap().to_string_lossy().to_string();
        if fname.starts_with(&wanted) {
            return p;
        }
    }
    panic!("{name} sidecar not found in {}", dir.display());
}

fn fixture(name: &str) -> PathBuf {
    let p = repo_root().join("tests/fixtures/generated").join(name);
    assert!(
        p.exists(),
        "fixture {} missing. Generate with: scripts/gen-fixtures.sh tests/fixtures/generated --small",
        p.display()
    );
    p
}

/// Probe a file and return raw JSON.
fn probe(path: &Path) -> String {
    let cmd = SidecarCommand::new(
        sidecar("ffprobe").display().to_string(),
        vec![
            "-v".into(),
            "error".into(),
            "-print_format".into(),
            "json".into(),
            "-show_format".into(),
            "-show_streams".into(),
            path.display().to_string(),
        ],
    );
    let out = cmd.run_captured().expect("ffprobe failed to run");
    assert!(
        out.success,
        "ffprobe exited {:?}: {}",
        out.status,
        out.stderr_tail
    );
    out.stdout
}

fn have_ffmpeg_with_libx264() -> bool {
    // Tests that need an H.264 *encoder* need the system ffmpeg, not our
    // sidecar (which deliberately has none -- ADR-006).
    Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("libx264"))
        .unwrap_or(false)
}

#[test]
fn probes_a_real_1080p_fixture() {
    let json = probe(&fixture("1080p30_10min.mp4"));
    let s = parse_probe_output(&json).expect("a real mp4 must parse");
    let v = s.video.expect("the fixture has video");
    assert_eq!(v.codec, "vp9", "the sidecar encodes VP9 (ADR-006)");
    assert_eq!((v.width, v.height), (1920, 1080));
    assert_eq!(v.avg_frame_rate, Some((30, 1)), "30fps constant source");
}

#[test]
fn probes_a_real_4k_fixture() {
    let json = probe(&fixture("4k30_10min.mp4"));
    let s = parse_probe_output(&json).unwrap();
    let v = s.video.unwrap();
    assert_eq!((v.width, v.height), (3840, 2160));
    // 4K triggers the proxy policy threshold (>1280 wide) in §7.2.
    assert!(v.width > 1280);
}

#[test]
fn probes_a_real_60fps_fixture() {
    let json = probe(&fixture("1080p60_5min.mp4"));
    let s = parse_probe_output(&json).unwrap();
    let v = s.video.unwrap();
    assert_eq!(v.avg_frame_rate, Some((60, 1)));
}

#[test]
fn probes_a_real_image_fixture() {
    let json = probe(&fixture("thumb_source.png"));
    let s = parse_probe_output(&json).expect("a PNG must parse as video");
    assert_eq!(s.video.unwrap().width, 640);
    // An image has no audio; the summary says so rather than inventing one.
    assert!(s.audio.is_none());
}

#[test]
fn duration_of_a_real_fixture_is_positive() {
    let json = probe(&fixture("1080p30_10min.mp4"));
    let d = probe_duration(&json).expect("duration must be reported");
    assert!(d > 0.0, "got {d}");
}

#[test]
fn a_file_with_no_video_is_rejected_with_a_typed_error() {
    // The tone fixture is audio-only. Importing it must fail cleanly rather
    // than producing a MediaRef with no video.
    let json = probe(&fixture("tone_1khz.wav"));
    assert!(parse_probe_output(&json).is_err());
}

#[test]
fn a_nonexistent_file_fails_at_the_process_boundary() {
    // Probing a missing file must fail at ffprobe, with its message preserved
    // as the error detail, rather than producing an empty-but-valid summary
    // that would import as a zero-length clip.
    let out = SidecarCommand::new(
        sidecar("ffprobe").display().to_string(),
        vec![
            "-v".into(), "error".into(),
            "-print_format".into(), "json".into(),
            "-show_streams".into(),
            "/tmp/definitely-not-a-real-file.mp4".into(),
        ],
    )
    .run_captured()
    .expect("ffprobe failed to run");

    assert!(!out.success, "ffprobe should fail on a missing file");
    assert!(
        !out.stderr_tail.is_empty(),
        "the failure reason must reach the error detail, not be swallowed"
    );
}

#[test]
fn h264_decode_is_available_even_though_h264_encode_is_not() {
    // The most important property of the current sidecar: it cannot *encode*
    // H.264 (ADR-006 is open) but it must be able to *decode* it, because that
    // is what import and preview need.
    if !have_ffmpeg_with_libx264() {
        eprintln!("skipping: no system ffmpeg with libx264 to produce a test file");
        return;
    }

    let dir = std::env::temp_dir().join(format!("quillie-h264-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("h264.mp4");

    let made = Command::new("ffmpeg")
        .args([
            "-hide_banner", "-loglevel", "error", "-y",
            "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=1",
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
        ])
        .arg(&path)
        .status();
    assert!(made.map(|s| s.success()).unwrap_or(false), "could not make an h264 file");

    // Our sidecar must be able to probe it...
    let json = probe(&path);
    let s = parse_probe_output(&json).expect("h264 must be probeable");
    assert_eq!(s.video.unwrap().codec, "h264");

    // ...and decode it, which is the part that actually matters for preview.
    let out = dir.join("decoded.mp4");
    let r = SidecarCommand::new(
        sidecar("ffmpeg").display().to_string(),
        vec![
            "-hide_banner".into(), "-loglevel".into(), "error".into(), "-y".into(),
            "-i".into(), path.display().to_string(),
            "-c:v".into(), "libvpx-vp9".into(), "-b:v".into(), "200k".into(),
            out.display().to_string(),
        ],
    )
    .run_captured()
    .expect("sidecar failed to run");
    assert!(
        r.success,
        "sidecar could not decode h264: {}",
        r.stderr_tail
    );
    assert!(out.exists() && std::fs::metadata(&out).unwrap().len() > 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rotation_of_a_plain_fixture_is_zero() {
    let json = probe(&fixture("1080p30_10min.mp4"));
    assert_eq!(rotation(&json), Some(0), "no rotation tag means zero");
}