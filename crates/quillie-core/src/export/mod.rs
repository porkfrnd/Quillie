//! Export planning and argument building (§11.2).
//!
//! One planner serves both the final export and the Smooth Zones preview cache,
//! so a cached segment cannot disagree with the file the user finally gets.

pub mod graph;

pub use graph::{plan, ExportGraph, Node, Profile, Unresolved};