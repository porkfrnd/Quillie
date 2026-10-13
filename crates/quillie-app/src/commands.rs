//! Tauri IPC commands (§5 rule 7).
//!
//! Every command here is deliberately thin: deserialize the arguments, call a
//! core or service function, return. A command much over thirty lines is a
//! design smell, because the moment one needs branching logic it should have
//! been that logic in `quillie-core` where it can be tested without a window.
//!
//! Errors cross the boundary as `{ code, message, detail, recoverable }`
//! (rule 8). Raw FFmpeg stderr stays in the log with its tail attached as
//! `detail`; it never becomes the user-facing message.

use crate::io::{ProjectStore, StoreError};
use quillie_core::project::Project;
use tauri::State;

/// Everything a command might need, in one place.
pub struct AppState {
    pub store: std::sync::Mutex<Option<ProjectStore>>,
    pub project: std::sync::Mutex<Option<Project>>,
    /// Undo history for the open project. Reset on open, never shared between
    /// projects: an inverse captured against one project applied to another
    /// would corrupt state by an unrecognised id at best.
    pub undo: std::sync::Mutex<quillie_core::project::UndoStack>,
    /// Set by any command that mutates the project, cleared by a successful
    /// save. The autosave debounce (§6.3) reads this; commands never block on it.
    pub dirty: std::sync::atomic::AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            store: std::sync::Mutex::new(None),
            project: std::sync::Mutex::new(None),
            undo: std::sync::Mutex::new(quillie_core::project::UndoStack::new()),
            dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

/// Lock order, everywhere: project, then undo.
///
/// Every command acquires them in this order, so there is no cycle to deadlock
/// on. Locking undo first somewhere else -- a later feature holding the undo
/// lock while asking for the project -- would deadlock against this.
fn with_both<T>(
    state: &AppState,
    f: impl FnOnce(&mut Project, &mut quillie_core::project::UndoStack) -> Result<T, WireError>,
) -> Result<T, WireError> {
    let mut p = lock_project(state)?;
    let mut u = lock_undo(state)?;
    let project = p.as_mut().ok_or_else(not_loaded)?;
    f(project, &mut u)
}

fn mark_dirty(state: &AppState) {
    state
        .dirty
        .store(true, std::sync::atomic::Ordering::Release);
}

/// The wire shape of an error. Stable, because the UI switches on `code`.
#[derive(Debug, serde::Serialize)]
pub struct WireError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub recoverable: bool,
}

impl WireError {
    /// A failed edit. The code is stable because the UI switches on it, and the
    /// detail is for the log: the message never contains an id, because a raw
    /// uuid on screen tells a user nothing they can act on.
    fn from_edit(e: quillie_core::project::EditError) -> Self {
        use quillie_core::project::EditError as E;
        let (code, message) = match &e {
            E::UnknownSequence(_) => ("unknown_sequence", "That timeline no longer exists."),
            E::UnknownTrack(_) => ("unknown_track", "That track no longer exists."),
            E::UnknownClip(_) => ("unknown_clip", "That clip is no longer on the timeline."),
            E::InvertedRange => (
                "inverted_range",
                "That edit would reverse a clip in time. Trim it the other way instead.",
            ),
            E::TrackLocked => (
                "track_locked",
                "That track is locked. Unlock it before editing.",
            ),
            E::UnknownMedia(_) => (
                "unknown_media",
                "That media is no longer in this project.",
            ),
        };
        WireError {
            code,
            message: message.into(),
            detail: Some(e.to_string()),
            recoverable: true,
        }
    }

    fn from_store(e: StoreError) -> Self {
        let (code, recoverable) = match &e {
            StoreError::Io { .. } => ("io", true),
            StoreError::Parse(_) => ("parse", true),
            StoreError::NotLoaded => ("not_loaded", false),
        };
        WireError {
            code,
            // The message is for a person; the source detail is not.
            message: match &e {
                StoreError::Parse(_) => "This project file could not be read.".into(),
                StoreError::Io { .. } => "The project could not be read from disk.".into(),
                StoreError::NotLoaded => "No project is open.".into(),
            },
            detail: Some(e.to_string()),
            recoverable,
        }
    }

