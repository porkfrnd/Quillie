//! Import against real files (§7.1).
//!
//! Gated behind the `media` feature.

#![cfg(feature = "media")]

use quillie_app::media::{find_missing, import_into, relink, ImportError, ProxyReason};
use quillie_core::project::Project;
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
    repo_root().join("tests/fixtures/generated").join(name)
}

#[test]
fn importing_a_real_file_populates_the_media_bin() {
    let mut p = Project::new("import", quillie_core::time::FrameRate::new(30, 1), (1920, 1080));
    let before = p.media.len();

    let imported = import_into(&mut p, &fixture("1080p30_10min.mp4")).expect("import must work");

    assert_eq!(p.media.len(), before + 1);
    assert!(p.media.contains_key(&imported.media_id));

    let media = &p.media[&imported.media_id];
    assert!(media.duration.raw() > 0, "duration must be known");
    assert!(media.path.ends_with("1080p30_10min.mp4"));
    assert_eq!(media.streams.video.as_ref().unwrap().width, 1920);
    assert!(media.fingerprint.size_bytes > 0);
}

#[test]
fn import_never_decodes_frames() {
    // Import is cheap by construction: it probes and fingerprints. If it
    // decoded a single frame, importing ten files on a mechanical disk would
    // take minutes and the whole "pay as you go" premise would be false at the
    // first screen the user sees.
    let mut p = Project::default();
    let start = std::time::Instant::now();
    import_into(&mut p, &fixture("thumb_source.png")).unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "import took {elapsed:?}, which suggests it is decoding"
    );
}

#[test]
fn a_4k_source_is_flagged_for_a_proxy_with_a_stated_reason() {
    let mut p = Project::default();
    let imported = import_into(&mut p, &fixture("4k30_10min.mp4")).unwrap();
    assert_eq!(
        imported.proxy_reason,
        Some(ProxyReason::WidthAbove(3840)),
        "4K must be proxied, and the reason must be recorded"
    );
}

#[test]
fn a_small_source_is_not_proxied() {
    let mut p = Project::default();
    let imported = import_into(&mut p, &fixture("thumb_source.png")).unwrap();
    assert_eq!(imported.proxy_reason, None, "a 640px still needs no proxy");
}

#[test]
fn importing_does_not_place_a_clip_on_the_timeline() {
    // Adding media and adding a clip are different intentions. Conflating them
    // means undo has nothing meaningful to describe.
    let mut p = Project::default();
    import_into(&mut p, &fixture("1080p30_10min.mp4")).unwrap();
    assert_eq!(p.primary_sequence().tracks.len(), 0, "import must not create tracks");
    assert_eq!(p.primary_sequence().duration().raw(), 0);
}

#[test]
fn importing_the_same_file_twice_creates_two_entries() {
    // Deduplication is not import's job: the same footage may legitimately be
    // cut into several places, and collapsing them silently is a guess.
    let mut p = Project::default();
    import_into(&mut p, &fixture("thumb_source.png")).unwrap();
    import_into(&mut p, &fixture("thumb_source.png")).unwrap();
    assert_eq!(p.media.len(), 2);
}

#[test]
fn a_missing_file_fails_before_launching_anything() {
    let mut p = Project::default();
    let r = import_into(&mut p, Path::new("/definitely/not/here.mp4"));
    assert!(matches!(r, Err(ImportError::NotFound(_))));
    assert!(p.media.is_empty(), "a failed import must leave nothing behind");
}

#[test]
fn missing_media_is_reported_rather_than_repaired() {
    let mut p = Project::default();
    let imported = import_into(&mut p, &fixture("thumb_source.png")).unwrap();

    // Nothing is missing yet.
    assert!(find_missing(&p).is_empty());

    // Point the entry at a file that is not there.
    p.media.get_mut(&imported.media_id).unwrap().path = "/gone.png".into();
    let missing = find_missing(&p);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].0, imported.media_id);
}

#[test]
fn relinking_refuses_a_file_that_does_not_match() {
    // Relinking to "some other file at that path" would silently substitute
    // different footage into a finished edit. The fingerprint check is the
    // whole point of relink.
    let mut p = Project::default();
    let a = import_into(&mut p, &fixture("thumb_source.png")).unwrap();
    let b = import_into(&mut p, &fixture("1080p30_10min.mp4")).unwrap();

    let r = relink(&mut p, a.media_id, &fixture("1080p30_10min.mp4"));
    assert!(r.is_err(), "relinking to different footage must fail");
    assert_eq!(p.media[&a.media_id].path, fixture("thumb_source.png").display().to_string());

    let _ = b;
}

#[test]
fn relinking_to_the_same_file_elsewhere_succeeds() {
    let dir = std::env::temp_dir().join(format!("quillie-relink-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let moved = dir.join("moved.png");
    std::fs::copy(fixture("thumb_source.png"), &moved).unwrap();

    let mut p = Project::default();
    let imported = import_into(&mut p, &fixture("thumb_source.png")).unwrap();

    relink(&mut p, imported.media_id, &moved).expect("same content, new location");
    assert_eq!(p.media[&imported.media_id].path, moved.display().to_string());

    let _ = std::fs::remove_dir_all(&dir);
}