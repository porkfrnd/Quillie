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
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            store: std::sync::Mutex::new(None),
            project: std::sync::Mutex::new(None),
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
    Ok(())
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
    .invoke_handler(tauri::generate_handler![open_project, save_project])
    .run(tauri::generate_context!())
    .expect("error while running Quillie");
}