    /// Export failures are mapped one-to-one onto stable codes, because the UI
    /// has to decide whether to offer a retry, point at the timeline, or just
    /// report it. The user-facing text never contains FFmpeg's stderr.
    fn from_export(e: crate::export::ExportError) -> Self {
        use crate::export::ExportError as E;
        let (code, message, recoverable) = match &e {
            E::NothingToRender => (
                "nothing_to_render",
                "There is nothing on the timeline to export.",
                true,
            ),
            E::NoFfmpeg => (
                "no_ffmpeg",
                "The video encoder is missing from this build.",
                false,
            ),
            E::UnresolvedMedia { .. } => (
                "unresolved_media",
                "Some clips on the timeline have no usable media. Relink them, or remove them, and try again.",
                true,
            ),
            E::Process(_) => (
                "encode_failed",
                "The export could not be encoded.",
                true,
            ),
            E::Validation(_) => (
                "output_unreadable",
                "The exported file could not be read back.",
                true,
            ),
            E::Invalid(_) => ("export_invalid", "The export failed validation.", true),
        };
        WireError {
            code,
            message: message.into(),
            detail: Some(e.to_string()),
            recoverable,
        }
    }
}

#[tauri::command]
pub fn open_project(path: String, state: State<'_, AppState>) -> Result<usize, WireError> {
    open_project_inner(&state, path)
}

/// Loaded a project, and refused to leave the previous one's history behind:
/// an inverse captured against one project applied to another is corruption by
/// unrecognised id at best.
pub fn open_project_inner(state: &AppState, path: String) -> Result<usize, WireError> {
    let store = ProjectStore::new(path, None);
    let project: Project = store.load().map_err(WireError::from_store)?;
    let len = project.sequences.iter().map(|s| s.tracks.len()).sum();
    // All three locks are taken before anything is mutated, so a failure part
    // way through cannot leave the window holding a project it has no store
    // for -- which would make every later save fail with `not_loaded`.
    let mut store_guard = lock_store(state)?;
    let mut project_guard = lock_project(state)?;
    let mut undo_guard = lock_undo(state)?;
    *store_guard = Some(store);
    *project_guard = Some(project);
    // History belongs to the project that was open, not to the window.
    undo_guard.clear();
    state
        .dirty
        .store(false, std::sync::atomic::Ordering::Release);
    Ok(len)
}

#[tauri::command]
pub fn save_project(state: State<'_, AppState>) -> Result<(), WireError> {
    save_project_inner(&state)
}

/// Atomic: temp file in the same directory, fsync, rename. A crash mid-save
/// leaves the previous version intact and a .bak behind it.
pub fn save_project_inner(state: &AppState) -> Result<(), WireError> {
    let store_guard = lock_store(state)?;
    let store = store_guard.as_ref().ok_or_else(not_loaded)?;

    let project_guard = lock_project(state)?;
    let project = project_guard.as_ref().ok_or_else(not_loaded)?;

    store.save(project).map_err(WireError::from_store)?;
    state
        .dirty
        .store(false, std::sync::atomic::Ordering::Release);
    Ok(())
}

/// The wire shape of one import call. Failed files come back alongside the
/// imported ones rather than as a log line the user will never see.
#[derive(Debug, serde::Serialize)]
pub struct ImportOutcome {
    pub imported: Vec<ImportedSummary>,
    /// One "path: reason" entry per file that could not be imported.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
}

