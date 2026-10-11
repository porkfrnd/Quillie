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
    /// Set by any command that mutates the project, cleared by a successful
    /// save. The autosave debounce (§6.3) reads this; commands never block on it.
    pub dirty: std::sync::atomic::AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            store: std::sync::Mutex::new(None),
            project: std::sync::Mutex::new(None),
            dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }
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
    let store = ProjectStore::new(path, None);
    let project: Project = store.load().map_err(WireError::from_store)?;
    let len = project.sequences.iter().map(|s| s.tracks.len()).sum();
    *state.store.lock().expect("app state mutex poisoned") = Some(store);
    *state.project.lock().expect("app state mutex poisoned") = Some(project);
    Ok(len)
}

#[tauri::command]
pub fn save_project(state: State<'_, AppState>) -> Result<(), WireError> {
    let store_guard = state.store.lock().expect("app state mutex poisoned");
    let store = store_guard.as_ref().ok_or_else(not_loaded)?;

    let project_guard = state.project.lock().expect("app state mutex poisoned");
    let project = project_guard.as_ref().ok_or_else(not_loaded)?;

    // Atomic: temp file in the same directory, fsync, rename. A crash mid-save
    // leaves the previous version intact and a .bak behind it.
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
    let mut project_guard = state.project.lock().expect("app state mutex poisoned");
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

    // The media table changed, so the project on disk is now stale.
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
    let project_guard = state.project.lock().expect("app state mutex poisoned");
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

fn not_loaded() -> WireError {
    WireError {
        code: "not_loaded",
        message: "No project is open.".into(),
        detail: None,
        recoverable: false,
    }
}

/// Start the application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_project,
            save_project,
            import_media,
            export_video
        ])
        .run(tauri::generate_context!())
        .expect("error while running Quillie");
}