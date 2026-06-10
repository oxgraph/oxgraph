//! Store lifecycle: create, open/recover, and the on-disk file helpers.
//!
//! `create` writes base-0, an empty delta-log, and the superblock (last, as
//! the create-complete marker); `open` recovers the live frontier by
//! replaying the valid delta-log prefix over the base the superblock names.

use std::{path::Path, sync::Arc};

use super::{CheckpointPolicy, Db};
use crate::{
    Catalog, CheckpointGeneration, CommitSeq, DbError, TransactionId,
    backing::Base,
    freeze::{self, FreezeStamps},
    overlay::{Overlay, Snapshot, WriteOverlay},
    state::NextIds,
    storage, wal,
    wire::SuperblockRecord,
};

/// Builds the base filename for generation `generation`.
///
/// # Performance
///
/// This function is `O(1)`.
pub(super) fn base_file(generation: u64) -> String {
    format!("base-{generation}.oxgdb")
}

/// Builds the delta-log filename for generation `generation`.
///
/// # Performance
///
/// This function is `O(1)`.
pub(super) fn delta_file(generation: u64) -> String {
    format!("delta-{generation}.log")
}

impl Db {
    /// Creates a new empty OXGDB database at `path`.
    ///
    /// The create order is base-0 then empty delta-0.log then the writer lock
    /// file then the superblock (written LAST as the create-complete marker), so
    /// a half-created store is detected on open rather than silently opened
    /// empty.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::AlreadyExists`] when a store already exists, or
    /// [`DbError::Io`]/[`DbError::InvalidStore`] when creation fails.
    ///
    /// # Performance
    ///
    /// This function is `O(empty base bytes)`.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let root = path.as_ref().to_path_buf();
        if root.join(wal::SUPERBLOCK_FILE).exists() {
            return Err(DbError::AlreadyExists);
        }
        // Base-0: an empty merged view (empty base under an empty overlay).
        let empty_base = crate::overlay::BaseRecords::empty();
        let empty_overlay = Overlay::empty(NextIds::INITIAL, Catalog::empty());
        let view = crate::overlay::MergedState::new(&empty_base, &empty_overlay);
        let base_bytes = freeze::freeze_view(
            &view,
            FreezeStamps {
                commit_seq: 0,
                transaction_id: 0,
                generation: 0,
            },
        )?;
        storage::atomic_write(
            &root,
            &root.join(format!("{}.tmp", base_file(0))),
            &root.join(base_file(0)),
            &base_bytes,
        )?;
        // Empty delta-0.log, durably created.
        create_empty_log(&root, 0)?;
        // Superblock is written LAST; its existence is the create-complete marker.
        write_superblock(&root, 0, 0, 0, 0)?;
        Self::open(&root)
    }

    /// Opens an existing OXGDB database, recovering the live frontier from the
    /// valid prefix of the delta-log replayed over the base named by the
    /// superblock.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the store is missing, malformed, or the log is
    /// corrupt beyond a torn tail.
    ///
    /// # Performance
    ///
    /// This function is `O(base bytes + log bytes)`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let root = path.as_ref().to_path_buf();
        let superblock = wal::read_superblock(&root)?;
        let generation = superblock.base_generation.get();

        let base = Arc::new(Base::open(&root.join(base_file(generation)), false)?);
        let base_records = Arc::new(crate::overlay::BaseRecords::open(&base)?);
        let base_header = *base.get().header();
        let base_catalog = base.get().catalog().clone();
        let base_next = NextIds::from_header(&base_header);

        // Replay the valid prefix of the per-generation delta-log.
        let log_path = root.join(delta_file(generation));
        let log_bytes = read_log(&log_path)?;
        let outcome = wal::replay(generation, &log_bytes)?;
        // A torn tail truncates the log back to its last-good byte length.
        if outcome.valid_len < log_bytes.len() {
            truncate_log(&log_path, outcome.valid_len)?;
        }

        // Fold the replayed frames into a fresh overlay over the base, deriving
        // the live frontier (commit_seq/txn_id) from the last good frame.
        let mut write = WriteOverlay::new(base_next, base_catalog);
        let mut recovered_next = base_next;
        let mut last_commit_seq = superblock.commit_seq.get();
        let mut last_txn = superblock.transaction_id.get();
        for frame in &outcome.frames {
            for op in &frame.ops {
                write.apply_replay_op(&base_records, op, &frame.blob, frame.lsn)?;
            }
            recovered_next = recovered_next.elementwise_max(write.next_ids());
            last_commit_seq = frame.lsn;
            last_txn = last_txn.max(frame.txn_id);
        }
        // ids are never reused: the recovered watermark is the elementwise max of
        // the base header and every replayed frame's watermark.
        write.set_next_ids(recovered_next);
        let overlay = Arc::new(write.freeze());

        // Reuse the records already decoded for replay instead of decoding the base
        // a second time inside `Snapshot::new`: the pinned base is byte-identical, so
        // the records (and their derived index) match. Halves open's base-decode cost.
        let snapshot = Arc::new(Snapshot::with_shared_base_records(
            CheckpointGeneration::new(generation),
            CommitSeq::new(last_commit_seq),
            base,
            overlay,
            base_records,
        ));

        Ok(Self {
            root,
            current: snapshot,
            base_generation: generation,
            last_transaction_id: TransactionId::new(last_txn),
            checkpoint_policy: CheckpointPolicy::default(),
        })
    }
}

