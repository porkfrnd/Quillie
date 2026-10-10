//! Filesystem, atomic writes, and project paths (§6.3).
//!
//! Everything that touches disk lives here, so that `quillie-core` stays free
//! of I/O and remains trivially testable.

pub mod project_store;

pub use project_store::{ProjectStore, StoreError};