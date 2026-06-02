//! Single-writer cross-process lock for an OXGDB database directory.
//!
//! The lock is an exclusively-created sentinel file in the database directory.
//! At most one writer holds it at a time; readers never touch it (an open
//! reader keeps a consistent view across a writer's atomic store rename). The
//! guard removes the sentinel on drop, so a committed or rolled-back writer
//! releases the lock automatically.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::DbError;

/// Filename of the single-writer lock sentinel.
const LOCK_FILE: &str = "store.lock";

/// RAII guard holding the single-writer lock; the sentinel is removed on drop.
///
/// # Performance
///
/// Acquisition and release are `O(1)` filesystem operations.
pub(crate) struct WriterLock {
    /// Path of the held lock sentinel.
    path: PathBuf,
}

impl WriterLock {
    /// Acquires the single-writer lock for the database rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::WriterLockHeld`] when another writer already holds the
    /// lock, or [`DbError::Io`] when the sentinel cannot be created.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) fn acquire(root: &Path) -> Result<Self, DbError> {
        let path = root.join(LOCK_FILE);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_file) => Ok(Self { path }),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(DbError::WriterLockHeld)
            }
            Err(error) => Err(DbError::io("acquire writer lock", error)),
        }
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _result = fs::remove_file(&self.path);
    }
}
