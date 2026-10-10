//! Job scheduling (§9.1).
//!
//! The policy lives in `quillie-core` as a pure module and is executed by
//! `quillie-app`. Nothing in here spawns a process or touches a disk, which is
//! what makes the concurrency and priority rules testable.

pub mod scheduler;

pub use scheduler::{
    Interacting, Job, JobState, Lane, Limits, Resource, Scheduler, CANCEL_CHECK_INTERVAL,
};
