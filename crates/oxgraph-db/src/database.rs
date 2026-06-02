//! Embedded `OxGraph` database engine API.
//!
//! This is the integration layer over the base+overlay+WAL core. A [`Database`]
//! holds the current `Arc<Snapshot>` (one immutable base generation plus the
//! frozen overlay published over it), the open append-only delta-log, and the
//! recovered id/transaction watermarks. Reads pin the current snapshot in `O(1)`
//! (`begin_read` clones the `Arc`); writes layer a fresh [`WriteOverlay`] over
//! the current snapshot, append a WAL frame on commit, and publish a new
//! snapshot. The whole read/query/projection surface resolves through the merged
//! [`StateView`] of the pinned snapshot; the old owned whole-DB state is gone.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    Catalog, CheckpointGeneration, CommitSeq, DbError, ElementId, ElementRecord, GraphProjection,
    HypergraphProjection, IncidenceId, IncidenceRecord, IndexId, LabelId, PreparedQuery,
    ProjectionDefinition, ProjectionId, PropertyKeyId, PropertySubject, PropertyType,
    PropertyValue, QueryLanguage, QueryResult, RelationId, RelationRecord, RelationTypeId, RoleId,
    TransactionId,
    backing::Base,
    catalog::{IndexDefinition, PropertyFamily},
    freeze::{self, FreezeStamps},
    lock::WriterLock,
    overlay::{Overlay, Snapshot, StateView, WriteOverlay},
    projection,
    state::NextIds,
    storage,
    traversal::{self, TraversalOptions, TraversalResult},
    wal,
    wire::SuperblockRecord,
};

/// Lookup input for a cataloged index.
///
/// This type makes index lookup shape explicit: membership indexes accept
/// [`IndexLookup::All`], single-property indexes accept scalar equality or
/// range inputs, and composite equality indexes accept an ordered value tuple.
///
/// # Performance
///
/// Copying this value is `O(1)`.
#[derive(Clone, Copy, Debug)]
pub enum IndexLookup<'value> {
    /// Lookup every subject represented by a membership-style index.
    All,
    /// Lookup one scalar equality value.
    Equal(&'value PropertyValue),
    /// Lookup one inclusive scalar range.
    Range {
        /// Inclusive lower bound.
        min: &'value PropertyValue,
        /// Inclusive upper bound.
        max: &'value PropertyValue,
    },
    /// Lookup one ordered composite equality tuple.
    CompositeEqual(&'value [PropertyValue]),
}

/// Auto-checkpoint policy: decides when a dirty commit should fold the
/// delta-log into a fresh base generation, bounding the log tail that recovery
/// must replay.
///
/// The default is size-ratio: trigger when the delta-log grows past `factor`
/// times the live base size (`factor` configurable). [`CheckpointPolicy::Manual`]
/// disables auto-triggering entirely (checkpoint only via
/// [`Database::checkpoint`]/[`Database::compact`]).
///
/// # Performance
///
/// Copying this value is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointPolicy {
    /// Never auto-checkpoint; the caller drives [`Database::checkpoint`].
    Manual,
    /// Auto-checkpoint after a dirty commit once the delta-log exceeds `factor`
    /// times the live base size (a small floor guards a tiny/empty base so the
    /// gen-0 store does not checkpoint on its first commit).
    SizeRatio {
        /// Log-to-base size factor `K`; the log may grow to `K × base` bytes
        /// before the next dirty commit folds it.
        factor: u32,
    },
}

impl CheckpointPolicy {
    /// The default auto-checkpoint factor `K`: fold when the delta-log exceeds
    /// four times the live base size.
    pub const DEFAULT_FACTOR: u32 = 4;

    /// The base-size floor (bytes) below which the size-ratio policy never fires,
    /// so a freshly created (near-empty) base is not checkpointed on its first
    /// commits before it carries meaningful data.
    const MIN_BASE_BYTES: u64 = 4 * 1024;

    /// Returns whether a delta-log of `log_bytes` over a base of `base_bytes`
    /// should trigger an auto-checkpoint under this policy.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    const fn should_checkpoint(self, log_bytes: u64, base_bytes: u64) -> bool {
        match self {
            Self::Manual => false,
            Self::SizeRatio { factor } => {
                let floor = if base_bytes < Self::MIN_BASE_BYTES {
                    Self::MIN_BASE_BYTES
                } else {
                    base_bytes
                };
                log_bytes > floor.saturating_mul(factor as u64)
            }
        }
    }
}

impl Default for CheckpointPolicy {
    /// The default policy: size-ratio with [`CheckpointPolicy::DEFAULT_FACTOR`].
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn default() -> Self {
        Self::SizeRatio {
            factor: Self::DEFAULT_FACTOR,
        }
    }
}

/// Builds the base filename for generation `generation`.
///
/// # Performance
///
/// This function is `O(1)`.
fn base_file(generation: u64) -> String {
    format!("base-{generation}.oxgdb")
}

/// Builds the delta-log filename for generation `generation`.
///
/// # Performance
///
/// This function is `O(1)`.
fn delta_file(generation: u64) -> String {
    format!("delta-{generation}.log")
}

/// Open OXGDB database handle.
///
/// # Performance
///
/// Moving a handle is `O(1)`: it moves the current `Arc<Snapshot>` and the open
/// delta-log handle.
pub struct Database {
    /// Root database directory.
    root: PathBuf,
    /// The current visible snapshot (base generation + published overlay),
    /// shared by readers through an atomically reference-counted handle.
    current: Arc<Snapshot>,
    /// Live base generation named by the superblock; every delta frame and the
    /// per-generation log filename carry it.
    base_generation: u64,
    /// Last writer transaction id durably recorded (the last dirty commit's id).
    /// A rollback burns a session-local id above this but does not advance it.
    last_transaction_id: TransactionId,
    /// Auto-checkpoint policy consulted after each dirty commit.
    checkpoint_policy: CheckpointPolicy,
}

impl Database {
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
        let base_records = Arc::new(crate::overlay::BaseRecords::from_view(base.get())?);
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

        let snapshot = Arc::new(Snapshot::new(
            CheckpointGeneration::new(generation),
            CommitSeq::new(last_commit_seq),
            base,
            overlay,
        )?);

