//! Quillie Tauri host (`quillie-app`).
//!
//! Architecture rules that shape this crate:
//!   - Commands are thin (§5 rule 7): deserialize, call core/service, return.
//!   - Rendering never persists and persistence never renders (rule 4).
//!   - Background work does not depend on the UI (rule 5): the scheduler lives
//!     here in Rust, not in the webview.

pub mod audio;
pub mod commands;
pub mod io;
pub mod process;
pub mod protocol;