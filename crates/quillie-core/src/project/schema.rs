//! Project schema versioning and migrations (§6.3).
//!
//! Every project file starts with `schema_version`. Changing the on-disk shape
//! means bumping [`SCHEMA_VERSION`] and adding a `vN -> vN+1` function here
//! with a test that loads a checked-in fixture from the old version.
//!
//! Migrations are **pure**: they take a value and return a value. Nothing here
//! touches the filesystem, because `quillie-core` does not depend on I/O.

use super::model::{Project, SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("project schema version {found} is newer than this build supports ({supported})")]
    FromTheFuture { found: u32, supported: u32 },
    #[error("project schema version is missing or not a number")]
    MissingVersion,
}

/// Bring a project at any supported version up to [`SCHEMA_VERSION`].
///
/// Runs migrations in order. A project already at the current version is
/// returned untouched, so this is cheap to call on every open.
pub fn migrate(project: Project) -> Result<Project, MigrationError> {
    let mut p = project;
    match p.schema_version {
        v if v > SCHEMA_VERSION => Err(MigrationError::FromTheFuture {
            found: v,
            supported: SCHEMA_VERSION,
        }),
        SCHEMA_VERSION => Ok(p),
        // Each arm is one step forward. Adding v1->v2 means adding a v1 arm
        // here and bumping SCHEMA_VERSION to 2.
        _v => {
            p = migrate_v1_to_v2(p)?;
            migrate(p)
        }
    }
}

/// Placeholder for the first real migration, so the shape of one is recorded.
///
/// A real migration looks like: match fields that changed, fill in defaults
/// for new ones, and keep the old value wherever the user would be unhappy to
/// lose it.
fn migrate_v1_to_v2(p: Project) -> Result<Project, MigrationError> {
    Ok(p)
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
}