        Ok(Self {
            root,
            current: snapshot,
            base_generation: generation,
            last_transaction_id: TransactionId::new(last_txn),
            checkpoint_policy: CheckpointPolicy::default(),
        })
    }

    /// Returns the live base generation named by the superblock (the count of
    /// folds this store has undergone; gen-0 is the freshly created store).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn live_generation(&self) -> CheckpointGeneration {
        CheckpointGeneration::new(self.base_generation)
    }

    /// Returns the configured auto-checkpoint policy.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn checkpoint_policy(&self) -> CheckpointPolicy {
        self.checkpoint_policy
    }

    /// Sets the auto-checkpoint policy consulted after each dirty commit.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub const fn set_checkpoint_policy(&mut self, policy: CheckpointPolicy) {
        self.checkpoint_policy = policy;
    }

    /// Validates the current handle by re-reading the superblock and verifying
    /// the live base's content CRC.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the superblock or base fails validation.
    ///
    /// # Performance
    ///
    /// This method is `O(base bytes)`.
    pub fn validate(&self) -> Result<(), DbError> {
        wal::read_superblock(&self.root)?;
        Base::open(&self.root.join(base_file(self.base_generation)), false).map(|_base| ())
    }

    /// Validates an OXGDB database at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the store fails to open and recover.
    ///
    /// # Performance
    ///
    /// This function is `O(base bytes + log bytes)`.
    pub fn validate_path(path: impl AsRef<Path>) -> Result<(), DbError> {
        Self::open(path).map(|_database| ())
    }

    /// Folds the current base+overlay into a new base generation, rotating the
    /// delta-log and republishing the superblock (a manual checkpoint).
    ///
    /// This is the checkpoint primitive, exposed here so the existing `compact`
    /// API keeps its "rewrite the store compactly" contract. Auto-triggering is
    /// configured separately via [`Database::set_checkpoint_policy`].
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when encoding, writing, or publishing the new
    /// generation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state bytes)`.
    pub fn compact(&mut self) -> Result<(), DbError> {
        self.checkpoint()
    }

    /// Folds the current base+overlay into base-`{g+1}`, creates an empty
    /// delta-`{g+1}`.log, republishes the superblock naming `g+1` (the
    /// linearization point), then unlinks the old base and log.
    ///
    /// The order is crash-safe: the new base is fully durable BEFORE the
    /// superblock names it (so a crash before the superblock leaves the OLD
    /// superblock authoritative and the orphan new base is ignored), and the old
    /// base/log are unlinked only AFTER the superblock names the new generation
    /// (so a crash before the unlink leaves the NEW superblock authoritative and
    /// the orphan old files are ignored). The
    /// [`crate::wire::SuperblockRecord`] rename is the single linearization point.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when encoding, writing, or publishing fails.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state bytes)`.
    pub fn checkpoint(&mut self) -> Result<(), DbError> {
        self.checkpoint_inner(
            #[cfg(test)]
            CheckpointStop::Complete,
        )
    }

    /// Crash-safe checkpoint body. Under `#[cfg(test)]` it accepts a
    /// [`CheckpointStop`] that simulates a crash by returning early right after a
    /// chosen fsync point, leaving the on-disk files exactly as a real crash
    /// there would, so the crash-matrix test can reopen and assert recovery.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when encoding, writing, or publishing fails.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state bytes)`.
    fn checkpoint_inner(&mut self, #[cfg(test)] stop: CheckpointStop) -> Result<(), DbError> {
        let _lock = WriterLock::acquire(&self.root)?;
        let next_generation = self
            .base_generation
            .checked_add(1)
            .ok_or_else(|| DbError::invalid_store("checkpoint generation overflow"))?;
        let view = self.current.view();
        let commit_seq = self.current.lsn().get();
        let base_bytes = freeze::freeze_view(
            &view,
            FreezeStamps {
                commit_seq,
                transaction_id: self.last_transaction_id.get(),
                generation: next_generation,
            },
        )?;
        // (1) write base-{g+1} (temp + fsync + rename + dir-fsync).
        storage::atomic_write(
            &self.root,
            &self
                .root
                .join(format!("{}.tmp", base_file(next_generation))),
            &self.root.join(base_file(next_generation)),
            &base_bytes,
        )?;
        // (2) create empty delta-{g+1}.log (fsync + dir-fsync).
        create_empty_log(&self.root, next_generation)?;
        // Crash point A: new base + new log durable, superblock NOT yet
        // published. The OLD superblock still names `g`, so recovery uses the old
        // generation; the new base/log are orphans.
        #[cfg(test)]
        if matches!(stop, CheckpointStop::BeforeSuperblock) {
            return Ok(());
        }
        // (3) publish the superblock naming g+1 — the linearization point.
        write_superblock(
            &self.root,
            next_generation,
            commit_seq,
            commit_seq,
            self.last_transaction_id.get(),
        )?;
        // Crash point B: superblock now names g+1, old base/log NOT yet unlinked.
        // Recovery uses the new generation; the old base/log are orphans.
        #[cfg(test)]
        if matches!(stop, CheckpointStop::BeforeRotate) {
            return Ok(());
        }
        // Re-open over the new generation, then (4) unlink the old base + log.
        let reopened = Self::open(&self.root)?;
        let old_generation = self.base_generation;
        let policy = self.checkpoint_policy;
        self.current = reopened.current;
        self.base_generation = reopened.base_generation;
        self.last_transaction_id = reopened.last_transaction_id;
        // The reopen reset the policy to the default; restore the caller's.
        self.checkpoint_policy = policy;
        let _ = std::fs::remove_file(self.root.join(base_file(old_generation)));
        let _ = std::fs::remove_file(self.root.join(delta_file(old_generation)));
        let _ = storage::sync_directory(&self.root);
        Ok(())
    }

    /// Auto-checkpoints when the configured [`CheckpointPolicy`] says the
    /// delta-log has grown too large relative to the base. Called after a dirty
    /// commit publishes its frame. A failed fold is surfaced so the caller can
    /// observe it; the committed data is already durable in the log regardless.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the triggered fold fails.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to decide; `O(visible state bytes)` when it folds.
    fn maybe_auto_checkpoint(&mut self) -> Result<(), DbError> {
        let log_bytes = file_len(&self.root.join(delta_file(self.base_generation)));
        let base_bytes = file_len(&self.root.join(base_file(self.base_generation)));
        if self
            .checkpoint_policy
            .should_checkpoint(log_bytes, base_bytes)
        {
            self.checkpoint()?;
        }
        Ok(())
    }

    /// Returns operational status for this handle, including the live generation
    /// count and the on-disk base/delta-log sizes the auto-checkpoint policy
    /// weighs.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state)` for the merged counts plus two `stat`
    /// syscalls for the file sizes.
    #[must_use]
    pub fn status(&self) -> DatabaseStatus {
        let view = self.current.view();
        DatabaseStatus {
            visible_commit_seq: self.current.lsn(),
            last_transaction_id: self.last_transaction_id,
            live_generation: CheckpointGeneration::new(self.base_generation),
            base_byte_size: file_len(&self.root.join(base_file(self.base_generation))),
            log_byte_size: file_len(&self.root.join(delta_file(self.base_generation))),
            element_count: view.element_count(),
            relation_count: view.relation_count(),
            incidence_count: view.incidence_count(),
            catalog: self.catalog_summary(),
        }
    }

    /// Returns a catalog-size summary.
    ///
    /// # Performance
    ///
    /// This method is `O(catalog entry count)`.
    #[must_use]
    pub fn catalog_summary(&self) -> CatalogSummary {
        CatalogSummary::from_catalog(self.current.view().catalog())
    }

    /// Starts a read transaction pinned to the current visible snapshot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`: the reader clones the current `Arc<Snapshot>` and
    /// observes a fixed state even across later commits and checkpoints.
    #[must_use]
    pub fn begin_read(&self) -> ReadTransaction {
        ReadTransaction {
            snapshot: Arc::clone(&self.current),
        }
    }

    /// Starts the single writer transaction, acquiring the cross-process writer
    /// lock for the transaction's lifetime.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::WriterLockHeld`] when another writer holds the lock or
    /// [`DbError::TransactionIdOverflow`] when writer ids are exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`: the writer layers a fresh empty write overlay over
    /// the current snapshot.
    pub fn begin_write(&mut self) -> Result<WriteTransaction<'_>, DbError> {
        let lock = WriterLock::acquire(&self.root)?;
        let transaction_id = self
            .last_transaction_id
            .checked_next()
            .ok_or(DbError::TransactionIdOverflow)?;
        // Burn the id eagerly so it is session-local-visible even on rollback;
        // it only becomes durable when a dirty commit writes its frame, and a
        // reopen recovers the durable high-water mark from the log.
        self.last_transaction_id = transaction_id;
        let parent = Arc::clone(&self.current);
        // Seed the writer delta from the parent's published overlay so the
        // writer reads every committed record; the parent overlay is never
        // mutated (the seed clones its maps).
        let delta = WriteOverlay::from_overlay(parent.overlay());
        Ok(WriteTransaction {
            database: self,
            parent,
            delta,
            transaction_id,
            lock,
        })
    }

    /// Prepares a query against the current catalog.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when parsing or semantic analysis fails.
    ///
    /// # Performance
    ///
    /// This method is `O(query length + catalog lookup cost)`.
    pub fn prepare(&self, language: QueryLanguage, query: &str) -> Result<PreparedQuery, DbError> {
        PreparedQuery::prepare(language, query, &self.current.view())
    }
}

