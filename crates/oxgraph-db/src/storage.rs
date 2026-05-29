//! OXGDB greenfield store format.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use oxgraph_snapshot::{Snapshot, SnapshotBuilder};
use serde::{Deserialize, Serialize};

use crate::{CommitSeq, DbError, TransactionId, state::DatabaseState};

/// OXGDB store filename.
const STORE_FILE: &str = "store.oxgdb";

/// Temporary store filename used for atomic replacement.
const TEMP_STORE_FILE: &str = "store.oxgdb.tmp";

/// Section kind for the serialized database state, inside the snapshot
/// container's reserved `DATABASE_BAND` (`0x0300..0x0400`).
const SNAPSHOT_KIND_DB_STATE: u32 = 0x0300;

/// Section version for the database-state payload.
const SNAPSHOT_DB_STATE_VERSION: u32 = 1;

/// Byte alignment (`log2`) requested for the serde-JSON state payload. The
/// payload is opaque JSON, so a single-byte alignment is sufficient.
const SNAPSHOT_DB_STATE_ALIGNMENT_LOG2: u8 = 0;

/// Durable database payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct StoredDatabase {
    /// Last visible commit sequence.
    pub(crate) commit_seq: CommitSeq,
    /// Last committed or empty-committed writer transaction ID.
    pub(crate) transaction_id: TransactionId,
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

/// Writes a complete OXGDB store.
///
/// # Errors
///
/// Returns [`DbError`] when encoding, writing, syncing, replacing the store, or
/// syncing the published directory entry fails.
///
/// # Performance
///
/// This function is `O(serialized database bytes)`.
pub(crate) fn write_store(root: &Path, stored: &StoredDatabase) -> Result<(), DbError> {
    fs::create_dir_all(root).map_err(|error| DbError::io("create database directory", error))?;
    let bytes = encode_store(stored)?;
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

/// Encodes a stored database into the deterministic snapshot container bytes.
///
/// The catalog/topology/property body stays a product-level serde-JSON payload
/// because it contains strings and typed values; the snapshot container only
/// supplies the framing as a single [`SNAPSHOT_KIND_DB_STATE`] section.
///
/// # Errors
///
/// Returns [`DbError`] when JSON encoding or snapshot planning fails.
///
/// # Performance
///
/// This function is `O(serialized database bytes)`.
fn encode_store(stored: &StoredDatabase) -> Result<Vec<u8>, DbError> {
    let payload = serde_json::to_vec(stored)?;
    let mut builder = SnapshotBuilder::new();
    builder
        .add_section(
            SNAPSHOT_KIND_DB_STATE,
            SNAPSHOT_DB_STATE_VERSION,
            SNAPSHOT_DB_STATE_ALIGNMENT_LOG2,
            payload,
        )
        .map_err(|error| DbError::invalid_store(error.to_string()))?;
    builder
        .finish()
        .map_err(|error| DbError::invalid_store(error.to_string()))
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
/// This function is `O(serialized database bytes)`.
pub(crate) fn read_store(root: &Path) -> Result<StoredDatabase, DbError> {
    let mut file = File::open(store_path(root)).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => DbError::NotFound,
        _kind => DbError::io("open store", error),
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| DbError::io("read store", error))?;
    let snapshot =
        Snapshot::open(&bytes).map_err(|error| DbError::invalid_store(error.to_string()))?;
    let section = snapshot
        .section(SNAPSHOT_KIND_DB_STATE)
        .ok_or_else(|| DbError::invalid_store("store is missing the database-state section"))?;
    let stored: StoredDatabase = serde_json::from_slice(section.bytes())?;
    // The snapshot container tolerates bytes past the last section, so re-encode
    // the parsed state and require an exact match. This rejects truncation,
    // trailing bytes, and any payload corruption the deterministic container
    // layout would otherwise round through.
    if encode_store(&stored)? != bytes {
        return Err(DbError::invalid_store("store bytes are not canonical"));
    }
    stored.state.validate()?;
    Ok(stored)
}

/// Validates a store without returning the payload.
///
/// # Errors
///
/// Returns [`DbError`] when store validation fails.
///
/// # Performance
///
/// This function is `O(serialized database bytes)`.
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
