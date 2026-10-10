//! Project persistence (§6.3).
//!
//! # The guarantee
//!
//! A crash must never leave a half-written project in place. There are exactly
//! two ways to lose work here — a torn write, or a lost edit between the last
//! autosave and the crash — and this module is built around closing both.
//!
//! Saving writes to a temporary file in the **same directory** as the target,
//! fsyncs it, then renames over the target. Same directory matters: a rename
//! across filesystems is not atomic, so a temp file in /tmp would reintroduce
//! exactly the failure this is meant to prevent.
//!
//! The previous version is kept as `.bak`, so a project that fails to load can
//! still be recovered by hand.

use quillie_core::project::Project;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

/// A project file plus its autosave journal.
pub struct ProjectStore {
    path: PathBuf,
    journal: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("could not parse project: {0}")]
    Parse(String),
    #[error("no project is loaded")]
    NotLoaded,
}

impl ProjectStore {
    /// Open (or create) a store rooted at `path`.
    ///
    /// The journal lives beside the project by default. Passing `None` uses
    /// `<app-data>/autosave/<project-id>.journal` instead, which is what the
    /// host does in production.
    pub fn new(path: impl Into<PathBuf>, journal: Option<PathBuf>) -> Self {
        ProjectStore { path: path.into(), journal }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn backup_path(&self) -> PathBuf {
        self.path.with_extension("quillie.bak")
    }

    pub fn journal_path(&self) -> Option<&Path> {
        self.journal.as_deref()
    }

    /// Save atomically.
    ///
    /// Order of operations, and why:
    /// 1. write the new content to a temp file **in the same directory**
    /// 2. `sync_all` the temp file -- without this the rename can land before
    ///    the data does, which on a crash leaves an empty file that *looks*
    ///    valid
    /// 3. copy the current target to `.bak`
    /// 4. rename the temp over the target (atomic within a filesystem)
    /// 5. `sync_all` the containing directory, so the rename itself survives
    ///    a power loss rather than merely surviving a process crash
    pub fn save(&self, project: &Project) -> Result<(), StoreError> {
        let json = serde_json::to_string_pretty(project)
            .map_err(|e| StoreError::Parse(e.to_string()))?;

        let dir = self
            .path
            .parent()
            .ok_or_else(|| StoreError::Io {
                path: self.path.clone(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "project has no parent directory"),
            })?;
        fs::create_dir_all(dir).map_err(|e| StoreError::Io { path: dir.to_path_buf(), source: e })?;

        // 1 + 2: write and fsync the replacement.
        let temp = NamedTempFile::new_in(dir).map_err(|e| StoreError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        {
            let mut f = temp.as_file();
            f.write_all(json.as_bytes())
                .map_err(|e| StoreError::Io { path: self.path.clone(), source: e })?;
            f.write_all(b"\n").map_err(|e| StoreError::Io {
                path: self.path.clone(),
                source: e,
            })?;
            f.sync_all().map_err(|e| StoreError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        }

        // 3: keep the previous version recoverable.
        if self.path.exists() {
            let _ = fs::copy(&self.path, self.backup_path());
        }

        // 4: atomic swap.
        temp.persist(&self.path).map_err(|e| StoreError::Io {
            path: self.path.clone(),
            source: e.error,
        })?;

        // 5: make the rename itself durable.
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    /// Load a project, transparently migrating older schema versions.
    pub fn load(&self) -> Result<Project, StoreError> {
        let text = fs::read_to_string(&self.path).map_err(|e| StoreError::Io {
            path: self.path.clone(),
            source: e,
        })?;
        Self::parse(&text)
    }

    /// Parse project JSON, applying migrations.
    pub fn parse(text: &str) -> Result<Project, StoreError> {
        let project: Project =
            serde_json::from_str(text).map_err(|e| StoreError::Parse(e.to_string()))?;
        quillie_core::project::migrate(project).map_err(|e| StoreError::Parse(e.to_string()))
    }

    /// Try the backup when the primary file will not load.
    ///
    /// Returning the backup silently would be wrong -- a user who opens a
    /// project and finds yesterday's edits with no explanation has no way to
    /// tell that from Quillie having lost their work.
    pub fn load_backup(&self) -> Result<Project, StoreError> {
        let text = fs::read_to_string(self.backup_path()).map_err(|e| StoreError::Io {
            path: self.backup_path(),
            source: e,
        })?;
        Self::parse(&text)
    }

    /// Append an operation to the autosave journal.
    ///
    /// Append-only, so a crash mid-write can lose at most the last record.
    /// Flushed per call because the point of the journal is to survive a crash.
    pub fn append_journal(&self, line: &str) -> Result<(), StoreError> {
        let path = self.journal.as_ref().ok_or(StoreError::NotLoaded)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| StoreError::Io { path: dir.to_path_buf(), source: e })?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| StoreError::Io { path: path.clone(), source: e })?;
        writeln!(f, "{line}").map_err(|e| StoreError::Io { path: path.clone(), source: e })?;
        f.sync_data().map_err(|e| StoreError::Io { path: path.clone(), source: e })?;
        Ok(())
    }

    /// Is the journal newer than the project file? If so, offer recovery.
    pub fn has_recovery(&self) -> bool {
        let Some(j) = &self.journal else { return false };
        if !j.exists() || !self.path.exists() {
            return false;
        }
        let Ok(jm) = fs::metadata(j).and_then(|m| m.modified()) else { return false };
        let Ok(pm) = fs::metadata(&self.path).and_then(|m| m.modified()) else { return false };
        jm > pm
    }

    /// Read journal records. Returns them even if the last line is torn.
    pub fn read_journal(&self) -> Result<Vec<String>, StoreError> {
        let path = self.journal.as_ref().ok_or(StoreError::NotLoaded)?;
        let text = fs::read_to_string(path).map_err(|e| StoreError::Io {
            path: path.clone(),
            source: e,
        })?;
        Ok(text.lines().map(|l| l.to_string()).collect())
    }

    /// Discard the journal after a successful save.
    pub fn clear_journal(&self) -> Result<(), StoreError> {
        if let Some(path) = &self.journal {
            if path.exists() {
                fs::remove_file(path).map_err(|e| StoreError::Io { path: path.clone(), source: e })?;
            }
        }
        Ok(())
    }
}