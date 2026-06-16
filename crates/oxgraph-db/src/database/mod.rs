//! Embedded `OxGraph` database engine API.
//!
//! This is the integration layer over the base+overlay+WAL core. A [`Db`]
//! holds the current `Arc<Snapshot>` (one immutable base generation plus the
//! frozen overlay published over it), the open append-only delta-log, and the
//! recovered id/transaction watermarks. Reads pin the current snapshot in `O(1)`
//! (`reader` clones the `Arc`); writes layer a fresh [`WriteOverlay`] over
//! the current snapshot, append a WAL frame on commit, and publish a new
//! snapshot. The whole read/query/projection surface resolves through the merged
//! [`StateView`] of the pinned snapshot.

use std::{path::PathBuf, sync::Arc};

use crate::{
    Bound, Catalog, CheckpointGeneration, CommitSeq, DbError, PreparedQuery, PropertyValue, Schema,
    TransactionId,
    lock::WriterLock,
    overlay::{Snapshot, StateView, WriteOverlay},
};

mod maintenance;
mod open;
mod reader;
mod writer;

#[cfg(test)]
#[cfg(not(miri))]
mod tests;

pub use maintenance::CheckpointPolicy;
use open::{base_file, delta_file, file_len};
pub use reader::{HypergraphPageRank, ReadPin, Reader};
pub use writer::Writer;

/// Lookup input for a cataloged index.
///
/// This type makes index lookup shape explicit: membership indexes accept
/// [`IndexProbe::All`], single-property indexes accept scalar equality or
/// range inputs, and composite equality indexes accept an ordered value tuple.
///
/// # Performance
///
/// Copying this value is `O(1)`.
#[derive(Clone, Copy, Debug)]
pub enum IndexProbe<'value> {
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
    Composite(&'value [PropertyValue]),
}

/// The durable result of a [`Db::write`]: whether a frame landed, and at which
/// commit sequence.
///
/// # Performance
///
/// Copying this value is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CommitOutcome {
    /// The transaction made no changes; no WAL frame was appended.
    Empty,
    /// A durable frame landed at this commit sequence.
    Committed(CommitSeq),
}

/// Open OXGDB database handle.
///
/// # Performance
///
/// Moving a handle is `O(1)`: it moves the current `Arc<Snapshot>` and the open
/// delta-log handle.
pub struct Db {
    /// Root database directory.
    pub(super) root: PathBuf,
    /// The current visible snapshot (base generation + published overlay),
    /// shared by readers through an atomically reference-counted handle.
    pub(super) current: Arc<Snapshot>,
    /// Live base generation named by the superblock; every delta frame and the
    /// per-generation log filename carry it.
    pub(super) base_generation: u64,
    /// Last writer transaction id durably recorded (the last dirty commit's id).
    /// A rollback burns a session-local id above this but does not advance it.
    pub(super) last_transaction_id: TransactionId,
    /// Auto-checkpoint policy consulted after each dirty commit.
    pub(super) checkpoint_policy: CheckpointPolicy,
}

impl Db {
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