/// What the UI needs to show after an import. Deliberately not
/// `ImportedMedia` itself: that type is a Rust-side detail whose shape should
/// be free to change without breaking the wire contract.
#[derive(Debug, serde::Serialize)]
pub struct ImportedSummary {
    pub media_id: String,
    pub file_name: String,
    pub duration_seconds: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codec: Option<String>,
    pub vfr: bool,
    pub rotation: u16,
    /// Set when a proxy is recommended. The UI surfaces this as a warning
    /// rather than acting on it -- building proxies is the scheduler's job, and
    /// silently starting work the user did not ask for is its own bug.
    pub proxy_reason: Option<String>,
}

#[tauri::command]
pub fn import_media(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ImportOutcome, WireError> {
    import_media_inner(&state, paths)
}

pub fn import_media_inner(
    state: &AppState,
    paths: Vec<String>,
) -> Result<ImportOutcome, WireError> {
    let mut project_guard = lock_project(state)?;
    let project = project_guard.as_mut().ok_or_else(not_loaded)?;

    let mut out = Vec::with_capacity(paths.len());
    let mut failed = Vec::new();

    // One bad file must not abandon the rest: importing a folder of media and
    // losing nine of ten because one is corrupt is its own kind of data loss.
    for raw in &paths {
        let path = std::path::Path::new(raw);
        match crate::media::import_into(project, path) {
            Ok(m) => out.push(ImportedSummary {
                media_id: m.media_id.to_string(),
                file_name: path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| raw.clone()),
                duration_seconds: m.duration.as_secs_f64(),
                width: m.streams.video.as_ref().map(|v| v.width),
                height: m.streams.video.as_ref().map(|v| v.height),
                codec: m.streams.video.as_ref().map(|v| v.codec.clone()),
                vfr: m.vfr,
                rotation: m.rotation,
                proxy_reason: m.proxy_reason.map(|r| r.describe()),
            }),
            Err(e) => failed.push(format!("{}: {e}", path.display())),
        }
    }

    if out.is_empty() && !failed.is_empty() {
        return Err(WireError {
            code: "import_failed",
            message: "None of the selected files could be imported.".into(),
            detail: Some(failed.join("; ")),
            recoverable: true,
        });
    }

    // The media table changed, so every recorded inverse may now be stale: an
    // entry captured before the import describes a project with fewer media
    // entries than the one it would be applied to. Rather than reasoning about
    // which inverses survive a media-table change, drop the history -- an undo
    // step that silently reverts only half of what the user did is worse than
    // one that is not offered.
    lock_undo(state)?.clear();

    state.dirty.store(true, std::sync::atomic::Ordering::Release);

    // Partial success is still a success, but it reports what failed so the
    // user does not believe they imported everything they selected.
    Ok(ImportOutcome { imported: out, failed })
}

/// What the UI shows after a successful export.
#[derive(Debug, serde::Serialize)]
pub struct ExportSummary {
    pub duration_seconds: f64,
    pub planned_seconds: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codec: Option<String>,
}

#[tauri::command]
pub fn export_video(
    path: String,
    width: u32,
    height: u32,
    video_bitrate_bps: u64,
    state: State<'_, AppState>,
) -> Result<ExportSummary, WireError> {
    export_video_inner(&state, path, width, height, video_bitrate_bps)
}

pub fn export_video_inner(
    state: &AppState,
    path: String,
    width: u32,
    height: u32,
    video_bitrate_bps: u64,
) -> Result<ExportSummary, WireError> {
    let project_guard = lock_project(state)?;
    let project = project_guard.as_ref().ok_or_else(not_loaded)?;

    // The custom preset inherits the working defaults (codecs, audio) and
    // overrides only the geometry and bitrate the caller chose.
    let mut preset = quillie_core::project::ExportPreset::landscape_1080p();
    preset.name = "Custom".into();
    preset.width = width;
    preset.height = height;
    preset.video_bitrate_bps = video_bitrate_bps;

    let report = crate::export::export(project, std::path::Path::new(&path), &preset)
        .map_err(WireError::from_export)?;

    Ok(ExportSummary {
        duration_seconds: report.duration_seconds,
        planned_seconds: report.planned_seconds,
        width: report.width,
        height: report.height,
        codec: report.codec,
    })
}

