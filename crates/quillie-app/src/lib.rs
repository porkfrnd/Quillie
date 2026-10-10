//! Quillie host crate (`quillie-app`).
//!
//! Everything that touches the outside world lives here: the filesystem,
//! FFmpeg and ffprobe processes, the audio device, and the Tauri IPC surface.
//! `quillie-core` next door stays pure so the model can be tested exhaustively
//! and fast.
//!
//! The Tauri shell is behind the `tauri-host` feature so that filesystem and
//! process code still compiles and tests on a machine with no webview
//! development packages.
//!
//! Rules this crate exists to enforce (§5):
//!   - Rendering never persists, and persistence never renders.
//!   - Background work does not depend on the UI: the scheduler lives here in
//!     Rust, so closing the window does not kill a proxy build.
//!   - A Tauri command deserializes, calls a core or service function, and
//!     returns. A command much over 30 lines is a design smell.

pub mod audio;
pub mod capability_probe;
pub mod export;
pub mod io;
pub mod process;

#[cfg(feature = "tauri-host")]
pub mod commands;

#[cfg(feature = "tauri-host")]
pub mod protocol;

#[cfg(feature = "tauri-host")]
pub use commands::run;