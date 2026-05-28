//! OXGDB greenfield store format.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U64},
};

use crate::{CommitSeq, DbError, TransactionId, state::DatabaseState};

/// OXGDB store filename.
const STORE_FILE: &str = "store.oxgdb";

/// Temporary store filename used for atomic replacement.
const TEMP_STORE_FILE: &str = "store.oxgdb.tmp";

/// OXGDB store magic.
const STORE_MAGIC: [u8; 8] = *b"OXGDB02\0";

/// OXGDB store version.
const STORE_VERSION: u64 = 2;

/// Fixed header byte length.
const HEADER_LEN: usize = core::mem::size_of::<RawStoreHeader>();

/// Fixed store header.
///
/// The variable catalog/topology/property body is a product-level serde payload
/// because it contains strings and typed values. The IO boundary starts with
/// this zerocopy header so validation never allocates before the length,
/// version, and checksum contract are known.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
struct RawStoreHeader {
    /// Store magic.
    magic: [u8; 8],
    /// Store format version.
    version: U64<LE>,
    /// Last visible commit sequence.
    commit_seq: U64<LE>,
    /// Last committed or empty-committed writer transaction ID.
    transaction_id: U64<LE>,
    /// Payload byte length.
    payload_len: U64<LE>,
    /// Payload checksum.
    payload_checksum: U64<LE>,
    /// Reserved bytes.
    reserved: [u8; 16],
}

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
    let payload = serde_json::to_vec(stored)?;
    let header = RawStoreHeader {
        magic: STORE_MAGIC,
        version: U64::new(STORE_VERSION),
        commit_seq: U64::new(stored.commit_seq.get()),
        transaction_id: U64::new(stored.transaction_id.get()),
        payload_len: U64::new(u64::try_from(payload.len()).map_err(|_error| DbError::IdOverflow)?),
        payload_checksum: U64::new(checksum_bytes(&payload)),
        reserved: [0; 16],
    };
    let temp_path = root.join(TEMP_STORE_FILE);
    let mut file = File::create(&temp_path).map_err(|error| DbError::io("create store", error))?;
    file.write_all(header.as_bytes())
        .map_err(|error| DbError::io("write store header", error))?;
    file.write_all(&payload)
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
/// This function is `O(serialized database bytes)`.
pub(crate) fn read_store(root: &Path) -> Result<StoredDatabase, DbError> {
    let mut file = File::open(store_path(root)).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => DbError::NotFound,
        _kind => DbError::io("open store", error),
    })?;
    let header = read_header(&mut file)?;
    let payload_len = usize::try_from(header.payload_len.get())
        .map_err(|_error| DbError::invalid_store("payload length does not fit usize"))?;
    let mut payload = vec![0_u8; payload_len];
    file.read_exact(&mut payload)
        .map_err(|error| DbError::io("read store payload", error))?;
    reject_trailing_bytes(&mut file)?;
    if checksum_bytes(&payload) != header.payload_checksum.get() {
        return Err(DbError::invalid_store("store payload checksum mismatch"));
    }
    let stored: StoredDatabase = serde_json::from_slice(&payload)?;
    if stored.commit_seq.get() != header.commit_seq.get()
        || stored.transaction_id.get() != header.transaction_id.get()
    {
        return Err(DbError::invalid_store(
            "store header does not match payload",
        ));
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

/// Reads and validates the fixed store header.
fn read_header(file: &mut File) -> Result<RawStoreHeader, DbError> {
    let mut bytes = [0_u8; HEADER_LEN];
    file.read_exact(&mut bytes)
        .map_err(|error| DbError::io("read store header", error))?;
    let header = RawStoreHeader::read_from_bytes(bytes.as_slice())
        .map_err(|_error| DbError::invalid_store("store header layout mismatch"))?;
    if header.magic != STORE_MAGIC || header.version.get() != STORE_VERSION {
        return Err(DbError::invalid_store("store magic or version mismatch"));
    }
    if header.reserved != [0; 16] {
        return Err(DbError::invalid_store("store reserved bytes are non-zero"));
    }
    Ok(header)
}

/// Rejects extra bytes after the declared store payload.
fn reject_trailing_bytes(file: &mut File) -> Result<(), DbError> {
    let mut extra = [0_u8; 1];
    match file
        .read(&mut extra)
        .map_err(|error| DbError::io("read store trailer", error))?
    {
        0 => Ok(()),
        _count => Err(DbError::invalid_store("store has trailing bytes")),
    }
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

/// Computes a deterministic non-cryptographic checksum.
fn checksum_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