/// Returns the on-disk byte length of `path`, or `0` when it is absent or cannot
/// be stat'd (size is advisory — used for status reporting and the
/// auto-checkpoint heuristic, never for correctness).
///
/// # Performance
///
/// This function is `O(1)`: one `stat` syscall.
fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

/// Test-only crash-injection point for [`Database::checkpoint_inner`]: stops the
/// fold right after a chosen fsync so the crash-matrix test can reopen and assert
/// the recovered state at each crash window.
///
/// The crash-matrix test that constructs the non-`Complete` variants is
/// `#[cfg(not(miri))]` (it reopens a real store across simulated crashes, which
/// miri's isolation cannot model), so under miri only `Complete` is constructed
/// and the other variants are expectedly unused.
///
/// # Performance
///
/// `perf: unspecified`; a test-only control tag.
#[cfg(test)]
#[cfg_attr(
    miri,
    expect(
        dead_code,
        reason = "the crash-injection variants are constructed only by the #[cfg(not(miri))] crash-matrix test"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckpointStop {
    /// Run the whole checkpoint (the production path).
    Complete,
    /// Stop after the new base + new log are durable, before the superblock is
    /// published (the old superblock stays authoritative).
    BeforeSuperblock,
    /// Stop after the superblock names the new generation, before the old
    /// base/log are unlinked (the new superblock is authoritative).
    BeforeRotate,
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
fn read_log(path: &Path) -> Result<Vec<u8>, DbError> {
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
fn truncate_log(path: &Path, len: usize) -> Result<(), DbError> {
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
fn create_empty_log(root: &Path, generation: u64) -> Result<(), DbError> {
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
fn open_log_for_append(root: &Path, generation: u64) -> Result<std::fs::File, DbError> {
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
fn write_superblock(
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

/// Snapshot of database status.
///
/// # Performance
///
/// Copying and comparing status is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseStatus {
    /// Last visible commit sequence.
    pub visible_commit_seq: CommitSeq,
    /// Last writer transaction ID burned by this handle.
    ///
    /// This value is durable after a dirty commit and session-local after
    /// rollback.
    pub last_transaction_id: TransactionId,
    /// Live base generation named by the superblock (the count of folds this
    /// store has undergone; gen-0 is the freshly created store).
    pub live_generation: CheckpointGeneration,
    /// On-disk byte size of the live base file.
    pub base_byte_size: u64,
    /// On-disk byte size of the live delta-log (the tail recovery replays and
    /// the auto-checkpoint policy weighs against the base size).
    pub log_byte_size: u64,
    /// Visible element count.
    pub element_count: usize,
    /// Visible relation count.
    pub relation_count: usize,
    /// Visible incidence count.
    pub incidence_count: usize,
    /// Catalog-size summary.
    pub catalog: CatalogSummary,
}

/// Catalog-size summary.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogSummary {
    /// Role count.
    pub role_count: usize,
    /// Label count.
    pub label_count: usize,
    /// Relation type count.
    pub relation_type_count: usize,
    /// Property key count.
    pub property_key_count: usize,
    /// Projection count.
    pub projection_count: usize,
    /// Index count.
    pub index_count: usize,
}

impl CatalogSummary {
    /// Builds a summary from a catalog.
    ///
    /// # Performance
    ///
    /// This function is `O(catalog entry count)`.
    #[must_use]
    pub fn from_catalog(catalog: &Catalog) -> Self {
        Self {
            role_count: catalog.roles().count(),
            label_count: catalog.labels().count(),
            relation_type_count: catalog.relation_types().count(),
            property_key_count: catalog.property_keys().count(),
            projection_count: catalog.projections().count(),
            index_count: catalog.indexes().count(),
        }
    }
}

/// Reader pin identifying the visible database generation.
///
/// # Performance
///
/// Copying and comparing a pin is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadPin {
    /// Pinned visible commit sequence.
    pub visible_commit_seq: CommitSeq,
    /// Pinned checkpoint generation.
    pub generation: CheckpointGeneration,
}

/// Read transaction over a pinned snapshot.
///
/// A read transaction owns its own `Arc<Snapshot>` and never borrows the
/// [`Database`], so it stays valid across a later `begin_write`/`checkpoint` on
/// the same handle (it cloned the snapshot before the write borrowed `&mut`). It
/// is [`Send`] + [`Sync`] (asserted below).
///
/// # Performance
///
/// Creating and cloning a read transaction is `O(1)`: it shares the pinned
/// snapshot through an `Arc`, not by copying.
pub struct ReadTransaction {
    /// The pinned snapshot this reader observes.
    snapshot: Arc<Snapshot>,
}

/// `ReadTransaction` MUST be `Send + Sync`: it pins only an `Arc<Snapshot>`,
/// which holds `Arc`-shared `Send + Sync` data (no `Rc`/`RefCell` reachable).
const fn assert_send_sync<T: Send + Sync>() {}
const _: () = assert_send_sync::<ReadTransaction>();
const _: () = assert_send_sync::<Arc<Snapshot>>();

impl ReadTransaction {
    /// Returns this transaction's reader pin.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn pin(&self) -> ReadPin {
        ReadPin {
            visible_commit_seq: self.snapshot.lsn(),
            generation: self.snapshot.generation(),
        }
    }

    /// Returns catalog metadata.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn catalog(&self) -> &Catalog {
        self.snapshot.view().catalog_ref()
    }

    /// Returns visible element count.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    #[must_use]
    pub fn element_count(&self) -> usize {
        self.snapshot.view().element_count()
    }

    /// Returns visible relation count.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    #[must_use]
    pub fn relation_count(&self) -> usize {
        self.snapshot.view().relation_count()
    }

    /// Returns visible incidence count.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    #[must_use]
    pub fn incidence_count(&self) -> usize {
        self.snapshot.view().incidence_count()
    }

    /// Returns every visible element id in id order.
    ///
    /// # Performance
    ///
    /// This method is `O(element count)`.
    #[must_use]
    pub fn element_ids(&self) -> Vec<ElementId> {
        self.snapshot
            .view()
            .elements()
            .map(|record| record.id)
            .collect()
    }

    /// Returns every visible relation id in id order.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count)`.
    #[must_use]
    pub fn relation_ids(&self) -> Vec<RelationId> {
        self.snapshot
            .view()
            .relations()
            .map(|record| record.id)
            .collect()
    }

    /// Returns whether an element exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    #[must_use]
    pub fn contains_element(&self, id: ElementId) -> bool {
        self.snapshot.view().contains_element(id)
    }

    /// Returns whether a relation exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    #[must_use]
    pub fn contains_relation(&self, id: RelationId) -> bool {
        self.snapshot.view().contains_relation(id)
    }

    /// Returns whether an incidence exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    #[must_use]
    pub fn contains_incidence(&self, id: IncidenceId) -> bool {
        self.snapshot.view().contains_incidence(id)
    }

    /// Returns an element record, borrowed from the base for a base-only id and
    /// owned for an overlay-supplied id.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    #[must_use]
    pub fn element(&self, id: ElementId) -> Option<Cow<'_, ElementRecord>> {
        self.snapshot.view().element_ref(id)
    }

    /// Returns a relation record (see [`Self::element`] for the borrow contract).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    #[must_use]
    pub fn relation(&self, id: RelationId) -> Option<Cow<'_, RelationRecord>> {
        self.snapshot.view().relation_ref(id)
    }

    /// Returns an incidence record (see [`Self::element`] for the borrow
    /// contract).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    #[must_use]
    pub fn incidence(&self, id: IncidenceId) -> Option<Cow<'_, IncidenceRecord>> {
        self.snapshot.view().incidence_ref(id)
    }

    /// Returns every visible incidence attached to an element, in ascending
    /// incidence-id order.
    ///
    /// The merged set mixes overlay-owned and base-borrowed records, so this
    /// returns an owned [`Vec`] ([`IncidenceRecord`] is [`Copy`], so the copy is
    /// cheap).
    ///
    /// # Performance
    ///
    /// This method is `O(base incidences + overlay incidence change)`.
    #[must_use]
    pub fn element_incidences(&self, id: ElementId) -> Vec<IncidenceRecord> {
        self.snapshot.view().element_incidences(id)
    }

    /// Returns one property value (see [`Self::element`] for the borrow
    /// contract).
    ///
    /// # Performance
    ///
    /// This method is `O(log subjects + log keys)`.
    #[must_use]
    pub fn property(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'_, PropertyValue>> {
        self.snapshot.view().property_ref(subject, key)
    }

    /// Looks up subjects with a property value.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the property key is unknown or `value` does not
    /// match the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(property subject count)`.
    pub fn lookup_property_equal(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.snapshot.view().typed_property_equal(key, value)
    }

    /// Looks up subjects with a property inside an inclusive range.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the property key is unknown or either bound
    /// does not match the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(property subject count)`.
    pub fn lookup_property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.snapshot.view().typed_property_range(key, min, max)
    }

    /// Executes an index lookup.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the index is unknown, the lookup shape does not
    /// match the index kind, or supplied property values do not match catalog
    /// schemas.
    ///
    /// # Performance
    ///
    /// This method is `O(indexed family size)`.
    pub fn lookup_index(
        &self,
        index: IndexId,
        lookup: IndexLookup<'_>,
    ) -> Result<Vec<PropertySubject>, DbError> {
        let view = self.snapshot.view();
        let entry = view
            .catalog()
            .index(index)
            .ok_or(DbError::UnknownIndex { id: index })?;
        match (&entry.definition, lookup) {
            (IndexDefinition::Label { label }, IndexLookup::All) => Ok(view
                .elements_with_label(*label)
                .into_iter()
                .map(PropertySubject::Element)
                .collect()),
            (IndexDefinition::Label { .. }, _lookup) => {
                Err(DbError::unsupported("label index expects all lookup"))
            }
            (IndexDefinition::RelationType { relation_type }, IndexLookup::All) => Ok(view
                .relations_with_type(*relation_type)
                .into_iter()
                .map(PropertySubject::Relation)
                .collect()),
            (IndexDefinition::RelationType { .. }, _lookup) => Err(DbError::unsupported(
                "relation type index expects all lookup",
            )),
            (IndexDefinition::PropertyEquality { key }, IndexLookup::Equal(value)) => {
                view.typed_property_equal(*key, value)
            }
            (IndexDefinition::PropertyEquality { .. }, _lookup) => Err(DbError::unsupported(
                "property equality index expects equality lookup",
            )),
            (IndexDefinition::PropertyRange { key }, IndexLookup::Range { min, max }) => {
                view.typed_property_range(*key, min, max)
            }
            (IndexDefinition::PropertyRange { .. }, _lookup) => Err(DbError::unsupported(
                "property range index expects range lookup",
            )),
            (IndexDefinition::CompositeEquality { keys }, IndexLookup::CompositeEqual(values)) => {
                view.typed_property_composite_equal(keys, values)
            }
            (IndexDefinition::CompositeEquality { .. }, _lookup) => Err(DbError::unsupported(
                "composite equality index expects composite equality lookup",
            )),
            (IndexDefinition::Projection { projection }, IndexLookup::All) => {
                self.projection_index_subjects(*projection)
            }
            (IndexDefinition::Projection { .. }, _lookup) => {
                Err(DbError::unsupported("projection index expects all lookup"))
            }
        }
    }

    /// Materializes a graph projection.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a graph, or
    /// fails validation against current topology.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count * incidence count)`.
    pub fn graph_projection(&self, id: ProjectionId) -> Result<GraphProjection, DbError> {
        let view = self.snapshot.view();
        let entry = view
            .catalog()
            .projection(id)
            .ok_or(DbError::UnknownProjection { id })?;
        match &entry.definition {
            ProjectionDefinition::Graph(definition) => {
                projection::GraphProjection::from_state(&view, definition.clone())
            }
            ProjectionDefinition::Hypergraph(_definition) => {
                Err(DbError::invalid_projection("projection is not a graph"))
            }
        }
    }

    /// Materializes a graph projection by catalog name.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a graph, or
    /// fails validation against current topology.
    ///
    /// # Performance
    ///
    /// This method is `O(log projection count + relation count * incidence count)`.
    pub fn graph_projection_by_name(&self, name: &str) -> Result<GraphProjection, DbError> {
        let id = self
            .snapshot
            .view()
            .catalog()
            .projection_id(name)
            .ok_or_else(|| DbError::unsupported(format!("unknown projection {name}")))?;
        self.graph_projection(id)
    }

    /// Traverses a cataloged graph projection from canonical seed elements.
    ///
    /// Rows are unique canonical elements in BFS first-discovery order. Depth is
    /// the shortest discovered hop count from any seed.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a graph,
    /// cannot be materialized, or a seed element is not part of the projection.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count * incidence count + visited edges)`.
    pub fn traverse_graph(
        &self,
        projection: ProjectionId,
        seeds: &[ElementId],
        options: TraversalOptions,
    ) -> Result<TraversalResult, DbError> {
        if seeds.is_empty() || options.limit == 0 {
            return Ok(TraversalResult::new(Vec::new()));
        }
        let graph = self.graph_projection(projection)?;
        traversal::traverse_graph_projection(&graph, seeds, options)
    }

    /// Materializes a hypergraph projection.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a hypergraph,
    /// or fails validation against current topology.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count * incidence count)`.
    pub fn hypergraph_projection(&self, id: ProjectionId) -> Result<HypergraphProjection, DbError> {
        let view = self.snapshot.view();
        let entry = view
            .catalog()
            .projection(id)
            .ok_or(DbError::UnknownProjection { id })?;
        match &entry.definition {
            ProjectionDefinition::Hypergraph(definition) => {
                projection::HypergraphProjection::from_state(&view, definition.clone())
            }
            ProjectionDefinition::Graph(_definition) => Err(DbError::invalid_projection(
                "projection is not a hypergraph",
            )),
        }
    }

    /// Executes a prepared query.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when execution cannot materialize a referenced
    /// projection.
    ///
    /// # Performance
    ///
    /// This method is `O(plan output + projection build cost when used)`.
    pub fn execute(&self, query: &PreparedQuery) -> Result<QueryResult, DbError> {
        query.execute(&self.snapshot.view())
    }

    /// Explains a prepared query.
    ///
    /// # Performance
    ///
    /// This method is `O(plan size)`.
    #[must_use]
    pub fn explain(&self, query: &PreparedQuery) -> String {
        query.explain()
    }

    /// Materializes subjects represented by a projection index.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown or cannot be
    /// materialized.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count * incidence count)`.
    fn projection_index_subjects(
        &self,
        projection: ProjectionId,
    ) -> Result<Vec<PropertySubject>, DbError> {
        let view = self.snapshot.view();
        let entry = view
            .catalog()
            .projection(projection)
            .ok_or(DbError::UnknownProjection { id: projection })?;
        match &entry.definition {
            ProjectionDefinition::Graph(definition) => {
                Ok(projection::GraphProjection::from_state(&view, definition.clone())?.subjects())
            }
            ProjectionDefinition::Hypergraph(definition) => Ok(
                projection::HypergraphProjection::from_state(&view, definition.clone())?.subjects(),
            ),
        }
    }
}

