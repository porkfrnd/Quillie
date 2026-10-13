//! The Quillie project model (§6.2).
//!
//! This module is the **only** source of truth for edits (architecture rule 1).
//! The UI holds a mirror built from patches plus transient interaction state;
//! the renderer holds nothing persistent.
//!
//! Nothing mutates a [`Project`] except through
//! [`Project::apply`](edit::Project::apply), which returns an
//! [`Inverse`](edit::Inverse) restoring the previous state exactly. That is
//! what makes undo total instead of approximate.

mod edit;
mod model;
mod schema;
mod undo;

pub use edit::{EditError, EditOp, Inverse, Patch, TimeRange, TrimEdge};
pub use model::*;
pub use schema::{migrate, MigrationError, OLDEST_SUPPORTED_VERSION};
pub use undo::UndoStack;