/// Returns the on-disk byte length of `path`, or `0` when it is absent or cannot
/// be stat'd (size is advisory — used for status reporting and the
/// auto-checkpoint heuristic, never for correctness).
///
/// # Performance
///
/// This function is `O(1)`: one `stat` syscall.
pub(super) fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

/// Reads the whole delta-log into memory, treating a missing file as empty.
///
/// # Errors
///
/// Returns [`DbError::Io`] when the file cannot be read.
///
/// # Performance
///
/// This function is `O(log bytes)`.
pub(super) fn read_log(path: &Path) -> Result<Vec<u8>, DbError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(DbError::io("read delta-log", error)),
    }
}

/// Truncates the delta-log back to `len` (its last-good byte length) and fsyncs,
/// discarding a torn tail under the open path.
///
/// # Errors
///
/// Returns [`DbError::Io`] when opening, truncating, or syncing fails.
///
/// # Performance
///
/// This function is `O(1)`.
pub(super) fn truncate_log(path: &Path, len: usize) -> Result<(), DbError> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| DbError::io("open delta-log for truncate", error))?;
    let len = u64::try_from(len)
        .map_err(|_overflow| DbError::invalid_store("delta-log length overflow"))?;
    file.set_len(len)
        .map_err(|error| DbError::io("truncate delta-log", error))?;
    file.sync_all()
        .map_err(|error| DbError::io("sync truncated delta-log", error))
}

/// Creates an empty per-generation delta-log, fsyncing the file and the
/// directory entry so the new (empty) log is durable.
///
/// # Errors
///
/// Returns [`DbError::Io`] when creation or syncing fails.
///
/// # Performance
///
/// This function is `O(1)`.
pub(super) fn create_empty_log(root: &Path, generation: u64) -> Result<(), DbError> {
    let path = root.join(delta_file(generation));
    let file =
        std::fs::File::create(&path).map_err(|error| DbError::io("create delta-log", error))?;
    file.sync_all()
        .map_err(|error| DbError::io("sync delta-log", error))?;
    storage::sync_directory(root)
}

/// Opens the live delta-log for appending (create when absent, read+append).
///
/// # Errors
///
/// Returns [`DbError::Io`] when the log cannot be opened.
///
/// # Performance
///
/// This function is `O(1)`.
pub(super) fn open_log_for_append(root: &Path, generation: u64) -> Result<std::fs::File, DbError> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .append(true)
        .open(root.join(delta_file(generation)))
        .map_err(|error| DbError::io("open delta-log for append", error))
}

/// Writes the superblock naming `generation` with the given frontier stamps.
///
/// # Errors
///
/// Returns [`DbError::Io`] when publishing fails.
///
/// # Performance
///
/// This function is `O(1)`.
pub(super) fn write_superblock(
    root: &Path,
    generation: u64,
    checkpoint_lsn: u64,
    commit_seq: u64,
    transaction_id: u64,
) -> Result<(), DbError> {
    wal::write_superblock(
        root,
        &SuperblockRecord {
            magic: crate::wire::SUPERBLOCK_MAGIC,
            base_generation: generation.into(),
            checkpoint_lsn: checkpoint_lsn.into(),
            log_byte_offset: 0u64.into(),
            commit_seq: commit_seq.into(),
            transaction_id: transaction_id.into(),
            format_version: crate::wire::OXGDB_FORMAT_VERSION.into(),
            flags: 0u32.into(),
            crc32c: 0u32.into(),
            pad: 0u32.into(),
        },
    )
}
