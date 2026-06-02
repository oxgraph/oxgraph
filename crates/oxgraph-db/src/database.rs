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
        })
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
    /// This is the P5b checkpoint primitive, exposed here so the existing
    /// `compact` API keeps its "rewrite the store compactly" contract. Auto-
    /// triggering and free-space pre-checks are P5b.
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
    /// # Errors
    ///
    /// Returns [`DbError`] when encoding, writing, or publishing fails.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state bytes)`.
    pub fn checkpoint(&mut self) -> Result<(), DbError> {
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
        // (3) publish the superblock naming g+1 — the linearization point.
        write_superblock(
            &self.root,
            next_generation,
            commit_seq,
            commit_seq,
            self.last_transaction_id.get(),
        )?;
        // Re-open over the new generation, then (4) unlink the old base + log.
        let reopened = Self::open(&self.root)?;
        let old_generation = self.base_generation;
        self.current = reopened.current;
        self.base_generation = reopened.base_generation;
        self.last_transaction_id = reopened.last_transaction_id;
        let _ = std::fs::remove_file(self.root.join(base_file(old_generation)));
        let _ = std::fs::remove_file(self.root.join(delta_file(old_generation)));
        let _ = storage::sync_directory(&self.root);
        Ok(())
    }

    /// Returns operational status for this handle.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state)` for the merged counts.
    #[must_use]
    pub fn status(&self) -> DatabaseStatus {
        let view = self.current.view();
        DatabaseStatus {
            visible_commit_seq: self.current.lsn(),
            last_transaction_id: self.last_transaction_id,
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
    /// Held single-writer advisory lock; its [`Drop`] releases the lock when this
    /// transaction drops (after commit or rollback). The field is an RAII guard
    /// read only via that drop, so the dead-code lint is silenced with a reason
    /// rather than threading a contrived explicit read.
    #[expect(
        dead_code,
        reason = "RAII guard: held only so its Drop releases the advisory writer lock when the transaction ends"
    )]
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
        self.delta.set_property(subject, key, value);
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
        self.delta.remove_property(subject, key);
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
    /// # Errors
    ///
    /// Returns [`DbError`] when commit-sequence allocation, frame encoding, or
    /// the durable append fails.
    ///
    /// # Performance
    ///
    /// This method is `O(change)` for the dirty path.
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
        let snapshot = Snapshot::new(
            self.parent.generation(),
            lsn,
            Arc::clone(self.parent.base()),
            new_overlay,
        )?;
        self.database.current = Arc::new(snapshot);
        self.database.last_transaction_id = self.transaction_id;
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
