//! Single-writer cross-process lock for an OXGDB database directory.
//!
//! The lock is an advisory whole-file lock taken on a persistent `store.lock`
//! file in the database directory via [`std::fs::File::try_lock`]. At most one
//! writer holds the exclusive lock at a time; readers never touch it (an open
//! reader keeps a consistent view across a writer's atomic publication because it
//! already cloned its snapshot). The kernel releases the lock automatically when
//! the holding [`File`] is dropped — including on process crash — so a crashed
//! writer never bricks the store. The `store.lock` file itself is created once
//! and never removed on drop; only the advisory lock is released.

use std::{
    fs::{self, File, TryLockError},
    path::Path,
};

use crate::DbError;

/// Filename of the single-writer advisory lock file.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
const LOCK_FILE: &str = "store.lock";

/// RAII guard holding the single-writer advisory lock; the kernel releases the
/// lock when the held [`File`] drops (including on crash). The `store.lock` file
/// is never removed, so a crash leaves it in place for the next writer to relock.
///
/// # Performance
///
/// Acquisition and release are `O(1)` filesystem operations.
pub(crate) struct WriterLock {
    /// The locked file; its [`Drop`] (below) releases the advisory lock, and the
    /// kernel also releases it if the process crashes with this handle open.
    file: File,
}

impl Drop for WriterLock {
    /// Releases the advisory lock when the guard drops. The kernel would release
    /// it on close anyway; unlocking explicitly documents the lifetime and reads
    /// the held [`File`]. Any unlock error is ignored — the close that follows
    /// releases the lock regardless, and there is no actionable recovery.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn drop(&mut self) {
        let _released = self.file.unlock();
    }
}

impl WriterLock {
    /// Acquires the single-writer advisory lock for the database rooted at
    /// `root`, creating the persistent `store.lock` file when absent.
    ///
    /// The lock is taken with [`File::try_lock`] (non-blocking), so a second
    /// writer fails fast with [`DbError::Txn(crate::error::TxnError::WriterLockHeld)`] rather than
    /// blocking.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Txn(crate::error::TxnError::WriterLockHeld)`] when another writer already
    /// holds the lock, or [`DbError::Io`] when the lock file cannot be created or locked.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) fn acquire(root: &Path) -> Result<Self, DbError> {
        fs::create_dir_all(root)
            .map_err(|error| DbError::io("create database directory", error))?;
        let path = root.join(LOCK_FILE);
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| DbError::io("open writer lock", error))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { file }),
            Err(TryLockError::WouldBlock) => {
                Err(DbError::Txn(crate::error::TxnError::WriterLockHeld))
            }
            Err(TryLockError::Error(error)) => Err(DbError::io("acquire writer lock", error)),
        }
    }
}