    /// Returns operational status for this handle, including the live generation
    /// count and the on-disk base/delta-log sizes the auto-checkpoint policy
    /// weighs.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state)` for the merged counts plus two `stat`
    /// syscalls for the file sizes.
    #[must_use]
    pub fn stats(&self) -> Stats {
        let view = self.current.view();
        Stats {
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
    pub fn reader(&self) -> Reader {
        Reader {
            snapshot: Arc::clone(&self.current),
        }
    }

    /// Returns the pin identifying the current visible snapshot — the commit
    /// sequence and checkpoint generation a [`Self::reader`] started now would
    /// observe — without starting a read transaction.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`: it copies the current snapshot's identity fields.
    #[must_use]
    pub fn pin(&self) -> ReadPin {
        ReadPin {
            visible_commit_seq: self.current.lsn(),
            generation: self.current.generation(),
        }
    }

    /// Starts the single writer transaction, acquiring the cross-process writer
    /// lock for the transaction's lifetime.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Txn(crate::error::TxnError::WriterLockHeld)`] when another writer holds
    /// the lock or [`DbError::Txn(crate::error::TxnError::TransactionIdOverflow)`] when writer
    /// ids are exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(parent change)` map entries with `O(1)` per entry:
    /// the writer seeds from the parent's published overlay by cloning its
    /// delta map structure, while label sets, text values, per-subject
    /// property delta maps, and index postings are `Arc`-shared copy-on-write
    /// — it scales with the committed-but-unfolded change count (not the base
    /// size, and not the payload bytes).
    pub(crate) fn begin_write(&mut self) -> Result<Writer<'_>, DbError> {
        let lock = WriterLock::acquire(&self.root)?;
        let transaction_id = self
            .last_transaction_id
            .checked_next()
            .ok_or(DbError::Txn(crate::error::TxnError::TransactionIdOverflow))?;
        // Burn the id eagerly so it is session-local-visible even on rollback;
        // it only becomes durable when a dirty commit writes its frame, and a
        // reopen recovers the durable high-water mark from the log.
        self.last_transaction_id = transaction_id;
        let parent = Arc::clone(&self.current);
        // Seed the writer delta from the parent's published overlay so the
        // writer reads every committed record; the parent overlay is never
        // mutated (the seed clones its maps).
        let delta = WriteOverlay::from_overlay(parent.overlay());
        Ok(Writer {
            database: self,
            parent,
            delta,
            transaction_id,
            lock,
        })
    }

    /// Runs `f` against a read transaction pinned to the current snapshot. The
    /// primary read entry point.
    ///
    /// # Errors
    ///
    /// Propagates whatever error `f` returns.
    ///
    /// # Performance
    ///
    /// Entering is `O(1)` (an `Arc` clone); the total cost is `f`'s cost.
    pub fn read<R>(&self, f: impl FnOnce(&Reader) -> Result<R, DbError>) -> Result<R, DbError> {
        f(&self.reader())
    }

    /// Runs `f` against the single write transaction, committing on `Ok` and
    /// rolling back on `Err` — control flow IS the commit protocol. Returns `f`'s
    /// value with the [`CommitOutcome`] (whether a durable frame landed). The
    /// primary write entry point.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Txn(crate::error::TxnError::WriterLockHeld)`] when another writer holds
    /// the lock, `f`'s error (after rolling back the staged delta), or a commit error.
    ///
    /// # Performance
    ///
    /// Begin is `O(parent change)` — the writer seeds by cloning the parent
    /// overlay's delta map structure (label sets, text values, per-subject
    /// property delta maps, and index postings are `Arc`-shared, so each of
    /// the `N` committed-but-unfolded entries costs `O(1)`; folded away by a
    /// checkpoint). Commit is `O(change)`. A triggered auto-fold adds
    /// `O(visible bytes)`.
    pub fn write<R>(
        &mut self,
        f: impl FnOnce(&mut Writer<'_>) -> Result<R, DbError>,
    ) -> Result<(R, CommitOutcome), DbError> {
        let mut writer = self.begin_write()?;
        // On `Err` the `?` drops `writer` here, releasing the lock and discarding
        // the staged delta — no frame is appended (rollback).
        let value = f(&mut writer)?;
        let committed = !writer.delta.is_empty();
        let lsn = writer.commit()?;
        let outcome = if committed {
            CommitOutcome::Committed(lsn)
        } else {
            CommitOutcome::Empty
        };
        Ok((value, outcome))
    }

    /// Resolves an already-applied [`Schema`] against the live catalog WITHOUT
    /// writing, returning the [`Bound`] handle bag (for a store already
    /// bootstrapped with this schema).
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownName`] when a declared item is absent.
    ///
    /// # Performance
    ///
    /// This method is `O(declared items × log catalog)`.
    pub fn bind(&self, schema: &Schema) -> Result<Bound, DbError> {
        let view = self.current.view();
        let catalog = view.catalog();
        let mut bound = Bound::default();
        for name in &schema.roles {
            let id = catalog.role_id(name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "role",
                    name: name.clone(),
                })
            })?;
            bound.roles.insert(name.clone(), id);
        }
        for name in &schema.labels {
            let id = catalog.label_id(name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "label",
                    name: name.clone(),
                })
            })?;
            bound.labels.insert(name.clone(), id);
        }
        for name in &schema.relation_types {
            let id = catalog.relation_type_id(name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "relation type",
                    name: name.clone(),
                })
            })?;
            bound.relation_types.insert(name.clone(), id);
        }
        for (name, _family, value_type) in &schema.keys {
            let id = catalog.property_key_id(name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "property key",
                    name: name.clone(),
                })
            })?;
            bound.keys.insert(name.clone(), (id, *value_type));
        }
        for (name, key_name) in &schema.equality_indexes {
            let (_key_id, value_type) = *bound.keys.get(key_name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "property key",
                    name: key_name.clone(),
                })
            })?;
            let id = catalog.index_id(name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "index",
                    name: name.clone(),
                })
            })?;
            bound
                .equality_indexes
                .insert(name.clone(), (id, value_type));
        }
        for spec in &schema.graph_projections {
            let id = catalog.projection_id(&spec.name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "projection",
                    name: spec.name.clone(),
                })
            })?;
            bound.projections.insert(spec.name.clone(), id);
        }
        for spec in &schema.hypergraph_projections {
            let id = catalog.projection_id(&spec.name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "projection",
                    name: spec.name.clone(),
                })
            })?;
            bound.projections.insert(spec.name.clone(), id);
        }
        Ok(bound)
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
    pub fn prepare(&self, query: &str) -> Result<PreparedQuery, DbError> {
        PreparedQuery::prepare(query, &self.current.view())
    }
}

/// Snapshot of database status.
///
/// # Performance
///
/// Copying and comparing status is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stats {
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