/// Single writer transaction.
///
/// Mutations accumulate into a private write overlay layered over the parent
/// snapshot; reads fall through the overlay then the base. `commit` appends the
/// overlay's mutation log to the WAL (when dirty) and publishes a fresh snapshot;
/// `rollback` drops the overlay and appends nothing.
///
/// # Performance
///
/// Creating and moving a writer is `O(1)`; each mutation is `O(log change)`.
pub struct WriteTransaction<'db> {
    /// Database receiving the commit.
    database: &'db mut Database,
    /// Parent snapshot the writer layers over (its base + frozen overlay).
    parent: Arc<Snapshot>,
    /// Private mutable delta this writer accumulates.
    delta: WriteOverlay,
    /// Writer transaction id (session-local until a dirty commit makes it
    /// durable).
    transaction_id: TransactionId,
    /// Held single-writer advisory lock. Its [`Drop`] releases the lock when this
    /// transaction ends (on `rollback`, or on any early-return error path); a
    /// successful dirty [`Self::commit`] releases it explicitly with `drop` so a
    /// triggered auto-checkpoint can re-acquire it.
    lock: WriterLock,
}

impl WriteTransaction<'_> {
    /// Registers a structural incidence role.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the name already exists or ID allocation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(log role count + name length)`.
    pub fn register_role(&mut self, name: impl Into<String>) -> Result<RoleId, DbError> {
        self.delta.register_role(name.into())
    }

    /// Registers an element or relation label.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the name already exists or ID allocation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(log label count + name length)`.
    pub fn register_label(&mut self, name: impl Into<String>) -> Result<LabelId, DbError> {
        self.delta.register_label(name.into())
    }

    /// Registers a relation type.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the name already exists or ID allocation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(log relation type count + name length)`.
    pub fn register_relation_type(
        &mut self,
        name: impl Into<String>,
    ) -> Result<RelationTypeId, DbError> {
        self.delta.register_relation_type(name.into())
    }

    /// Registers a typed property key.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the name already exists or ID allocation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(log property key count + name length)`.
    pub fn register_property_key(
        &mut self,
        name: impl Into<String>,
        family: PropertyFamily,
        value_type: PropertyType,
    ) -> Result<PropertyKeyId, DbError> {
        self.delta
            .register_property_key(name.into(), family, value_type)
    }

    /// Defines a physical projection.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when referenced catalog IDs are unknown, the
    /// projection name already exists, or ID allocation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(definition size + catalog lookup cost)`.
    pub fn define_projection(
        &mut self,
        definition: ProjectionDefinition,
    ) -> Result<ProjectionId, DbError> {
        self.validate_projection_definition(&definition)?;
        self.delta.register_projection(definition)
    }

    /// Defines an index.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when referenced catalog IDs are unknown, the index
    /// name already exists, or ID allocation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(definition size + catalog lookup cost)`.
    pub fn define_index(
        &mut self,
        name: impl Into<String>,
        definition: IndexDefinition,
    ) -> Result<IndexId, DbError> {
        self.validate_index_definition(&definition)?;
        self.delta.register_index(name.into(), definition)
    }

    /// Creates a canonical element.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when element IDs are exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log element change)`.
    pub fn create_element(&mut self) -> Result<ElementId, DbError> {
        self.delta.create_element()
    }

    /// Creates a canonical relation.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when relation IDs are exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log relation change)`.
    pub fn create_relation(&mut self) -> Result<RelationId, DbError> {
        self.delta.create_relation()
    }

    /// Creates a canonical incidence.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when referenced IDs are unknown or incidence IDs are
    /// exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log incidence change + reference lookup cost)`.
    pub fn create_incidence(
        &mut self,
        relation: RelationId,
        element: ElementId,
        role: RoleId,
    ) -> Result<IncidenceId, DbError> {
        self.require_relation(relation)?;
        self.require_element(element)?;
        self.require_role(role)?;
        self.delta.create_incidence(relation, element, role)
    }

    /// Tombstones a canonical element and its incidences.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownElement`] when the element is not visible.
    ///
    /// # Performance
    ///
    /// This method is `O(incidence count)`.
    pub fn tombstone_element(&mut self, id: ElementId) -> Result<(), DbError> {
        self.require_element(id)?;
        let base = self.parent.base_records();
        self.delta.tombstone_element(base, id);
        // Cascade: every incidence referencing the element is tombstoned too.
        let incidences: Vec<IncidenceId> = self
            .merged()
            .incidences()
            .filter(|record| record.element == id)
            .map(|record| record.id)
            .collect();
        for incidence in incidences {
            self.delta
                .tombstone_incidence(self.parent.base_records(), incidence);
        }
        Ok(())
    }

    /// Tombstones a canonical relation and its incidences.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownRelation`] when the relation is not visible.
    ///
    /// # Performance
    ///
    /// This method is `O(incidence count)`.
    pub fn tombstone_relation(&mut self, id: RelationId) -> Result<(), DbError> {
        self.require_relation(id)?;
        let base = self.parent.base_records();
        self.delta.tombstone_relation(base, id);
        let incidences: Vec<IncidenceId> = self
            .merged()
            .incidences()
            .filter(|record| record.relation == id)
            .map(|record| record.id)
            .collect();
        for incidence in incidences {
            self.delta
                .tombstone_incidence(self.parent.base_records(), incidence);
        }
        Ok(())
    }

    /// Tombstones a canonical incidence.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownIncidence`] when the incidence is not visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log incidence change)`.
    pub fn tombstone_incidence(&mut self, id: IncidenceId) -> Result<(), DbError> {
        self.require_incidence(id)?;
        self.delta
            .tombstone_incidence(self.parent.base_records(), id);
        Ok(())
    }

    /// Adds a label to an element.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the element or label is unknown.
    ///
    /// # Performance
    ///
    /// This method is `O(log element change + log label count)`.
    pub fn add_element_label(&mut self, element: ElementId, label: LabelId) -> Result<(), DbError> {
        self.require_element(element)?;
        self.require_label(label)?;
        self.delta
            .add_element_label(self.parent.base_records(), element, label);
        Ok(())
    }

    /// Adds a label to a relation.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the relation or label is unknown.
    ///
    /// # Performance
    ///
    /// This method is `O(log relation change + log label count)`.
    pub fn add_relation_label(
        &mut self,
        relation: RelationId,
        label: LabelId,
    ) -> Result<(), DbError> {
        self.require_relation(relation)?;
        self.require_label(label)?;
        self.delta
            .add_relation_label(self.parent.base_records(), relation, label);
        Ok(())
    }

    /// Sets a relation type.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the relation or relation type is unknown.
    ///
    /// # Performance
    ///
    /// This method is `O(log relation change + log relation type count)`.
    pub fn set_relation_type(
        &mut self,
        relation: RelationId,
        relation_type: RelationTypeId,
    ) -> Result<(), DbError> {
        self.require_relation(relation)?;
        self.require_relation_type(relation_type)?;
        self.delta
            .set_relation_type(self.parent.base_records(), relation, relation_type);
        Ok(())
    }

    /// Sets a property value.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the subject or key is unknown, or the value
    /// does not match the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(log subject change + log key count)`.
    pub fn set_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: PropertyValue,
    ) -> Result<(), DbError> {
        // Referential integrity: the subject must be visible (this rejects an
        // orphan property against a tombstoned/absent subject at the transaction
        // boundary — the overlay layer is permissive by design).
        self.require_subject(subject)?;
        let definition = self
            .merged()
            .catalog()
            .property_key(key)
            .cloned()
            .ok_or(DbError::UnknownPropertyKey { id: key })?;
        if definition.family != subject.family() {
            return Err(DbError::WrongPropertyFamily {
                expected: definition.family,
                actual: subject.family(),
            });
        }
        if definition.value_type != value.value_type() {
            return Err(DbError::PropertyTypeMismatch {
                expected: definition.value_type,
                actual: value.value_type(),
            });
        }
        self.delta
            .set_property(self.parent.base_records(), subject, key, value);
        Ok(())
    }

    /// Removes a property value.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the subject or key is unknown.
    ///
    /// # Performance
    ///
    /// This method is `O(log subject change + log key count)`.
    pub fn remove_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Result<(), DbError> {
        self.require_subject(subject)?;
        if self.merged().catalog().property_key(key).is_none() {
            return Err(DbError::UnknownPropertyKey { id: key });
        }
        self.delta
            .remove_property(self.parent.base_records(), subject, key);
        Ok(())
    }

    /// Commits this write transaction durably.
    ///
    /// A non-dirty commit returns the parent's commit sequence without appending
    /// to the WAL or publishing. A dirty commit encodes the overlay's mutation
    /// log into one WAL frame (with the watermark op last), appends it with an
    /// fsync (truncating back to the captured EOF on any write error so no
    /// interior torn record survives), THEN folds the delta into a fresh
    /// `Arc<Overlay>` and publishes a new `Arc<Snapshot>`.
    ///
    /// After publishing, a dirty commit consults the configured
    /// [`CheckpointPolicy`]: it releases the writer lock FIRST (so the fold can
    /// re-acquire it), then folds when the delta-log has outgrown the base. The
    /// committed frame is already durable, so an auto-fold failure does not lose
    /// data; it is surfaced to the caller.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when commit-sequence allocation, frame encoding, the
    /// durable append, or a triggered auto-checkpoint fold fails.
    ///
    /// # Performance
    ///
    /// This method is `O(change)` for the dirty path — flat as the base grows.
    /// The publish step shares the parent snapshot's already-materialized
    /// [`crate::overlay::BaseRecords`] and derived index by `Arc` (a commit never
    /// folds, so the base is byte-identical within the generation), so it neither
    /// re-decodes the base nor rebuilds the index. A triggered fold adds
    /// `O(visible state bytes)` on top.
    pub fn commit(self) -> Result<CommitSeq, DbError> {
        if self.delta.is_empty() {
            // Non-dirty commit: no append, no publish, no durable id advance.
            return Ok(self.parent.lsn());
        }
        let lsn = self
            .parent
            .lsn()
            .checked_next()
            .ok_or(DbError::CommitSeqOverflow)?;
        let (ops, blob) = self.delta.encode_frame();
        let frame = wal::encode_commit(
            lsn.get(),
            self.transaction_id.get(),
            self.database.base_generation,
            &ops,
            &blob,
        )?;
        let mut log = open_log_for_append(&self.database.root, self.database.base_generation)?;
        wal::append_commit(&mut log, &frame)?;

        // Durable: the delta was seeded from the parent overlay and only added
        // this writer's changes, so freezing it directly is the full new
        // published overlay (parent state + this commit). The parent overlay was
        // never mutated — this is a brand-new frozen `Arc<Overlay>`, so a reader
        // pinning the parent is unaffected.
        let new_overlay = Arc::new(self.delta.freeze());
        // A commit never folds, so the new snapshot pins the SAME base generation
        // as the parent — the base wire bytes are byte-identical, and so are the
        // owned records and the derived index built from them. Share the parent's
        // `Arc<BaseRecords>` (and its `BaseIndex`) instead of re-decoding the base
        // and rebuilding the index, which keeps a single-element commit `O(change)`
        // rather than `O(base)` regardless of how large the base has grown.
        let snapshot = Snapshot::with_shared_base_records(
            self.parent.generation(),
            lsn,
            Arc::clone(self.parent.base()),
            new_overlay,
            Arc::clone(self.parent.base_records()),
        );
        self.database.current = Arc::new(snapshot);
        self.database.last_transaction_id = self.transaction_id;
        // Release the writer lock before any auto-fold so the fold can re-acquire
        // it (a partial move out of `self`, legal because `WriteTransaction` has
        // no `Drop` impl; the remaining `&mut Database` borrow stays live).
        drop(self.lock);
        self.database.maybe_auto_checkpoint()?;
        Ok(lsn)
    }

    /// Drops this write transaction without committing.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` excluding staged-delta drop cost.
    pub fn rollback(self) {}

    /// Returns the merged read view this writer sees (overlay over base).
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to construct.
    fn merged(&self) -> crate::overlay::WriteMergedState<'_> {
        crate::overlay::WriteMergedState::new(self.parent.base_records(), &self.delta)
    }

    /// Requires an element to be visible in the writer's merged view.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownElement`] when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn require_element(&self, id: ElementId) -> Result<(), DbError> {
        if self.merged().contains_element(id) {
            Ok(())
        } else {
            Err(DbError::UnknownElement { id })
        }
    }

    /// Requires a relation to be visible.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownRelation`] when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn require_relation(&self, id: RelationId) -> Result<(), DbError> {
        if self.merged().contains_relation(id) {
            Ok(())
        } else {
            Err(DbError::UnknownRelation { id })
        }
    }

    /// Requires an incidence to be visible.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownIncidence`] when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn require_incidence(&self, id: IncidenceId) -> Result<(), DbError> {
        if self.merged().contains_incidence(id) {
            Ok(())
        } else {
            Err(DbError::UnknownIncidence { id })
        }
    }

    /// Requires a role to exist in the merged catalog.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownRole`] when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log role count)`.
    fn require_role(&self, id: RoleId) -> Result<(), DbError> {
        if self.delta.catalog().role(id).is_some() {
            Ok(())
        } else {
            Err(DbError::UnknownRole { id })
        }
    }

    /// Requires a label to exist in the merged catalog.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownLabel`] when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log label count)`.
    fn require_label(&self, id: LabelId) -> Result<(), DbError> {
        if self.delta.catalog().label(id).is_some() {
            Ok(())
        } else {
            Err(DbError::UnknownLabel { id })
        }
    }

    /// Requires a relation type to exist in the merged catalog.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownRelationType`] when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log relation type count)`.
    fn require_relation_type(&self, id: RelationTypeId) -> Result<(), DbError> {
        if self.delta.catalog().relation_type(id).is_some() {
            Ok(())
        } else {
            Err(DbError::UnknownRelationType { id })
        }
    }

    /// Requires a property subject to be visible.
    ///
    /// # Errors
    ///
    /// Returns the matching `Unknown*` error when the subject is absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn require_subject(&self, subject: PropertySubject) -> Result<(), DbError> {
        match subject {
            PropertySubject::Element(id) => self.require_element(id),
            PropertySubject::Relation(id) => self.require_relation(id),
            PropertySubject::Incidence(id) => self.require_incidence(id),
        }
    }

    /// Validates one projection definition against the merged catalog.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when a referenced role or relation type is unknown.
    ///
    /// # Performance
    ///
    /// This method is `O(definition size)`.
    fn validate_projection_definition(
        &self,
        definition: &ProjectionDefinition,
    ) -> Result<(), DbError> {
        match definition {
            ProjectionDefinition::Graph(graph) => {
                self.require_role(graph.source_role)?;
                self.require_role(graph.target_role)?;
                for relation_type in &graph.relation_types {
                    self.require_relation_type(*relation_type)?;
                }
                Ok(())
            }
            ProjectionDefinition::Hypergraph(hyper) => {
                for role in &hyper.source_roles {
                    self.require_role(*role)?;
                }
                for role in &hyper.target_roles {
                    self.require_role(*role)?;
                }
                for relation_type in &hyper.relation_types {
                    self.require_relation_type(*relation_type)?;
                }
                Ok(())
            }
        }
    }

    /// Validates one index definition against the merged catalog.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when a referenced catalog id is unknown or a
    /// composite index has no keys.
    ///
    /// # Performance
    ///
    /// This method is `O(definition size)`.
    fn validate_index_definition(&self, definition: &IndexDefinition) -> Result<(), DbError> {
        let catalog = self.delta.catalog();
        match definition {
            IndexDefinition::Label { label } => self.require_label(*label),
            IndexDefinition::RelationType { relation_type } => {
                self.require_relation_type(*relation_type)
            }
            IndexDefinition::PropertyEquality { key } | IndexDefinition::PropertyRange { key } => {
                self.require_property_key(*key)
            }
            IndexDefinition::CompositeEquality { keys } => {
                if keys.is_empty() {
                    return Err(DbError::unsupported(
                        "composite equality index requires at least one key",
                    ));
                }
                for key in keys {
                    self.require_property_key(*key)?;
                }
                Ok(())
            }
            IndexDefinition::Projection { projection } => catalog
                .projection(*projection)
                .is_some()
                .then_some(())
                .ok_or(DbError::UnknownProjection { id: *projection }),
        }
    }

    /// Requires a property key to exist in the merged catalog.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`] when absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log property key count)`.
    fn require_property_key(&self, id: PropertyKeyId) -> Result<(), DbError> {
        if self.delta.catalog().property_key(id).is_some() {
            Ok(())
        } else {
            Err(DbError::UnknownPropertyKey { id })
        }
    }
}

