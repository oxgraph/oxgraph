//! Crash-atomic file publication helpers shared by the base/superblock writers.
//!
//! The byte formats live in [`crate::freeze`] (base) and [`crate::wire`]
//! (superblock); this module owns the on-disk file lifecycle primitive — write a
//! temp file, fsync it, atomically rename it into place, then fsync the
//! directory entry — reused by [`crate::wal`] and the base checkpoint writer.

use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

use crate::DbError;

/// Writes `bytes` to `final_path` crash-atomically: write to `temp_path`, fsync
/// the file, atomically rename it over `final_path`, then fsync the containing
/// directory so the rename is durable.
///
/// # Errors
///
/// Returns [`DbError::Io`] when creating, writing, syncing, renaming, or syncing
/// the directory entry fails.
///
/// # Performance
///
/// This function is `O(bytes.len())`.
pub(crate) fn atomic_write(
    directory: &Path,
    temp_path: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<(), DbError> {
    fs::create_dir_all(directory)
        .map_err(|error| DbError::io("create database directory", error))?;
    let mut file =
        File::create(temp_path).map_err(|error| DbError::io("create temp file", error))?;
    file.write_all(bytes)
        .map_err(|error| DbError::io("write temp file", error))?;
    file.flush()
        .map_err(|error| DbError::io("flush temp file", error))?;
    file.sync_all()
        .map_err(|error| DbError::io("sync temp file", error))?;
    fs::rename(temp_path, final_path).map_err(|error| DbError::io("publish file", error))?;
    sync_directory(directory)
}

/// Syncs a directory entry publication on Unix filesystems.
///
/// # Errors
///
/// Returns [`DbError::Io`] when the directory cannot be opened or synced.
///
/// # Performance
///
/// This function is `O(1)`.
#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), DbError> {
    let directory =
        File::open(path).map_err(|error| DbError::io("open database directory", error))?;
    directory
        .sync_all()
        .map_err(|error| DbError::io("sync database directory", error))
}

/// Treats directory sync as unsupported on non-Unix targets.
///
/// # Performance
///
/// This function is `O(1)`.
#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> Result<(), DbError> {
    Ok(())
}
