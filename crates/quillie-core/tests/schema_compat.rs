//! Backwards compatibility of the on-disk project format (§6.3).
//!
//! # Why a checked-in fixture
//!
//! A round-trip test cannot catch a compatibility break, because it writes the
//! file with today's struct and then reads it back with today's struct. Both
//! sides move together and the test stays green while every project a user
//! saved yesterday stops opening.
//!
//! So this loads a **v1 file that was written before `solo` existed** —
//! checked in, not generated — and asserts what it must produce. The fixture
//! is deliberately a real project with media, a clip and a transform, not an
//! empty shell: a compatibility break usually shows up in the nested shapes
//! first.
//!
//! `docs/project-format.md` states the rule this pins: "Adding a field is
//! backwards compatible: serde's `#[serde(default)]` means an older file still
//! loads. Removing or renaming one is not, and needs a migration."

use quillie_core::project::{
    migrate, MigrationError, Project, TrackKind, SCHEMA_VERSION,
};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1-project.json")
}

fn read_fixture() -> String {
    let path = fixture();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

#[test]
fn adding_solo_did_not_bump_the_schema_version() {
    // If this fails, the additive-field argument in docs/project-format.md was
    // wrong and a migration is owed instead of a `#[serde(default)]`.
    assert_eq!(SCHEMA_VERSION, 1, "v1 is still the current format");
}

#[test]
fn a_project_saved_before_solo_existed_still_opens() {
    let project: Project = serde_json::from_str(&read_fixture())
        .expect("a v1 project file must still deserialize");

    assert_eq!(project.name, "Holiday");
    assert_eq!(project.sequences.len(), 1);
    assert_eq!(project.media.len(), 1);
    assert_eq!(project.primary_sequence().tracks.len(), 2);
}

#[test]
fn a_track_without_a_solo_key_reads_as_not_soloed() {
    let project: Project = serde_json::from_str(&read_fixture()).unwrap();

    for track in &project.primary_sequence().tracks {
        assert!(
            !track.solo,
            "a project written before solo existed means nothing is soloed"
        );
    }
}

#[test]
fn the_other_flags_in_a_v1_file_keep_the_values_they_were_saved_with() {
    // The default must apply to `solo` alone. If a default had been added
    // elsewhere, a user's saved mute would silently reset on open — the same
    // class of bug, on a field that already existed.
    let project: Project = serde_json::from_str(&read_fixture()).unwrap();
    let tracks = &project.primary_sequence().tracks;

    let video = tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .expect("the fixture has a video track");
    assert!(!video.muted);
    assert!(!video.locked);
    assert!(!video.hidden);

    let audio = tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio)
        .expect("the fixture has an audio track");
    assert!(audio.muted, "a saved mute must survive the round trip");
}

#[test]
fn a_v1_file_round_trips_and_gains_the_new_field_on_save() {
    let original: Project = serde_json::from_str(&read_fixture()).unwrap();
    let resaved = serde_json::to_string(&original).unwrap();

    // Saving is where the new field appears: the old file has no `solo`, the
    // new one does, and the value is unchanged.
    assert!(!read_fixture().contains("\"solo\""));
    assert!(resaved.contains("\"solo\":false"));

    let reread: Project = serde_json::from_str(&resaved).unwrap();
    assert_eq!(reread, original, "a save/reload must not change anything");
}

#[test]
fn migration_leaves_a_v1_project_exactly_as_it_was() {
    let project: Project = serde_json::from_str(&read_fixture()).unwrap();
    let before = serde_json::to_string(&project).unwrap();

    let migrated = migrate(project).expect("v1 is the current version");

    assert_eq!(
        before,
        serde_json::to_string(&migrated).unwrap(),
        "migrating a current-version project must be a no-op"
    );
}

#[test]
fn a_version_below_the_oldest_readable_one_is_an_error_not_a_crash() {
    // Reachable by hand-editing or by a truncated write that happened to parse.
    // It used to recurse until the stack overflowed, on the open path.
    let mut text = read_fixture();
    text = text.replacen("\"schema_version\": 1", "\"schema_version\": 0", 1);
    let mut project: Project = serde_json::from_str(&text).unwrap();

    let err = migrate(std::mem::take(&mut project)).unwrap_err();
    assert!(
        matches!(err, MigrationError::Unsupported { found: 0, .. }),
        "got {err:?}"
    );
}

#[test]
fn a_version_from_the_future_is_an_error_not_a_default() {
    let text = read_fixture().replacen("\"schema_version\": 1", "\"schema_version\": 99", 1);
    let project: Project = serde_json::from_str(&text).unwrap();

    let err = migrate(project).unwrap_err();
    assert!(matches!(err, MigrationError::FromTheFuture { found: 99, .. }), "got {err:?}");
}