#[cfg(test)]
#[cfg(not(miri))]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    /// Per-process path counter for unique temporary store directories.
    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    /// Returns a unique temporary store path and removes any prior contents.
    fn temp_store(name: &str) -> PathBuf {
        let id = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("oxgraph-db-cp-{name}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    /// The exact logical state the crash-matrix asserts recovery preserves: the
    /// visible element ids, the rank-keyed property values, and the `Person`
    /// label membership.
    #[derive(Debug, Eq, PartialEq)]
    struct LogicalState {
        /// Visible element ids in ascending order.
        elements: Vec<ElementId>,
        /// Subjects whose `rank` equals each probed value, by value.
        rank_eq_500: Vec<PropertySubject>,
        /// Element ids carrying the `Person` label.
        person_members: Vec<ElementId>,
    }

    /// Catalog/topology fixture ids returned by [`build_fixture`].
    struct Fixture {
        /// `rank` integer property key.
        rank: PropertyKeyId,
        /// `Person` label.
        person: LabelId,
    }

    /// Builds a committed fixture: 8 elements, each ranked `index * 100`, the
    /// even-indexed ones labelled `Person`. Returns the fixture ids.
    fn build_fixture(database: &mut Database) -> Fixture {
        let mut writer = database.begin_write().expect("begin write");
        let rank = writer
            .register_property_key("rank", PropertyFamily::Element, PropertyType::Integer)
            .expect("rank key");
        let person = writer.register_label("Person").expect("person label");
        for index in 0..8u64 {
            let element = writer.create_element().expect("element");
            writer
                .set_property(
                    PropertySubject::Element(element),
                    rank,
                    PropertyValue::Integer(i64::try_from(index).expect("index") * 100),
                )
                .expect("set rank");
            if index % 2 == 0 {
                writer
                    .add_element_label(element, person)
                    .expect("add label");
            }
        }
        writer.commit().expect("commit fixture");
        Fixture { rank, person }
    }

    /// Reads the logical state through the index-backed read surface.
    fn read_logical(database: &Database, fixture: &Fixture) -> LogicalState {
        let read = database.begin_read();
        let elements = read.element_ids();
        let rank_eq_500 = read
            .lookup_property_equal(fixture.rank, &PropertyValue::Integer(500))
            .expect("rank lookup");
        let person_members = read.snapshot.view().elements_with_label(fixture.person);
        LogicalState {
            elements,
            rank_eq_500,
            person_members,
        }
    }

    /// Asserts ids are never reused across a fold BEHAVIORALLY: the next element
    /// `database` mints must take the id one past the current maximum visible
    /// element id, i.e. the recovered watermark survived the fold. A regression
    /// that dropped the watermark on fold (so the recovered record set is
    /// unchanged but the next-id counter reset) would reuse an existing id and
    /// fail this assertion — which the unchanged-record-set checks alone miss.
    ///
    /// The probe element is rolled back, so it does not perturb the logical state
    /// the surrounding test re-reads.
    fn assert_no_id_reuse_across_fold(database: &mut Database) {
        let max_existing = database
            .begin_read()
            .element_ids()
            .into_iter()
            .map(ElementId::get)
            .max()
            .unwrap_or(0);
        let expected = ElementId::new(max_existing + 1);
        let mut writer = database.begin_write().expect("watermark probe writer");
        let minted = writer.create_element().expect("watermark probe element");
        assert_eq!(
            minted, expected,
            "the next minted id must be one past the max existing id (watermark \
             survived the fold; ids are never reused)",
        );
        // Roll the probe back so it leaves no trace in the logical state.
        writer.rollback();
    }

    /// CHECKPOINT-CRASH-MATRIX: a crash after each fsync point in `checkpoint`
    /// recovers EXACTLY the correct logical state. After a crash before the
    /// superblock lands, the OLD generation stays authoritative (the orphan new
    /// base is ignored); after a crash once the superblock names the new
    /// generation, the NEW base is authoritative. The completed checkpoint
    /// recovers the same logical state from the folded base. In every case the
    /// index-backed lookups return the same answers as before the (attempted)
    /// fold.
    #[test]
    fn checkpoint_crash_matrix_recovers_exact_state() {
        for stop in [
            CheckpointStop::BeforeSuperblock,
            CheckpointStop::BeforeRotate,
            CheckpointStop::Complete,
        ] {
            let path = temp_store(&format!("crash-{stop:?}"));
            let mut database = Database::create(&path).expect("create");
            let fixture = build_fixture(&mut database);
            let before = read_logical(&database, &fixture);
            let before_generation = database.base_generation;

            // Simulate a crash at `stop`: the checkpoint returns right after the
            // chosen fsync, leaving the intermediate files in place. We then drop
            // the handle (as a crash would) and reopen from disk.
            database
                .checkpoint_inner(stop)
                .expect("checkpoint stop returns ok");
            drop(database);

            let mut recovered = Database::open(&path).expect("reopen after crash");
            let after = read_logical(&recovered, &fixture);
            assert_eq!(
                after, before,
                "crash at {stop:?} must recover the exact logical state",
            );

            // The recovered watermark survives every crash window: the next minted
            // id is one past the max recovered element id, so ids are never reused
            // across the (attempted) fold — asserted behaviorally, not merely
            // inferred from the unchanged record set.
            assert_no_id_reuse_across_fold(&mut recovered);

            // Generation expectation per crash window.
            match stop {
                CheckpointStop::BeforeSuperblock => assert_eq!(
                    recovered.base_generation, before_generation,
                    "old superblock stays authoritative before the new one lands",
                ),
                CheckpointStop::BeforeRotate | CheckpointStop::Complete => assert_eq!(
                    recovered.base_generation,
                    before_generation + 1,
                    "the new superblock names the folded generation",
                ),
            }

            // A second open is idempotent (orphan files from a partial crash do
            // not derail a repeat recovery).
            let reopened = Database::open(&path).expect("second reopen");
            assert_eq!(read_logical(&reopened, &fixture), before);

            drop(reopened);
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    /// The auto-checkpoint policy folds the delta-log into a fresh base once the
    /// log outgrows the base by the configured factor: under a tiny factor, a
    /// run of dirty commits advances the live generation (the log was folded),
    /// and the logical state is preserved across the fold. The manual policy
    /// never auto-folds.
    #[test]
    fn auto_checkpoint_policy_folds_when_log_outgrows_base() {
        // Manual policy: many commits, generation never advances on its own.
        let manual_path = temp_store("auto-manual");
        let mut manual = Database::create(&manual_path).expect("create manual");
        manual.set_checkpoint_policy(CheckpointPolicy::Manual);
        let _fixture = build_fixture(&mut manual);
        for _ in 0..200 {
            let mut writer = manual.begin_write().expect("writer");
            writer.create_element().expect("element");
            writer.commit().expect("commit");
        }
        assert_eq!(
            manual.live_generation(),
            CheckpointGeneration::new(0),
            "manual policy must never auto-fold",
        );
        drop(manual);
        let _ = std::fs::remove_dir_all(&manual_path);

        // Size-ratio policy with the smallest factor: the log soon outgrows the
        // tiny base floor, so a run of commits triggers at least one fold.
        let auto_path = temp_store("auto-ratio");
        let mut auto = Database::create(&auto_path).expect("create auto");
        auto.set_checkpoint_policy(CheckpointPolicy::SizeRatio { factor: 1 });
        let fixture = build_fixture(&mut auto);
        let before = read_logical(&auto, &fixture);
        for _ in 0..400 {
            let mut writer = auto.begin_write().expect("writer");
            writer.create_element().expect("element");
            writer.commit().expect("commit");
        }
        assert!(
            auto.live_generation() > CheckpointGeneration::new(0),
            "size-ratio policy must auto-fold once the log outgrows the base",
        );
        // The pre-existing logical state survives every fold; the policy is also
        // surfaced in status and preserved across the fold.
        let after = read_logical(&auto, &fixture);
        assert_eq!(after.rank_eq_500, before.rank_eq_500);
        assert_eq!(after.person_members, before.person_members);
        // Ids are never reused across the auto-fold: the next minted id is one
        // past the max existing id (the watermark folded into the new base).
        assert_no_id_reuse_across_fold(&mut auto);
        assert_eq!(
            auto.checkpoint_policy(),
            CheckpointPolicy::SizeRatio { factor: 1 },
            "the auto-fold reopen must preserve the configured policy",
        );
        // Status surfaces the live generation and the (now small) log size.
        let status = auto.status();
        assert_eq!(status.live_generation, auto.live_generation());
        assert!(status.base_byte_size > 0, "live base has bytes");
        drop(auto);
        let _ = std::fs::remove_dir_all(&auto_path);
    }
}
