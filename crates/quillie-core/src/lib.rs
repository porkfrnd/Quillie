//! Quillie core: the project model, time base, and media logic.
//!
//! This crate is deliberately pure (rule §4): it never spawns a process, never
//! touches the filesystem directly, and never depends on Tauri. Anything that
//! needs I/O lives in `quillie-app`, which calls into here.
//!
//! That boundary is what lets the model, its inverses, and its migrations be
//! tested exhaustively and fast, and it is what keeps `cargo test` working on a
//! machine with no FFmpeg installed.

// Architecture rule 2 (§5): the only way to change a project is
// `Project::apply(op) -> Result<Inverse, EditError>`. Nothing else mutates state.
pub mod project;
pub mod time;

pub use project::{EditOp, Inverse, Project};
pub use time::{FrameRate, Ticks};