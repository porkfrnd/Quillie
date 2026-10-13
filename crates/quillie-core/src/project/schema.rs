//! Project schema versioning and migrations (§6.3).
//!
//! Every project file starts with `schema_version`. Changing the on-disk shape
//! means bumping [`SCHEMA_VERSION`] and adding a `vN -> vN+1` function here
//! with a test that loads a checked-in fixture from the old version.
//!
//! Migrations are **pure**: they take a value and return a value. Nothing here
//! touches the filesystem, because `quillie-core` does not depend on I/O.

use super::model::{Project, SCHEMA_VERSION};

/// The oldest project this build can still open.
///
/// Separate from [`SCHEMA_VERSION`] on purpose. Every version at or above this
/// and at or below [`SCHEMA_VERSION`] must load; anything outside that range
/// is refused. Naming the floor explicitly is what stops the migration ladder
/// from growing an arm that silently accepts a version it cannot migrate.
pub const OLDEST_SUPPORTED_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("project schema version {found} is newer than this build supports ({supported})")]
    FromTheFuture { found: u32, supported: u32 },
    #[error("project schema version is missing or not a number")]
    MissingVersion,
    #[error("project schema version {found} predates this build; the oldest readable version is {oldest}")]
    Unsupported { found: u32, oldest: u32 },
}

/// Bring a project at any supported version up to [`SCHEMA_VERSION`].
///
/// A project already at the current version is returned untouched, so this is
/// cheap enough to call on every open.
///
/// # Why there is no loop here
///
/// The obvious implementation is a catch-all arm that runs one migration and
/// recurses. That is only correct if *every* migration sets
/// `schema_version` to the version it produced. One that forgets leaves the
/// version unchanged, the catch-all arm matches again, and a project that
/// should have loaded instead recurses until the stack overflows — on the
/// open path, for a file a user produced by hand-editing.
///
/// So the range is checked explicitly instead, and the ladder is written as a
/// straight-line `match` on the version. There is nowhere for a step to be
/// forgotten.
///
/// To add a migration: bump [`SCHEMA_VERSION`], add a `1 => { ... }` arm that
/// fills in new fields and sets `p.schema_version = 2`, and add a test loading
/// a checked-in v1 fixture. Leaving [`OLDEST_SUPPORTED_VERSION`] at 1 is
/// correct until a *released* format is dropped.
pub fn migrate(project: Project) -> Result<Project, MigrationError> {
    let v = project.schema_version;
    if v > SCHEMA_VERSION {
        return Err(MigrationError::FromTheFuture {
            found: v,
            supported: SCHEMA_VERSION,
        });
    }
    if v < OLDEST_SUPPORTED_VERSION {
        return Err(MigrationError::Unsupported {
            found: v,
            oldest: OLDEST_SUPPORTED_VERSION,
        });
    }

    match v {
        SCHEMA_VERSION => Ok(project),
        // No migration steps exist yet: v1 is both the oldest readable
        // version and the current one.
        v => Err(MigrationError::Unsupported {
            found: v,
            oldest: OLDEST_SUPPORTED_VERSION,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::FrameRate;

    #[test]
    fn a_current_project_passes_through_unchanged() {
        let p = Project::new("x", FrameRate::new(30, 1), (1920, 1080));
        let migrated = migrate(p.clone()).unwrap();
        assert_eq!(migrated, p);
    }

    #[test]
    fn a_project_from_the_future_is_refused_not_mangled() {
        let mut p = Project::new("x", FrameRate::new(30, 1), (1920, 1080));
        p.schema_version = SCHEMA_VERSION + 5;
        let err = migrate(p).unwrap_err();
        assert!(matches!(err, MigrationError::FromTheFuture { .. }));
    }

    /// This is the test the old implementation could not have passed.
    ///
    /// A v0 project matched the catch-all arm, ran a placeholder migration
    /// that did not advance `schema_version`, and matched it again. The run
    /// ended in a stack overflow, not an error. If this test ever hangs rather
    /// than failing, the version ladder has grown an arm that forgets to
    /// record the version it produced.
    #[test]
    fn a_project_older_than_the_oldest_readable_version_is_refused_not_looped() {
        let mut p = Project::new("x", FrameRate::new(30, 1), (1920, 1080));
        p.schema_version = 0;
        let err = migrate(p).unwrap_err();
        assert!(
            matches!(err, MigrationError::Unsupported { found: 0, oldest: 1 }),
            "got {err:?}"
        );
    }

    #[test]
    fn every_version_in_the_supported_range_is_accepted() {
        // The guard clauses must not be so eager that a version the ladder can
        // handle is refused, and the ladder must not accept one it cannot.
        for v in OLDEST_SUPPORTED_VERSION..=SCHEMA_VERSION {
            let mut p = Project::new("x", FrameRate::new(30, 1), (1920, 1080));
            p.schema_version = v;
            assert!(migrate(p).is_ok(), "version {v} must load");
        }
    }
}