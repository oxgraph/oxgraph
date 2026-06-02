//! OXGDB v1 store: durable payload bundle plus crash-atomic publication.
//!
//! The byte format lives in [`crate::freeze`]; this module owns the on-disk
//! file lifecycle (temp file, fsync, atomic rename, directory fsync).

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    CheckpointGeneration, CommitSeq, DbError, TransactionId, freeze, state::DatabaseState,
};

/// OXGDB store filename.
const STORE_FILE: &str = "store.oxgdb";

/// Temporary store filename used for atomic replacement.
const TEMP_STORE_FILE: &str = "store.oxgdb.tmp";

/// Durable database payload.
///
/// # Performance
///
/// Cloning is `O(database size)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredDatabase {
    /// Last visible commit sequence.
    pub(crate) commit_seq: CommitSeq,
    /// Last committed or empty-committed writer transaction ID.
    pub(crate) transaction_id: TransactionId,
    /// Durable checkpoint/root generation stamp.
    pub(crate) generation: CheckpointGeneration,
    /// Canonical state.
    pub(crate) state: DatabaseState,
}

impl StoredDatabase {
    /// Creates an empty stored database.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub(crate) const fn empty() -> Self {
        Self {
            commit_seq: CommitSeq::new(0),
            transaction_id: TransactionId::new(0),
            generation: CheckpointGeneration::new(0),
            state: DatabaseState::empty(),
        }
    }
}

/// Returns the OXGDB store path.
///
/// # Performance
///
/// This function is `O(path length)`.
#[must_use]
pub(crate) fn store_path(root: &Path) -> PathBuf {
    root.join(STORE_FILE)
}

/// Writes a complete OXGDB store, publishing it crash-atomically.
///
/// # Errors
///
/// Returns [`DbError`] when encoding, writing, syncing, replacing the store, or
/// syncing the published directory entry fails.
///
/// # Performance
///
/// This function is `O(database size)`.
pub(crate) fn write_store(root: &Path, stored: &StoredDatabase) -> Result<(), DbError> {
    fs::create_dir_all(root).map_err(|error| DbError::io("create database directory", error))?;
    let bytes = freeze::freeze(stored)?;
    let temp_path = root.join(TEMP_STORE_FILE);
    let mut file = File::create(&temp_path).map_err(|error| DbError::io("create store", error))?;
    file.write_all(&bytes)
        .map_err(|error| DbError::io("write store payload", error))?;
    file.flush()
        .map_err(|error| DbError::io("flush store", error))?;
    file.sync_all()
        .map_err(|error| DbError::io("sync store", error))?;
    fs::rename(temp_path, store_path(root)).map_err(|error| DbError::io("publish store", error))?;
    sync_directory(root)?;
    Ok(())
}

/// Reads and validates a complete OXGDB store.
///
/// # Errors
///
/// Returns [`DbError`] when the file is missing, malformed, or semantically
/// invalid.
///
/// # Performance
///
/// This function is `O(database size)`.
pub(crate) fn read_store(root: &Path) -> Result<StoredDatabase, DbError> {
    let mut file = File::open(store_path(root)).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => DbError::NotFound,
        _kind => DbError::io("open store", error),
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| DbError::io("read store", error))?;
    freeze::open(&bytes)
}

/// Validates a store without returning the payload.
///
/// # Errors
///
/// Returns [`DbError`] when store validation fails.
///
/// # Performance
///
/// This function is `O(database size)`.
pub(crate) fn validate_store(root: &Path) -> Result<(), DbError> {
    read_store(root).map(|_stored| ())
}

/// Syncs a directory entry publication on Unix filesystems.
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), DbError> {
    let directory =
        File::open(path).map_err(|error| DbError::io("open database directory", error))?;
    directory
        .sync_all()
        .map_err(|error| DbError::io("sync database directory", error))
}

/// Treats directory sync as unsupported on non-Unix targets.
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), DbError> {
    Ok(())
}