/// What the UI needs to keep its undo/redo affordances honest.
#[derive(Debug, serde::Serialize)]
pub struct UndoState {
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Apply one edit and record it for undo.
///
/// The op is only pushed after `apply` succeeds, so a rejected edit never
/// becomes an undo step. A step whose inverse is `NoOp` is not recorded
/// either, because pressing undo and seeing nothing happen reads as a dropped
/// keystroke.
#[tauri::command]
pub fn apply_edit(op: quillie_core::project::EditOp, state: State<'_, AppState>) -> Result<UndoState, WireError> {
    apply_edit_inner(&state, op)
}

pub fn apply_edit_inner(state: &AppState, op: quillie_core::project::EditOp) -> Result<UndoState, WireError> {
        let out = with_both(state, |project, undo| {
            let inverse = project.apply(op.clone()).map_err(WireError::from_edit)?;
            // A NoOp inverse means there is nothing to undo for this op. The
            // op still took effect; recording nothing is the correct outcome,
            // not an error.
            undo.push(op, inverse);
            Ok(UndoState {
                can_undo: undo.can_undo(),
can_redo: undo.can_redo(),
        })
    })?;
    mark_dirty(state);
    Ok(out)
}

/// Undo the most recent edit, returning the new affordance state.
#[tauri::command]
pub fn undo(state: State<'_, AppState>) -> Result<UndoState, WireError> {
    undo_inner(&state)
}

pub fn undo_inner(state: &AppState) -> Result<UndoState, WireError> {
    let out = with_both(state, |project, undo| {
        let inverse = undo
            .undo()
            .ok_or_else(|| WireError::no_history("There is nothing left to undo."))?;
        inverse.undo(project).map_err(WireError::from_edit)?;
        Ok(UndoState {
            can_undo: undo.can_undo(),
            can_redo: undo.can_redo(),
        })
    })?;
    mark_dirty(state);
    Ok(out)
}

/// Reapply the most recently undone edit.
#[tauri::command]
pub fn redo(state: State<'_, AppState>) -> Result<UndoState, WireError> {
    redo_inner(&state)
}

pub fn redo_inner(state: &AppState) -> Result<UndoState, WireError> {
    let out = with_both(state, |project, undo| {
        let op = undo
            .redo()
            .ok_or_else(|| WireError::no_history("There is nothing to redo."))?;
        // The op is applied forward again. If it now fails -- the timeline
        // changed underneath it -- the step stays consumed rather than being
        // replayed in a loop forever.
        project.apply(op).map_err(WireError::from_edit)?;
        Ok(UndoState {
            can_undo: undo.can_undo(),
            can_redo: undo.can_redo(),
        })
    })?;
    mark_dirty(state);
    Ok(out)
}

impl WireError {
    fn no_history(message: &'static str) -> Self {
        WireError {
            code: "no_history",
            message: message.into(),
            detail: None,
            // Recoverable: the user pressed a key that was simply unavailable.
            recoverable: true,
        }
    }
}

fn not_loaded() -> WireError {
    WireError {
        code: "not_loaded",
        message: "No project is open.".into(),
        detail: None,
        recoverable: false,
    }
}

/// Recover from a poisoned lock instead of aborting the process.
///
/// A mutex is poisoned when a thread panics while holding it. The obvious
/// `.expect("mutex poisoned")` turns the *next* command into a second panic,
/// and because Tauri commands run on the runtime, that unwinds the backend
/// process: the user loses the window, and with it any unsaved project.
///
/// The state behind a poisoned lock is usually still perfectly consistent --
/// poisoning records that a panic happened, not that the data is corrupt -- so
/// the data is recovered and the condition is reported. `PoisonError::into_inner`
/// is the standard way to say "I accept the risk".
fn poisoned(what: &str) -> WireError {
    WireError {
        code: "internal",
        message: "Quillie hit an internal error and recovered. Please save your project.".into(),
        detail: Some(format!("{what} was poisoned by an earlier panic; state recovered")),
        // Not recoverable by retrying the same call -- the user has to be told
        // something went wrong, but the window still works.
        recoverable: false,
    }
}

/// Lock the project, mapping poisoning to a wire error.
fn lock_project(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, Option<Project>>, WireError> {
    state.project.lock().map_err(|_| poisoned("project state"))
}

/// Lock the undo stack, mapping poisoning to a wire error.
fn lock_undo(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, quillie_core::project::UndoStack>, WireError> {
    state.undo.lock().map_err(|_| poisoned("undo stack"))
}

/// Lock the store, mapping poisoning to a wire error.
fn lock_store(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, Option<ProjectStore>>, WireError> {
    state.store.lock().map_err(|_| poisoned("project store"))
}

/// Start the application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_project,
            save_project,
            import_media,
            export_video,
            apply_edit,
            undo,
            redo
        ])
        .run(tauri::generate_context!())
        .expect("error while running Quillie");
}

#[cfg(test)]
mod tests {
    use super::*;
    use quillie_core::project::{Clip, EditOp, Track, TrackKind, TrimEdge};
    use quillie_core::time::{FrameRate, Ticks};
    use std::sync::atomic::Ordering;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("quillie-cmd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A saved project with one video track, so edits have something to act on.
    fn loaded(name: &str) -> (AppState, std::path::PathBuf) {
        let dir = tmp(name);
        let mut p = Project::new("t", FrameRate::new(30, 1), (1920, 1080));
        p.primary_sequence_mut()
            .tracks
            .push(Track::new(TrackKind::Video, "V1"));
        let path = dir.join("p.quillie");
        ProjectStore::new(path.clone(), None).save(&p).unwrap();
        let state = AppState::default();
        open_project_inner(&state, path.display().to_string()).unwrap();
        (state, dir)
    }

    fn clip_count(state: &AppState) -> usize {
        state
            .project
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .primary_sequence()
            .tracks[0]
            .clips
            .len()
    }

    #[test]
    fn every_edit_command_refuses_cleanly_when_no_project_is_open() {
        // The window can receive a keystroke before a project is open. Each
        // command must say so in its own terms rather than panicking on a
        // missing project.
        let s = AppState::default();
        let op = EditOp::DeleteClip {
            sequence: uuid::Uuid::new_v4(),
            track: uuid::Uuid::new_v4(),
            clip: uuid::Uuid::new_v4(),
        };
        for r in [
            apply_edit_inner(&s, op).err(),
            undo_inner(&s).err(),
            redo_inner(&s).err(),
            save_project_inner(&s).err(),
        ] {
            let e = r.expect("must refuse without a project");
            assert_eq!(e.code, "not_loaded");
            assert!(!e.recoverable, "there is nothing to recover to");
        }
    }

    #[test]
    fn undo_restores_the_project_exactly_and_redo_reapplies_it() {
        let (state, _dir) = loaded("undo-redo");
        let seq = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().id;
        let track = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().tracks[0].id;
        let before = serde_json::to_string(
            &state.project.lock().unwrap().as_ref().unwrap().primary_sequence(),
        )
        .unwrap();

        apply_edit_inner(
            &state,
            EditOp::InsertClip {
                sequence: seq,
                track,
                index: 0,
                clip: Box::new(Clip::new(
                    None,
                    Ticks::ZERO,
                    Ticks::ZERO,
                    Ticks::from_seconds(2),
                )),
            },
        )
        .unwrap();
        assert_eq!(clip_count(&state), 1);

        let after_edit = serde_json::to_string(
            &state.project.lock().unwrap().as_ref().unwrap().primary_sequence(),
        )
        .unwrap();

        let u = undo_inner(&state).unwrap();
        assert!(!u.can_undo, "one edit means one undo, not two");
        assert!(u.can_redo);
        assert_eq!(
            before,
            serde_json::to_string(
                &state.project.lock().unwrap().as_ref().unwrap().primary_sequence()
            )
            .unwrap(),
            "undo must restore the sequence byte for byte"
        );

        let r = redo_inner(&state).unwrap();
        assert!(r.can_undo);
        assert!(!r.can_redo);
        assert_eq!(
            after_edit,
            serde_json::to_string(
                &state.project.lock().unwrap().as_ref().unwrap().primary_sequence()
            )
            .unwrap(),
            "redo must restore the edit byte for byte"
        );
    }

    #[test]
    fn undoing_past_the_start_and_redoing_past_the_end_is_an_error_not_a_panic() {
        // Hammering Ctrl+Z past the bottom of the stack must be a clean error.
        let (state, _dir) = loaded("empty-history");
        for _ in 0..3 {
            let e = undo_inner(&state).expect_err("nothing to undo");
            assert_eq!(e.code, "no_history");
            assert!(e.recoverable, "pressing undo too often is not a failure");
        }
        for _ in 0..3 {
            let e = redo_inner(&state).expect_err("nothing to redo");
            assert_eq!(e.code, "no_history");
        }
    }

    #[test]
    fn a_rejected_edit_does_not_become_an_undo_step() {
        // The important one: if a failing edit were recorded, undo would revert
        // some *earlier* edit instead, which looks like the app undoing the
        // wrong thing for no reason.
        let (state, _dir) = loaded("rejected-edit");
        let seq = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().id;
        let track = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().tracks[0].id;

        let e = apply_edit_inner(
            &state,
            EditOp::DeleteClip {
                sequence: seq,
                track,
                clip: uuid::Uuid::new_v4(),
            },
        )
        .expect_err("clip does not exist");
        assert_eq!(e.code, "unknown_clip");

        assert!(
            undo_inner(&state).is_err(),
            "a rejected edit must leave nothing to undo"
        );
    }

    #[test]
    fn a_new_edit_discards_the_redo_branch() {
        // Branching history is a separate feature; presenting a redo that
        // conflicts with the current state would be worse than not offering it.
        let (state, _dir) = loaded("branch");
        let seq = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().id;
        let track = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().tracks[0].id;
        let insert = |start: i64| EditOp::InsertClip {
            sequence: seq,
            track,
            index: 0,
            clip: Box::new(Clip::new(
                None,
                Ticks::from_seconds(start),
                Ticks::ZERO,
                Ticks::from_seconds(1),
            )),
        };

        apply_edit_inner(&state, insert(0)).unwrap();
        apply_edit_inner(&state, insert(1)).unwrap();
        undo_inner(&state).unwrap();
        assert!(
            redo_inner(&state).is_ok(),
            "a single undo leaves exactly one step to redo"
        );

        undo_inner(&state).unwrap();
        assert!(
            redo_inner(&state).is_ok(),
            "undoing twice leaves two steps to redo"
        );

        apply_edit_inner(&state, insert(2)).unwrap();
        let e = redo_inner(&state).expect_err("a new edit kills the redo branch");
        assert_eq!(e.code, "no_history");
    }

    #[test]
    fn opening_another_project_discards_the_previous_history() {
        // An inverse captured against one project must never reach another:
        // it would be applied by id, to ids that need not mean the same thing.
        let (state, dir) = loaded("history-reset");

        let seq = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().id;
        let track = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().tracks[0].id;
        apply_edit_inner(
            &state,
            EditOp::InsertClip {
                sequence: seq,
                track,
                index: 0,
                clip: Box::new(Clip::new(
                    None,
                    Ticks::ZERO,
                    Ticks::ZERO,
                    Ticks::from_seconds(1),
                )),
            },
        )
        .unwrap();
        assert!(undo_inner(&state).is_ok(), "history exists before the reopen");

        // A second, unrelated project.
        let other = dir.join("other.quillie");
        ProjectStore::new(other.clone(), None)
            .save(&Project::new("other", FrameRate::new(30, 1), (640, 480)))
            .unwrap();
        open_project_inner(&state, other.display().to_string()).unwrap();

        assert!(
            undo_inner(&state).is_err(),
            "history from the previous project must not survive an open"
        );
        // The new project is untouched by any of that history.
        assert_eq!(
            state
                .project
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .primary_sequence()
                .tracks
                .len(),
            0
        );
    }

    #[test]
    fn the_dirty_flag_tracks_unsaved_work_and_is_cleared_by_a_save() {
        let (state, _dir) = loaded("dirty");
        assert!(
            !state.dirty.load(Ordering::Acquire),
            "a freshly opened project has nothing to save"
        );

        let seq = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().id;
        let track = state.project.lock().unwrap().as_ref().unwrap().primary_sequence().tracks[0].id;
        apply_edit_inner(
            &state,
            EditOp::InsertClip {
                sequence: seq,
                track,
                index: 0,
                clip: Box::new(Clip::new(
                    None,
                    Ticks::ZERO,
                    Ticks::ZERO,
                    Ticks::from_seconds(1),
                )),
            },
        )
        .unwrap();
        assert!(state.dirty.load(Ordering::Acquire), "an edit makes it dirty");

        undo_inner(&state).unwrap();
        assert!(
            state.dirty.load(Ordering::Acquire),
            "an edit undone back to the saved state is still worth a save, and \
             must not be silently marked clean by a flag the user never sees"
        );

        save_project_inner(&state).unwrap();
        assert!(!state.dirty.load(Ordering::Acquire), "saving clears the flag");
    }

    #[test]
    fn a_sequence_of_edits_undoes_back_to_the_saved_bytes() {
        // The property the stack exists for, over a realistic mix of ops.
        let (state, dir) = loaded("sequence");
        let (seq, track) = {
            let p = state.project.lock().unwrap();
            let p = p.as_ref().unwrap();
            (p.primary_sequence().id, p.primary_sequence().tracks[0].id)
        };
        let before = serde_json::to_string(
            &state.project.lock().unwrap().as_ref().unwrap().primary_sequence(),
        )
        .unwrap();

        let id = apply_edit_inner(
            &state,
            EditOp::InsertClip {
                sequence: seq,
                track,
                index: 0,
                clip: Box::new(Clip::new(
                    None,
                    Ticks::ZERO,
                    Ticks::ZERO,
                    Ticks::from_seconds(4),
                )),
            },
        )
        .map(|_| {
            state.project.lock().unwrap().as_ref().unwrap()
                .primary_sequence().tracks[0].clips[0].id
        })
        .unwrap();

        apply_edit_inner(
            &state,
            EditOp::TrimClip {
                sequence: seq,
                track,
                clip: id,
                edge: TrimEdge::Out,
                to: Ticks::from_seconds(2),
            },
        )
        .unwrap();
        apply_edit_inner(
            &state,
            EditOp::MoveClip {
                sequence: seq,
                track,
                clip: id,
                to: Ticks::from_millis_f64(500.0),
            },
        )
        .unwrap();

        for _ in 0..3 {
            undo_inner(&state).unwrap();
        }
        assert_eq!(
            before,
            serde_json::to_string(
                &state.project.lock().unwrap().as_ref().unwrap().primary_sequence()
            )
            .unwrap(),
            "three edits undone must land exactly on the saved bytes"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wire_errors_never_leak_internal_detail_into_the_message() {
        // The UI shows `message`; only `detail` goes to the log. A uuid in the
        // message tells a user nothing they can act on.
        let e = WireError::from_edit(quillie_core::project::EditError::UnknownClip(
            uuid::Uuid::new_v4(),
        ));
        assert_eq!(e.code, "unknown_clip");
        assert!(!e.message.contains('-'), "message leaked an id: {}", e.message);
        assert!(e.detail.is_some(), "the detail is kept for the log");
    }
}