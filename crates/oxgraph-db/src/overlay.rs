//! In-memory delta overlay layered over the borrowed base.
//!
//! This is the state model of the new MVCC engine: an owned, change-sized
//! delta ([`Overlay`]) layered over the immutable, borrowed base
//! ([`crate::backing::Base`]), unified for reads by a k-way merge
//! ([`MergedState`]) so a lookup sees the overlay first and falls through to
//! the base. Three layers compose:
//!
//! * [`WriteOverlay`] — the MUTABLE delta a single in-flight write transaction accumulates. It
//!   records creates, tombstones, property sets/removes, and catalog registrations into per-family
//!   maps WITHOUT touching the base, and carries the nine monotonic id allocators (the watermark).
//!   It is private to the writer and never shared.
//! * [`Overlay`] — the FROZEN, published delta: the same data, immutable. [`WriteOverlay::freeze`]
//!   turns the writer's accumulated delta into one. A published `Arc<Overlay>` is immutable
//!   FOREVER; a commit seeds a fresh writer from the parent overlay
//!   ([`WriteOverlay::from_overlay`]), applies the new mutations on top, freezes it, and publishes
//!   a brand-new `Arc<Overlay>` — it NEVER mutates a published overlay. There is no
//!   [`Arc::make_mut`] anywhere on this type.
//! * [`MergedState`] — the borrowed read view that merges an `Overlay` over a base snapshot. Point
//!   reads return [`Cow`]: a base-only id borrows from the base (zero clone, the fast path); an
//!   overlay-supplied or overlay-overridden id is owned by the overlay; a tombstoned id is absent.
//!
//! Canonical ids are NEVER reused: the overlay only adds records or tombstones
//! them (masking a base record), never renumbers, and the watermark is monotonic
//! across every commit (each fresh writer is seeded from the parent's watermark).
//!
//! # Wiring
//!
//! This overlay/merge model IS the live read/write path. The former owned
//! `DatabaseState` is gone: `query.rs`, `projection.rs`, and `database.rs` all
//! read state through [`StateView`] (a [`Snapshot`]'s [`MergedState`], or a
//! writer's [`WriteMergedState`]), and writes accumulate into a [`WriteOverlay`]
//! that a commit freezes into a published [`Overlay`]. The clone-and-apply
//! primitive used to prove the merge laws (`Overlay::with_applied`, gated to
//! `cfg(test)`/`cfg(kani)`) mirrors that commit semantics for the proofs; the
//! live path freezes the seeded writer directly.
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines the overlay/merge primitives. Each
//! item below carries its own contract: overlay point reads and mutators are
//! `O(log change)`; building the next overlay (clone the parent, apply the delta)
//! is `O(parent change + delta change)`; merge iterators are `O(base + overlay
//! change)`.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, btree_map},
    sync::Arc,
};

use zerocopy::byteorder::{LE, U64};

use crate::{
    Catalog, CheckpointGeneration, DbError, ElementId, IncidenceId, IndexId, LabelId, ProjectionId,
    PropertyKeyId, RelationId, RelationTypeId, RoleId,
    backing::{Base, BaseView},
    catalog::{IndexDefinition, ProjectionDefinition, PropertyFamily, PropertyKeyDefinition},
    id::CommitSeq,
    index::{BaseIndex, OverlayIndex},
    state::{ElementRecord, IncidenceRecord, NextIds, PropertySubject, RelationRecord},
    value::{PropertyType, PropertyValue},
    wire::{self, MUTATION_OP_PAYLOAD_WORDS, MutationOp},
};

/// One owned, masked entry in an overlay delta: `Some(record)` adds or overrides
/// the record for that id; `None` is a tombstone that hides the base record (and
/// any earlier overlay record) for that id.
///
/// # Performance
///
/// Copying the variant tag is `O(1)`; cloning a present record is `O(record
/// size)`.
type Delta<R> = BTreeMap<<R as Keyed>::Id, Option<R>>;

/// A record keyed by a canonical id, so the per-family delta maps can name their
/// key type generically.
///
/// # Performance
///
/// [`Self::record_id`] is `O(1)`.
pub(crate) trait Keyed {
    /// Canonical id type that keys this record's delta map.
    type Id: Copy + Ord;

    /// Returns this record's canonical id.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn record_id(&self) -> Self::Id;
}

impl Keyed for ElementRecord {
    type Id = ElementId;

    fn record_id(&self) -> Self::Id {
        self.id
    }
}

impl Keyed for RelationRecord {
    type Id = RelationId;

    fn record_id(&self) -> Self::Id {
        self.id
    }
}

impl Keyed for IncidenceRecord {
    type Id = IncidenceId;

    fn record_id(&self) -> Self::Id {
        self.id
    }
}

/// An ordered, replayable record of every mutation a writer applied, captured
/// AS each mutation happens so the WAL frame replays into a byte-identical
/// overlay. Each entry is a fixed [`MutationOp`]; variable-length names/text are
/// interned into [`Self::blob`] and referenced by `(offset, len)` payload words.
///
/// The writer records ops in two places — into its [`WriteOverlay`] maps (for
/// in-memory reads and the published overlay) and into this log (for the WAL) —
/// and the commit path serializes this log verbatim. Recovery decodes the same
/// ops and re-applies them through the same [`WriteOverlay`] mutators, so the
/// replayed overlay equals the committed one.
///
/// # Performance
///
/// Each push is `O(1)` amortized; interning a name is `O(name.len())`.
#[derive(Clone, Debug, Default)]
pub(crate) struct MutationLog {
    /// Mutation ops in application order.
    ops: Vec<MutationOp>,
    /// Interned UTF-8 names/text referenced by `(offset, len)` payload words.
    blob: Vec<u8>,
}

impl MutationLog {
    /// Returns whether any op has been recorded (a non-dirty writer logs none).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Interns a name/text value into the blob, returning its `(offset, len)` as
    /// `u64` payload words. Both fit `u64` for any in-memory buffer, so this is
    /// infallible; the overall frame `len` is `u32`-checked at append time.
    ///
    /// # Performance
    ///
    /// This method is `O(value.len())`.
    fn intern(&mut self, value: &[u8]) -> (u64, u64) {
        let offset = self.blob.len() as u64;
        self.blob.extend_from_slice(value);
        let len = value.len() as u64;
        (offset, len)
    }

    /// Interns a `u64` definition-body word run into the blob as little-endian
    /// bytes, returning the byte `(offset, len)` of the run.
    ///
    /// # Performance
    ///
    /// This method is `O(words.len())`.
    fn intern_words(&mut self, words: &[u64]) -> (u64, u64) {
        let offset = self.blob.len() as u64;
        for word in words {
            self.blob.extend_from_slice(&word.to_le_bytes());
        }
        let len = size_of_val(words) as u64;
        (offset, len)
    }

    /// Records one op with the given kind, packed flags, and leading payload
    /// words (remaining words zero).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn push(&mut self, op_kind: u32, flags: u32, words: &[u64]) {
        let mut payload = [U64::<LE>::new(0); MUTATION_OP_PAYLOAD_WORDS];
        for (slot, value) in payload.iter_mut().zip(words) {
            *slot = U64::new(*value);
        }
        self.ops.push(MutationOp {
            op_kind: op_kind.into(),
            flags: flags.into(),
            payload,
        });
    }

    /// Appends the nine-value next-id watermark as the final op of a dirty
    /// frame, so recovery restores allocators without recomputing them.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn push_watermark(&mut self, next: NextIds) {
        self.push(
            wire::OP_NEXT_ID_WATERMARK,
            0,
            &[
                next.element.get(),
                next.relation.get(),
                next.incidence.get(),
                next.role.get(),
                next.label.get(),
                next.relation_type.get(),
                next.property_key.get(),
                next.projection.get(),
                next.index.get(),
            ],
        );
    }
}

/// Reads a `(offset, len)` UTF-8 slice from a replay frame blob.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the slice is out of bounds or not UTF-8.
///
/// # Performance
///
/// This function is `O(len)`.
fn blob_str(blob: &[u8], offset: u64, len: u64, lsn: u64) -> Result<String, DbError> {
    let start = usize::try_from(offset).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "blob offset overflow",
    })?;
    let length = usize::try_from(len).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "blob length overflow",
    })?;
    let end = start.checked_add(length).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "blob slice overflow",
    })?;
    let bytes = blob.get(start..end).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "blob slice out of bounds",
    })?;
    core::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_error| DbError::LogCorrupt {
            lsn,
            reason: "blob slice is not UTF-8",
        })
}

/// The MUTABLE delta a single in-flight write transaction accumulates.
///
/// Every mutator records into one of the per-family delta maps or the property
/// delta WITHOUT touching the base; a tombstone is a `None` entry that masks the
/// base record. Catalog registrations append to [`Self::catalog`] and bump the
/// matching id allocator in [`Self::next`]. Allocators are monotonic: an id is
/// drawn from [`Self::next`] and the watermark advances, so a committed id is
/// never reissued.
///
/// This type is private to the writer that owns it; it is frozen into an
/// immutable [`Overlay`] at commit by [`Self::freeze`].
///
/// # Performance
///
/// Construction is `O(1)`; each mutator is `O(log change)`.
#[derive(Clone, Debug)]
pub(crate) struct WriteOverlay {
    /// Element delta: id -> add/override record, or tombstone.
    elements: Delta<ElementRecord>,
    /// Relation delta: id -> add/override record, or tombstone.
    relations: Delta<RelationRecord>,
    /// Incidence delta: id -> add/override record, or tombstone.
    incidences: Delta<IncidenceRecord>,
    /// Property delta: subject -> key -> set value, or remove (tombstone).
    properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>>,
    /// Catalog additions accumulated by this writer (register-only in v1).
    catalog: Catalog,
    /// The nine monotonic id allocators (the watermark) after this delta.
    next: NextIds,
    /// Ordered, replayable log of every mutation this writer applied.
    log: MutationLog,
    /// Incremental membership/equality index deltas this writer maintains, kept
    /// in lockstep with the record/property deltas so a merged lookup is
    /// index-backed (`O(log n + matches)`), not a full scan.
    index: OverlayIndex,
}

impl WriteOverlay {
    /// Creates an empty writer delta whose watermark starts at `next`.
    ///
    /// The watermark is seeded from the parent snapshot's watermark so newly
    /// allocated ids continue monotonically above every id the parent ever
    /// issued, and the catalog seeds from the parent so duplicate-name checks
    /// see the parent's registrations.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`; it moves the seed watermark and catalog in.
    pub(crate) const fn new(next: NextIds, catalog: Catalog) -> Self {
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog,
            next,
            log: MutationLog {
                ops: Vec::new(),
                blob: Vec::new(),
            },
            index: OverlayIndex::new(),
        }
    }

    /// Seeds a fresh writer delta from the parent's published overlay: the
    /// writer starts holding the full committed (but not-yet-checkpointed) state
    /// so its reads see every committed record, while the mutation log starts
    /// EMPTY so the WAL frame records only this writer's new changes.
    ///
    /// The published parent overlay is never mutated; the seeded maps are owned
    /// clones layered over the same base. Commit freezes this delta directly into
    /// a brand-new published `Arc<Overlay>` (parent state + this commit), so a
    /// reader pinning the parent overlay is unaffected.
    ///
    /// # Performance
    ///
    /// This function is `O(parent change)`; it clones the parent's delta maps.
    pub(crate) fn from_overlay(parent: &Overlay) -> Self {
        Self {
            elements: parent.elements.clone(),
            relations: parent.relations.clone(),
            incidences: parent.incidences.clone(),
            properties: parent.properties.clone(),
            catalog: parent.catalog.clone(),
            next: parent.next,
            log: MutationLog {
                ops: Vec::new(),
                blob: Vec::new(),
            },
            // Seed the index deltas from the parent's frozen overlay so the
            // writer's lookups stay index-backed over the full committed delta.
            index: parent.index.clone(),
        }
    }

    /// Returns whether this writer recorded any mutation (a non-dirty writer
    /// has an empty log and commits without appending to the WAL).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// Encodes this writer's mutation log into a WAL frame: the ordered ops with
    /// the nine-value watermark appended as the final op, plus the interned blob.
    ///
    /// # Performance
    ///
    /// This method is `O(op count + blob length)`; it clones the log so the
    /// writer's published-overlay fold can still run.
    pub(crate) fn encode_frame(&self) -> (Vec<MutationOp>, Vec<u8>) {
        let mut log = self.log.clone();
        log.push_watermark(self.next);
        (log.ops, log.blob)
    }

    /// Reads the merged catalog this writer accumulated (parent + additions).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Applies one decoded replay op against `base`, reconstructing the exact
    /// mutation it recorded. Unlike the allocating mutators, creates here take
    /// the EXPLICIT id from the op payload (recovery never reallocates), and the
    /// terminal watermark op sets the allocators directly.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::LogCorrupt`] when an op kind is unknown, a payload tag
    /// is out of range, or a blob slice is out of bounds.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)` plus blob/def decoding for the few
    /// variable-length ops.
    pub(crate) fn apply_replay_op(
        &mut self,
        base: &BaseRecords,
        op: &MutationOp,
        blob: &[u8],
        lsn: u64,
    ) -> Result<(), DbError> {
        let kind = op.op_kind.get();
        let payload = &op.payload;
        let word = |index: usize| payload.get(index).map_or(0, |w| w.get());
        match kind {
            wire::OP_CREATE_ELEMENT
            | wire::OP_CREATE_RELATION
            | wire::OP_CREATE_INCIDENCE
            | wire::OP_TOMBSTONE_ELEMENT
            | wire::OP_TOMBSTONE_RELATION
            | wire::OP_TOMBSTONE_INCIDENCE
            | wire::OP_ADD_ELEMENT_LABEL
            | wire::OP_ADD_RELATION_LABEL
            | wire::OP_SET_RELATION_TYPE => {
                self.apply_topology_op(kind, base, op);
            }
            wire::OP_SET_PROPERTY => {
                self.apply_set_property(base, op, blob, lsn)?;
            }
            wire::OP_REMOVE_PROPERTY => {
                let (subject_kind, _high) = wire::unpack_flags(op.flags.get());
                let subject =
                    wire::decode_subject(subject_kind, word(0)).ok_or(DbError::LogCorrupt {
                        lsn,
                        reason: "remove-property subject kind",
                    })?;
                self.remove_property_inner(base, subject, PropertyKeyId::new(word(1)));
            }
            wire::OP_CATALOG_REGISTER_ROLE
            | wire::OP_CATALOG_REGISTER_LABEL
            | wire::OP_CATALOG_REGISTER_RELATION_TYPE
            | wire::OP_CATALOG_REGISTER_PROPERTY_KEY
            | wire::OP_CATALOG_REGISTER_PROJECTION
            | wire::OP_CATALOG_REGISTER_INDEX => {
                self.apply_catalog_op(kind, op, blob, lsn)?;
            }
            wire::OP_NEXT_ID_WATERMARK => {
                self.next = NextIds {
                    element: ElementId::new(word(0)),
                    relation: RelationId::new(word(1)),
                    incidence: IncidenceId::new(word(2)),
                    role: RoleId::new(word(3)),
                    label: LabelId::new(word(4)),
                    relation_type: RelationTypeId::new(word(5)),
                    property_key: PropertyKeyId::new(word(6)),
                    projection: ProjectionId::new(word(7)),
                    index: IndexId::new(word(8)),
                };
            }
            _other => {
                return Err(DbError::LogCorrupt {
                    lsn,
                    reason: "unknown mutation op kind",
                });
            }
        }
        Ok(())
    }

    /// Applies a decoded topology op (create/tombstone/label/type) against
    /// `base` during replay, reconstructing the recorded mutation directly with
    /// the explicit ids from the op payload.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    fn apply_topology_op(&mut self, kind: u32, base: &BaseRecords, op: &MutationOp) {
        let payload = &op.payload;
        let word = |index: usize| payload.get(index).map_or(0, |w| w.get());
        match kind {
            wire::OP_CREATE_ELEMENT => {
                let id = ElementId::new(word(0));
                self.elements.insert(
                    id,
                    Some(ElementRecord {
                        id,
                        labels: BTreeSet::new(),
                    }),
                );
            }
            wire::OP_CREATE_RELATION => {
                let id = RelationId::new(word(0));
                self.relations.insert(
                    id,
                    Some(RelationRecord {
                        id,
                        relation_type: None,
                        labels: BTreeSet::new(),
                    }),
                );
            }
            wire::OP_CREATE_INCIDENCE => {
                let id = IncidenceId::new(word(0));
                self.incidences.insert(
                    id,
                    Some(IncidenceRecord {
                        id,
                        relation: RelationId::new(word(1)),
                        element: ElementId::new(word(2)),
                        role: RoleId::new(word(3)),
                    }),
                );
            }
            wire::OP_TOMBSTONE_ELEMENT => {
                self.tombstone_element_replay(base, ElementId::new(word(0)));
            }
            wire::OP_TOMBSTONE_RELATION => {
                self.tombstone_relation_replay(base, RelationId::new(word(0)));
            }
            wire::OP_TOMBSTONE_INCIDENCE => {
                self.tombstone_incidence_replay(base, IncidenceId::new(word(0)));
            }
            wire::OP_ADD_ELEMENT_LABEL => {
                self.add_element_label_replay(base, ElementId::new(word(0)), LabelId::new(word(1)));
            }
            wire::OP_ADD_RELATION_LABEL => {
                self.add_relation_label_replay(
                    base,
                    RelationId::new(word(0)),
                    LabelId::new(word(1)),
                );
            }
            // OP_SET_RELATION_TYPE (the only remaining topology kind).
            _set_type => {
                self.set_relation_type_replay(
                    base,
                    RelationId::new(word(0)),
                    RelationTypeId::new(word(1)),
                );
            }
        }
    }

    /// Applies a decoded catalog-register op (role/label/relation-type/
    /// property-key/projection/index) against the blob during replay.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::LogCorrupt`] when a blob slice, tag, or definition body
    /// is malformed, or [`DbError::DuplicateCatalogName`] when the catalog
    /// rejects the registration.
    ///
    /// # Performance
    ///
    /// This method is `O(name length + definition size)`.
    fn apply_catalog_op(
        &mut self,
        kind: u32,
        op: &MutationOp,
        blob: &[u8],
        lsn: u64,
    ) -> Result<(), DbError> {
        let payload = &op.payload;
        let word = |index: usize| payload.get(index).map_or(0, |w| w.get());
        let name = blob_str(blob, word(1), word(2), lsn)?;
        match kind {
            wire::OP_CATALOG_REGISTER_ROLE => self.catalog.insert_role(RoleId::new(word(0)), name),
            wire::OP_CATALOG_REGISTER_LABEL => {
                self.catalog.insert_label(LabelId::new(word(0)), name)
            }
            wire::OP_CATALOG_REGISTER_RELATION_TYPE => self
                .catalog
                .insert_relation_type(RelationTypeId::new(word(0)), name),
            wire::OP_CATALOG_REGISTER_PROPERTY_KEY => {
                let (family_tag, value_tag) = wire::unpack_flags(op.flags.get());
                let family =
                    wire::property_family_from_tag(family_tag).ok_or(DbError::LogCorrupt {
                        lsn,
                        reason: "property-key family tag",
                    })?;
                let value_type =
                    wire::property_type_from_tag(value_tag).ok_or(DbError::LogCorrupt {
                        lsn,
                        reason: "property-key value tag",
                    })?;
                self.catalog.insert_property_key(PropertyKeyDefinition {
                    id: PropertyKeyId::new(word(0)),
                    name,
                    family,
                    value_type,
                })
            }
            wire::OP_CATALOG_REGISTER_PROJECTION => {
                let words = decode_def_words(blob, word(3), word(4), lsn)?;
                let definition = decode_projection_def(op.flags.get(), name, &words, lsn)?;
                self.catalog
                    .insert_projection(ProjectionId::new(word(0)), definition)
            }
            // OP_CATALOG_REGISTER_INDEX (the only remaining catalog kind).
            _index => {
                let words = decode_def_words(blob, word(3), word(4), lsn)?;
                let definition = decode_index_def(op.flags.get(), &words, lsn)?;
                self.catalog
                    .insert_index(IndexId::new(word(0)), name, definition)
            }
        }
    }

    /// Applies a decoded `OP_SET_PROPERTY` op against the blob (replay path).
    ///
    /// # Errors
    ///
    /// Returns [`DbError::LogCorrupt`] when the subject kind or value tag is out
    /// of range or the text slice is out of bounds.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + text length)`.
    fn apply_set_property(
        &mut self,
        base: &BaseRecords,
        op: &MutationOp,
        blob: &[u8],
        lsn: u64,
    ) -> Result<(), DbError> {
        let payload = &op.payload;
        let word = |index: usize| payload.get(index).map_or(0, |w| w.get());
        let (subject_kind, value_tag) = wire::unpack_flags(op.flags.get());
        let subject = wire::decode_subject(subject_kind, word(0)).ok_or(DbError::LogCorrupt {
            lsn,
            reason: "set-property subject kind",
        })?;
        let value_type = wire::property_type_from_tag(value_tag).ok_or(DbError::LogCorrupt {
            lsn,
            reason: "set-property value tag",
        })?;
        let value = match value_type {
            PropertyType::Boolean => PropertyValue::Boolean(word(2) != 0),
            PropertyType::Integer => PropertyValue::Integer(word(2).cast_signed()),
            PropertyType::Text => PropertyValue::Text(blob_str(blob, word(3), word(4), lsn)?),
        };
        self.set_property_inner(base, subject, PropertyKeyId::new(word(1)), value);
        Ok(())
    }

    /// Tombstones an element during replay without re-logging the op.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    fn tombstone_element_replay(&mut self, base: &BaseRecords, id: ElementId) {
        self.tombstone_element_inner(base, id);
    }

    /// Tombstones a relation during replay without re-logging the op.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    fn tombstone_relation_replay(&mut self, base: &BaseRecords, id: RelationId) {
        self.tombstone_relation_inner(base, id);
    }

    /// Tombstones an incidence during replay without re-logging the op.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    fn tombstone_incidence_replay(&mut self, base: &BaseRecords, id: IncidenceId) {
        self.tombstone_incidence_inner(base, id);
    }

    /// Adds an element label during replay without re-logging the op.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + labels)`.
    fn add_element_label_replay(&mut self, base: &BaseRecords, element: ElementId, label: LabelId) {
        self.add_element_label_inner(base, element, label);
    }

    /// Adds a relation label during replay without re-logging the op.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + labels)`.
    fn add_relation_label_replay(
        &mut self,
        base: &BaseRecords,
        relation: RelationId,
        label: LabelId,
    ) {
        match self.relations.entry(relation) {
            btree_map::Entry::Occupied(mut entry) => {
                if let Some(record) = entry.get_mut() {
                    record.labels.insert(label);
                }
            }
            btree_map::Entry::Vacant(entry) => {
                if let Some(record) = base.relation(relation) {
                    let mut record = record.clone();
                    record.labels.insert(label);
                    entry.insert(Some(record));
                }
            }
        }
    }

    /// Sets a relation type during replay without re-logging the op.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)`.
    fn set_relation_type_replay(
        &mut self,
        base: &BaseRecords,
        relation: RelationId,
        relation_type: RelationTypeId,
    ) {
        self.set_relation_type_inner(base, relation, relation_type);
    }

    /// Returns the watermark (the nine monotonic allocators) after this delta.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn next_ids(&self) -> NextIds {
        self.next
    }

    /// Overwrites the watermark to `next` (the recovery path sets the recovered
    /// elementwise-max watermark directly so canonical ids are never reused).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn set_next_ids(&mut self, next: NextIds) {
        self.next = next;
    }

    /// Returns the merged-visible value of `(subject, key)` BEFORE a pending
    /// property mutation: the overlay value when this writer already set it, the
    /// base value when the overlay has not touched it, or `None` when the overlay
    /// removed it or neither layer has it. The index maintainer uses this as the
    /// posting the subject is leaving.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn visible_property(
        &self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<PropertyValue> {
        self.properties
            .get(&subject)
            .and_then(|keys| keys.get(&key))
            .map_or_else(|| base.property(subject, key).cloned(), Clone::clone)
    }

    /// Returns the merged-visible labels of `element` BEFORE a pending mutation
    /// (overlay record wins; a tombstone is empty; otherwise the base labels).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n + labels)`.
    fn visible_element_labels(&self, base: &BaseRecords, element: ElementId) -> BTreeSet<LabelId> {
        match self.elements.get(&element) {
            Some(Some(record)) => record.labels.clone(),
            Some(None) => BTreeSet::new(),
            None => base
                .element(element)
                .map_or_else(BTreeSet::new, |record| record.labels.clone()),
        }
    }

    /// Returns the merged-visible relation type of `relation` BEFORE a pending
    /// mutation (overlay record wins; a tombstone is `None`; otherwise base).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn visible_relation_type(
        &self,
        base: &BaseRecords,
        relation: RelationId,
    ) -> Option<RelationTypeId> {
        match self.relations.get(&relation) {
            Some(Some(record)) => record.relation_type,
            Some(None) => None,
            None => base
                .relation(relation)
                .and_then(|record| record.relation_type),
        }
    }

    /// Returns every merged-visible property of `subject` BEFORE a pending
    /// tombstone, so the index can withdraw each `(key, value)` posting it leaves.
    ///
    /// # Performance
    ///
    /// This method is `O(subject's base + overlay properties)`.
    fn visible_subject_properties(
        &self,
        base: &BaseRecords,
        subject: PropertySubject,
    ) -> BTreeMap<PropertyKeyId, PropertyValue> {
        let mut visible: BTreeMap<PropertyKeyId, PropertyValue> = BTreeMap::new();
        if let Some(keys) = base.properties.get(&subject) {
            for (key, value) in keys {
                visible.insert(*key, value.clone());
            }
        }
        let Some(keys) = self.properties.get(&subject) else {
            return visible;
        };
        for (key, entry) in keys {
            // A set overrides the base value; a removal masks it.
            if let Some(value) = entry {
                visible.insert(*key, value.clone());
            } else {
                visible.remove(key);
            }
        }
        visible
    }

    /// Records a fresh element, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when the element id space is exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)`.
    pub(crate) fn create_element(&mut self) -> Result<ElementId, DbError> {
        let id = self.next.element;
        self.next.element = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.elements.insert(
            id,
            Some(ElementRecord {
                id,
                labels: BTreeSet::new(),
            }),
        );
        self.log.push(wire::OP_CREATE_ELEMENT, 0, &[id.get()]);
        Ok(id)
    }

    /// Records a fresh relation, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when the relation id space is exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)`.
    pub(crate) fn create_relation(&mut self) -> Result<RelationId, DbError> {
        let id = self.next.relation;
        self.next.relation = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.relations.insert(
            id,
            Some(RelationRecord {
                id,
                relation_type: None,
                labels: BTreeSet::new(),
            }),
        );
        self.log.push(wire::OP_CREATE_RELATION, 0, &[id.get()]);
        Ok(id)
    }

    /// Records a fresh incidence, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when the incidence id space is exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)`.
    pub(crate) fn create_incidence(
        &mut self,
        relation: RelationId,
        element: ElementId,
        role: RoleId,
    ) -> Result<IncidenceId, DbError> {
        let id = self.next.incidence;
        self.next.incidence = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.incidences.insert(
            id,
            Some(IncidenceRecord {
                id,
                relation,
                element,
                role,
            }),
        );
        self.log.push(
            wire::OP_CREATE_INCIDENCE,
            0,
            &[id.get(), relation.get(), element.get(), role.get()],
        );
        Ok(id)
    }

    /// Tombstones an element: masks the base/overlay record AND every property
    /// whose subject is that element (a property cannot outlive its subject, so
    /// each of the subject's base properties is masked with a property
    /// tombstone, and any overlay property delta for the subject is dropped).
    /// Idempotent — re-tombstoning an already-tombstoned or absent id records the
    /// same single tombstone.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    pub(crate) fn tombstone_element(&mut self, base: &BaseRecords, id: ElementId) {
        self.tombstone_element_inner(base, id);
        self.log.push(wire::OP_TOMBSTONE_ELEMENT, 0, &[id.get()]);
    }

    /// Tombstones an element, masking its record + properties and withdrawing
    /// every label and property posting it carried from the index. Shared by the
    /// live mutator and the replay path (which does not re-log).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's visible labels + properties)`.
    fn tombstone_element_inner(&mut self, base: &BaseRecords, id: ElementId) {
        let subject = PropertySubject::Element(id);
        let labels = self.visible_element_labels(base, id);
        let properties = self.visible_subject_properties(base, subject);
        self.index.on_tombstone_element(id, &labels, &properties);
        self.elements.insert(id, None);
        self.tombstone_subject_properties(base, subject);
    }

    /// Tombstones a relation, masking its record and its properties. Idempotent.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    pub(crate) fn tombstone_relation(&mut self, base: &BaseRecords, id: RelationId) {
        self.tombstone_relation_inner(base, id);
        self.log.push(wire::OP_TOMBSTONE_RELATION, 0, &[id.get()]);
    }

    /// Tombstones a relation, masking its record + properties and withdrawing its
    /// relation-type and property postings from the index. Shared by the live
    /// mutator and the replay path.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's properties)`.
    fn tombstone_relation_inner(&mut self, base: &BaseRecords, id: RelationId) {
        let subject = PropertySubject::Relation(id);
        let relation_type = self.visible_relation_type(base, id);
        let properties = self.visible_subject_properties(base, subject);
        self.index
            .on_tombstone_relation(id, relation_type, &properties);
        self.relations.insert(id, None);
        self.tombstone_subject_properties(base, subject);
    }

    /// Tombstones an incidence, masking its record and its properties.
    /// Idempotent.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    pub(crate) fn tombstone_incidence(&mut self, base: &BaseRecords, id: IncidenceId) {
        self.tombstone_incidence_inner(base, id);
        self.log.push(wire::OP_TOMBSTONE_INCIDENCE, 0, &[id.get()]);
    }

    /// Tombstones an incidence, masking its record + properties and withdrawing
    /// its property postings from the index. Shared by the live mutator and the
    /// replay path.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's properties)`.
    fn tombstone_incidence_inner(&mut self, base: &BaseRecords, id: IncidenceId) {
        let subject = PropertySubject::Incidence(id);
        let properties = self.visible_subject_properties(base, subject);
        self.index.on_tombstone_incidence(subject, &properties);
        self.incidences.insert(id, None);
        self.tombstone_subject_properties(base, subject);
    }

    /// Masks every property of `subject`: drops the writer's accumulated property
    /// delta for the subject, then records a property tombstone for each base
    /// property of the subject so the merge hides it.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + subject's base properties)`.
    fn tombstone_subject_properties(&mut self, base: &BaseRecords, subject: PropertySubject) {
        self.properties.remove(&subject);
        if let Some(keys) = base.properties.get(&subject) {
            let entry = self.properties.entry(subject).or_default();
            for key in keys.keys() {
                entry.insert(*key, None);
            }
        }
    }

    /// Adds a label to an element's overlay record, materializing the base
    /// record into the overlay first when the element is not already present.
    ///
    /// `base` supplies the element's current label set when this writer has not
    /// yet shadowed it. A tombstoned element is left tombstoned (the label is a
    /// no-op against a deleted element).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + labels)`.
    pub(crate) fn add_element_label(
        &mut self,
        base: &BaseRecords,
        element: ElementId,
        label: LabelId,
    ) {
        self.add_element_label_inner(base, element, label);
        self.log
            .push(wire::OP_ADD_ELEMENT_LABEL, 0, &[element.get(), label.get()]);
    }

    /// Adds an element label to the overlay record and (when the record actually
    /// receives the label) to the label index. Shared by the live mutator and
    /// the replay path (which does not re-log). A tombstoned element is left
    /// tombstoned and the index is untouched.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + labels)`.
    fn add_element_label_inner(&mut self, base: &BaseRecords, element: ElementId, label: LabelId) {
        let mut labelled = false;
        match self.elements.entry(element) {
            btree_map::Entry::Occupied(mut entry) => {
                if let Some(record) = entry.get_mut() {
                    record.labels.insert(label);
                    labelled = true;
                }
            }
            btree_map::Entry::Vacant(entry) => {
                if let Some(record) = base.element(element) {
                    let mut record = record.clone();
                    record.labels.insert(label);
                    entry.insert(Some(record));
                    labelled = true;
                }
            }
        }
        if labelled {
            self.index.on_add_element_label(element, label);
        }
    }

    /// Adds a label to a relation's overlay record, materializing the base
    /// record first when needed. A tombstoned relation is left tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + labels)`.
    pub(crate) fn add_relation_label(
        &mut self,
        base: &BaseRecords,
        relation: RelationId,
        label: LabelId,
    ) {
        match self.relations.entry(relation) {
            btree_map::Entry::Occupied(mut entry) => {
                if let Some(record) = entry.get_mut() {
                    record.labels.insert(label);
                }
            }
            btree_map::Entry::Vacant(entry) => {
                if let Some(record) = base.relation(relation) {
                    let mut record = record.clone();
                    record.labels.insert(label);
                    entry.insert(Some(record));
                }
            }
        }
        self.log.push(
            wire::OP_ADD_RELATION_LABEL,
            0,
            &[relation.get(), label.get()],
        );
    }

    /// Sets a relation's type in its overlay record, materializing the base
    /// record first when needed. A tombstoned relation is left tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + labels)`.
    pub(crate) fn set_relation_type(
        &mut self,
        base: &BaseRecords,
        relation: RelationId,
        relation_type: RelationTypeId,
    ) {
        self.set_relation_type_inner(base, relation, relation_type);
        self.log.push(
            wire::OP_SET_RELATION_TYPE,
            0,
            &[relation.get(), relation_type.get()],
        );
    }

    /// Sets a relation's type in its overlay record and (when the record
    /// actually receives the type) updates the relation-type index, withdrawing
    /// the previous type's posting. Shared by the live mutator and the replay
    /// path (which does not re-log). A tombstoned relation is left tombstoned and
    /// the index is untouched.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)`.
    fn set_relation_type_inner(
        &mut self,
        base: &BaseRecords,
        relation: RelationId,
        relation_type: RelationTypeId,
    ) {
        let previous = self.visible_relation_type(base, relation);
        let mut typed = false;
        match self.relations.entry(relation) {
            btree_map::Entry::Occupied(mut entry) => {
                if let Some(record) = entry.get_mut() {
                    record.relation_type = Some(relation_type);
                    typed = true;
                }
            }
            btree_map::Entry::Vacant(entry) => {
                if let Some(record) = base.relation(relation) {
                    let mut record = record.clone();
                    record.relation_type = Some(relation_type);
                    entry.insert(Some(record));
                    typed = true;
                }
            }
        }
        if typed {
            self.index
                .on_set_relation_type(relation, previous, relation_type);
        }
    }

    /// Records a property set: subject's key maps to `Some(value)`, overriding
    /// any base/overlay value for that pair.
    ///
    /// # Phase note — UNVALIDATED delta; orphan properties must be rejected in P4
    ///
    /// The property layer and the record layer are independent maps, so this
    /// records a visible property even when the same writer has tombstoned the
    /// subject's element/relation/incidence (or never created it): the property
    /// merge and the record merge do not consult each other. This is STRICTLY
    /// more permissive than the type it replaces — the live
    /// the former owned-state `set_property` gates on `require_subject`
    /// and rejects a property whose subject is absent.
    ///
    /// This is acceptable here because the overlay is an unvalidated delta;
    /// referential-integrity validation lives in the commit / write-transaction
    /// layer (P4). P4's `WriteTransaction` (or commit-time `validate()`) MUST
    /// reject a `set_property` against a subject tombstoned earlier in the same
    /// transaction, and MUST carry a test asserting that orphan-property case is
    /// rejected at the transaction boundary, so this permissiveness does not leak
    /// into the live path.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn set_property(
        &mut self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: PropertyValue,
    ) {
        let (subject_kind, subject_id) = wire::encode_subject(subject);
        let value_tag = wire::property_type_tag(value.value_type());
        let (scalar, text_off, text_len) = match &value {
            PropertyValue::Boolean(flag) => (u64::from(*flag), 0, 0),
            PropertyValue::Integer(number) => ((*number).cast_unsigned(), 0, 0),
            PropertyValue::Text(string) => {
                let (off, len) = self.log.intern(string.as_bytes());
                (0, off, len)
            }
        };
        self.log.push(
            wire::OP_SET_PROPERTY,
            wire::pack_flags(subject_kind, value_tag),
            &[subject_id, key.get(), scalar, text_off, text_len],
        );
        self.set_property_inner(base, subject, key, value);
    }

    /// Applies a property set to the property delta and the equality index,
    /// withdrawing the subject's previous value posting. Shared by the live
    /// mutator and the replay path (which does not re-log).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn set_property_inner(
        &mut self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: PropertyValue,
    ) {
        let previous = self.visible_property(base, subject, key);
        self.index
            .on_set_property(subject, key, previous.as_ref(), &value);
        self.properties
            .entry(subject)
            .or_default()
            .insert(key, Some(value));
    }

    /// Records a property removal: subject's key maps to `None` (a tombstone
    /// masking the base value). Idempotent.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn remove_property(
        &mut self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) {
        let (subject_kind, subject_id) = wire::encode_subject(subject);
        self.log.push(
            wire::OP_REMOVE_PROPERTY,
            wire::pack_flags(subject_kind, 0),
            &[subject_id, key.get()],
        );
        self.remove_property_inner(base, subject, key);
    }

    /// Applies a property removal to the property delta and the equality index,
    /// withdrawing the subject's previous value posting. Shared by the live
    /// mutator and the replay path (which does not re-log).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn remove_property_inner(
        &mut self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) {
        let previous = self.visible_property(base, subject, key);
        self.index
            .on_remove_property(subject, key, previous.as_ref());
        self.properties
            .entry(subject)
            .or_default()
            .insert(key, None);
    }

    /// Registers a role, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when the role id space is exhausted or
    /// [`DbError::DuplicateCatalogName`] when the name is taken.
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    pub(crate) fn register_role(&mut self, name: String) -> Result<RoleId, DbError> {
        let id = self.next.role;
        self.next.role = id.checked_next().ok_or(DbError::IdOverflow)?;
        let (name_off, name_len) = self.log.intern(name.as_bytes());
        self.catalog.insert_role(id, name)?;
        self.log.push(
            wire::OP_CATALOG_REGISTER_ROLE,
            0,
            &[id.get(), name_off, name_len],
        );
        Ok(id)
    }

    /// Registers a label, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] or [`DbError::DuplicateCatalogName`].
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    pub(crate) fn register_label(&mut self, name: String) -> Result<LabelId, DbError> {
        let id = self.next.label;
        self.next.label = id.checked_next().ok_or(DbError::IdOverflow)?;
        let (name_off, name_len) = self.log.intern(name.as_bytes());
        self.catalog.insert_label(id, name)?;
        self.log.push(
            wire::OP_CATALOG_REGISTER_LABEL,
            0,
            &[id.get(), name_off, name_len],
        );
        Ok(id)
    }

    /// Registers a relation type, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] or [`DbError::DuplicateCatalogName`].
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    pub(crate) fn register_relation_type(
        &mut self,
        name: String,
    ) -> Result<RelationTypeId, DbError> {
        let id = self.next.relation_type;
        self.next.relation_type = id.checked_next().ok_or(DbError::IdOverflow)?;
        let (name_off, name_len) = self.log.intern(name.as_bytes());
        self.catalog.insert_relation_type(id, name)?;
        self.log.push(
            wire::OP_CATALOG_REGISTER_RELATION_TYPE,
            0,
            &[id.get(), name_off, name_len],
        );
        Ok(id)
    }

    /// Registers a typed property key, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] or [`DbError::DuplicateCatalogName`].
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    pub(crate) fn register_property_key(
        &mut self,
        name: String,
        family: PropertyFamily,
        value_type: PropertyType,
    ) -> Result<PropertyKeyId, DbError> {
        let id = self.next.property_key;
        self.next.property_key = id.checked_next().ok_or(DbError::IdOverflow)?;
        let (name_off, name_len) = self.log.intern(name.as_bytes());
        self.catalog.insert_property_key(PropertyKeyDefinition {
            id,
            name,
            family,
            value_type,
        })?;
        self.log.push(
            wire::OP_CATALOG_REGISTER_PROPERTY_KEY,
            wire::pack_flags(
                wire::property_family_tag(family),
                wire::property_type_tag(value_type),
            ),
            &[id.get(), name_off, name_len],
        );
        Ok(id)
    }

    /// Registers a projection definition, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] or [`DbError::DuplicateCatalogName`].
    ///
    /// # Performance
    ///
    /// This method is `O(definition size)`.
    pub(crate) fn register_projection(
        &mut self,
        definition: ProjectionDefinition,
    ) -> Result<ProjectionId, DbError> {
        let id = self.next.projection;
        self.next.projection = id.checked_next().ok_or(DbError::IdOverflow)?;
        let (name_off, name_len) = self.log.intern(definition.name().as_bytes());
        let (kind, words) = encode_projection_words(&definition);
        let (def_off, def_len) = self.log.intern_words(&words);
        self.catalog.insert_projection(id, definition)?;
        self.log.push(
            wire::OP_CATALOG_REGISTER_PROJECTION,
            kind,
            &[id.get(), name_off, name_len, def_off, def_len],
        );
        Ok(id)
    }

    /// Registers an index definition, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] or [`DbError::DuplicateCatalogName`].
    ///
    /// # Performance
    ///
    /// This method is `O(definition size)`.
    pub(crate) fn register_index(
        &mut self,
        name: String,
        definition: IndexDefinition,
    ) -> Result<IndexId, DbError> {
        let id = self.next.index;
        self.next.index = id.checked_next().ok_or(DbError::IdOverflow)?;
        let (name_off, name_len) = self.log.intern(name.as_bytes());
        let (kind, words) = encode_index_words(&definition);
        let (def_off, def_len) = self.log.intern_words(&words);
        self.catalog.insert_index(id, name, definition)?;
        self.log.push(
            wire::OP_CATALOG_REGISTER_INDEX,
            kind,
            &[id.get(), name_off, name_len, def_off, def_len],
        );
        Ok(id)
    }

    /// Freezes this writer delta into an immutable published [`Overlay`].
    ///
    /// This is the only transition from the mutable layer to the published one;
    /// it consumes the writer and moves its maps in (no clone).
    ///
    /// # Performance
    ///
    /// This function is `O(1)`; it moves the delta maps.
    pub(crate) fn freeze(self) -> Overlay {
        Overlay {
            elements: self.elements,
            relations: self.relations,
            incidences: self.incidences,
            properties: self.properties,
            catalog: self.catalog,
            next: self.next,
            index: self.index,
        }
    }
}

/// Definition-body discriminant for a binary graph projection (stamped in the
/// `OP_CATALOG_REGISTER_PROJECTION` op `flags`).
const DEF_PROJECTION_GRAPH: u32 = 0;
/// Definition-body discriminant for a hypergraph projection.
const DEF_PROJECTION_HYPER: u32 = 1;
/// Definition-body discriminant for a label membership index.
const DEF_INDEX_LABEL: u32 = 0;
/// Definition-body discriminant for a relation-type membership index.
const DEF_INDEX_RELATION_TYPE: u32 = 1;
/// Definition-body discriminant for a single-key equality index.
const DEF_INDEX_PROPERTY_EQUALITY: u32 = 2;
/// Definition-body discriminant for a single-key range index.
const DEF_INDEX_PROPERTY_RANGE: u32 = 3;
/// Definition-body discriminant for a composite equality index.
const DEF_INDEX_COMPOSITE_EQUALITY: u32 = 4;
/// Definition-body discriminant for a projection-materialization index.
const DEF_INDEX_PROJECTION: u32 = 5;

/// Pushes a length-prefixed id set into a definition-body word run.
///
/// # Performance
///
/// This function is `O(set size)`.
fn push_id_set(words: &mut Vec<u64>, ids: impl ExactSizeIterator<Item = u64>) {
    words.push(ids.len() as u64);
    words.extend(ids);
}

/// Encodes a projection definition body into a `u64` word run, returning the
/// `(discriminant, words)` the WAL op records.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn encode_projection_words(definition: &ProjectionDefinition) -> (u32, Vec<u64>) {
    let mut words = Vec::new();
    match definition {
        ProjectionDefinition::Graph(graph) => {
            words.push(graph.source_role.get());
            words.push(graph.target_role.get());
            push_id_set(&mut words, graph.relation_types.iter().map(|id| id.get()));
            (DEF_PROJECTION_GRAPH, words)
        }
        ProjectionDefinition::Hypergraph(hyper) => {
            push_id_set(&mut words, hyper.source_roles.iter().map(|id| id.get()));
            push_id_set(&mut words, hyper.target_roles.iter().map(|id| id.get()));
            push_id_set(&mut words, hyper.relation_types.iter().map(|id| id.get()));
            (DEF_PROJECTION_HYPER, words)
        }
    }
}

/// Encodes an index definition body into a `u64` word run, returning the
/// `(discriminant, words)` the WAL op records.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn encode_index_words(definition: &IndexDefinition) -> (u32, Vec<u64>) {
    match definition {
        IndexDefinition::Label { label } => (DEF_INDEX_LABEL, vec![label.get()]),
        IndexDefinition::RelationType { relation_type } => {
            (DEF_INDEX_RELATION_TYPE, vec![relation_type.get()])
        }
        IndexDefinition::PropertyEquality { key } => (DEF_INDEX_PROPERTY_EQUALITY, vec![key.get()]),
        IndexDefinition::PropertyRange { key } => (DEF_INDEX_PROPERTY_RANGE, vec![key.get()]),
        IndexDefinition::CompositeEquality { keys } => (
            DEF_INDEX_COMPOSITE_EQUALITY,
            keys.iter().map(|key| key.get()).collect(),
        ),
        IndexDefinition::Projection { projection } => {
            (DEF_INDEX_PROJECTION, vec![projection.get()])
        }
    }
}

/// Decodes the `(offset, len)` byte run of a definition body into its `u64`
/// words.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the run is out of bounds or not a whole
/// number of `u64` words.
///
/// # Performance
///
/// This function is `O(len)`.
fn decode_def_words(blob: &[u8], offset: u64, len: u64, lsn: u64) -> Result<Vec<u64>, DbError> {
    let start = usize::try_from(offset).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "def offset overflow",
    })?;
    let length = usize::try_from(len).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "def length overflow",
    })?;
    let end = start.checked_add(length).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def slice overflow",
    })?;
    let bytes = blob.get(start..end).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def slice out of bounds",
    })?;
    if !bytes.len().is_multiple_of(size_of::<u64>()) {
        return Err(DbError::LogCorrupt {
            lsn,
            reason: "def slice is not whole u64 words",
        });
    }
    Ok(bytes
        .chunks_exact(size_of::<u64>())
        .map(|chunk| {
            let mut word = [0u8; 8];
            word.copy_from_slice(chunk);
            u64::from_le_bytes(word)
        })
        .collect())
}

/// Reads a length-prefixed id set from a definition-body word run at `cursor`,
/// advancing it past the set.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the length or slice is out of bounds.
///
/// # Performance
///
/// This function is `O(set size)`.
fn read_id_set(words: &[u64], cursor: &mut usize, lsn: u64) -> Result<Vec<u64>, DbError> {
    let count = usize::try_from(*words.get(*cursor).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def missing id-set length",
    })?)
    .map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "def id-set length overflow",
    })?;
    *cursor += 1;
    let end = cursor.checked_add(count).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def id-set overflow",
    })?;
    let slice = words.get(*cursor..end).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def id-set out of bounds",
    })?;
    let ids = slice.to_vec();
    *cursor = end;
    Ok(ids)
}

/// Decodes a projection definition from a replay op's `flags` discriminant,
/// name, and body words.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the discriminant is unknown or the body
/// is malformed.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn decode_projection_def(
    discriminant: u32,
    name: String,
    words: &[u64],
    lsn: u64,
) -> Result<ProjectionDefinition, DbError> {
    match discriminant {
        DEF_PROJECTION_GRAPH => {
            let source_role = *words.first().ok_or(DbError::LogCorrupt {
                lsn,
                reason: "graph def missing source role",
            })?;
            let target_role = *words.get(1).ok_or(DbError::LogCorrupt {
                lsn,
                reason: "graph def missing target role",
            })?;
            let mut cursor = 2;
            let relation_types = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Graph(
                crate::catalog::GraphProjectionDefinition {
                    name,
                    relation_types,
                    source_role: RoleId::new(source_role),
                    target_role: RoleId::new(target_role),
                },
            ))
        }
        DEF_PROJECTION_HYPER => {
            let mut cursor = 0;
            let source_roles = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let target_roles = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let relation_types = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Hypergraph(
                crate::catalog::HypergraphProjectionDefinition {
                    name,
                    relation_types,
                    source_roles,
                    target_roles,
                },
            ))
        }
        _other => Err(DbError::LogCorrupt {
            lsn,
            reason: "unknown projection definition kind",
        }),
    }
}

/// Decodes an index definition from a replay op's `flags` discriminant and body
/// words.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the discriminant is unknown or the body
/// is malformed.
///
/// # Performance
///
/// This function is `O(definition size)`.
fn decode_index_def(
    discriminant: u32,
    words: &[u64],
    lsn: u64,
) -> Result<IndexDefinition, DbError> {
    let first = || {
        words.first().copied().ok_or(DbError::LogCorrupt {
            lsn,
            reason: "index def missing id",
        })
    };
    match discriminant {
        DEF_INDEX_LABEL => Ok(IndexDefinition::Label {
            label: LabelId::new(first()?),
        }),
        DEF_INDEX_RELATION_TYPE => Ok(IndexDefinition::RelationType {
            relation_type: RelationTypeId::new(first()?),
        }),
        DEF_INDEX_PROPERTY_EQUALITY => Ok(IndexDefinition::PropertyEquality {
            key: PropertyKeyId::new(first()?),
        }),
        DEF_INDEX_PROPERTY_RANGE => Ok(IndexDefinition::PropertyRange {
            key: PropertyKeyId::new(first()?),
        }),
        DEF_INDEX_COMPOSITE_EQUALITY => Ok(IndexDefinition::CompositeEquality {
            keys: words.iter().map(|word| PropertyKeyId::new(*word)).collect(),
        }),
        DEF_INDEX_PROJECTION => Ok(IndexDefinition::Projection {
            projection: ProjectionId::new(first()?),
        }),
        _other => Err(DbError::LogCorrupt {
            lsn,
            reason: "unknown index definition kind",
        }),
    }
}

/// The FROZEN, published delta: an immutable [`Overlay`] shared via `Arc`.
///
/// A published `Arc<Overlay>` is immutable forever. To advance state, a commit
/// seeds a fresh writer from the parent ([`WriteOverlay::from_overlay`]), applies
/// the new mutations on top, freezes the result with [`WriteOverlay::freeze`],
/// and publishes a brand-new `Arc<Overlay>`; it NEVER calls [`Arc::make_mut`] on,
/// nor interior-mutates, a published overlay. (The `cfg(test)`/`cfg(kani)`
/// `with_applied` helper proves that same clone-the-parent, apply-the-delta
/// semantics for the merge laws.) A reader that pinned an `Arc<Overlay>`
/// therefore observes a fixed delta for the lifetime of its pin.
///
/// # Performance
///
/// Cloning is `O(change)` (it deep-clones the delta maps); the accessors below
/// are `O(log change)` or `O(change)` as documented.
#[derive(Clone, Debug)]
pub(crate) struct Overlay {
    /// Element delta: id -> add/override record, or tombstone.
    elements: Delta<ElementRecord>,
    /// Relation delta: id -> add/override record, or tombstone.
    relations: Delta<RelationRecord>,
    /// Incidence delta: id -> add/override record, or tombstone.
    incidences: Delta<IncidenceRecord>,
    /// Property delta: subject -> key -> set value, or remove (tombstone).
    properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>>,
    /// Catalog additions folded into this published overlay.
    catalog: Catalog,
    /// The nine monotonic id allocators (the watermark) for this overlay.
    next: NextIds,
    /// Frozen incremental index deltas this overlay carries, so a reader's
    /// merged lookup over `(base index, overlay index)` is index-backed.
    index: OverlayIndex,
}

impl Overlay {
    /// Creates the empty overlay for a freshly opened base: no deltas, the
    /// base's catalog, and the base's watermark.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`; it moves the watermark and catalog in.
    pub(crate) const fn empty(next: NextIds, catalog: Catalog) -> Self {
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog,
            next,
            index: OverlayIndex::new(),
        }
    }

    /// Returns the watermark (the nine monotonic allocators) for this overlay.
    ///
    /// The live commit path seeds a writer from this overlay (via
    /// [`WriteOverlay::from_overlay`]) rather than reading the watermark out
    /// here, so this accessor is exercised by the merge-law proofs and tests.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[cfg(any(test, kani))]
    pub(crate) const fn next_ids(&self) -> NextIds {
        self.next
    }

    /// Builds the NEXT published overlay by cloning this one (the parent) and
    /// applying `delta` on top, layer by layer: a `delta` entry (record,
    /// tombstone, property set/remove) overrides the same key from the parent;
    /// the watermark and catalog are taken from `delta`. The parent is left
    /// untouched — this NEVER mutates a published overlay.
    ///
    /// The live commit path instead seeds the writer from the parent overlay
    /// ([`WriteOverlay::from_overlay`]) and freezes the seeded delta directly, so
    /// this clone-and-apply primitive is exercised by the merge-law proofs and
    /// tests (it proves the same parent-untouched, watermark-monotone contract).
    ///
    /// # Performance
    ///
    /// This function is `O(parent change + delta change)`.
    #[cfg(any(test, kani))]
    pub(crate) fn with_applied(&self, delta: &WriteOverlay) -> Self {
        let mut next = self.clone();
        for (id, entry) in &delta.elements {
            next.elements.insert(*id, entry.clone());
        }
        for (id, entry) in &delta.relations {
            next.relations.insert(*id, entry.clone());
        }
        for (id, entry) in &delta.incidences {
            next.incidences.insert(*id, *entry);
        }
        for (subject, keys) in &delta.properties {
            let merged = next.properties.entry(*subject).or_default();
            for (key, value) in keys {
                merged.insert(*key, value.clone());
            }
        }
        next.catalog = delta.catalog.clone();
        next.next = delta.next;
        // The delta carries the full composed index (seeded from this parent's
        // index, then mutated), matching `freeze`/`from_overlay` on the live
        // path, so the published overlay takes the delta's index verbatim.
        next.index = delta.index.clone();
        next
    }
}

/// Owned realization of one base generation's records, materialized once from a
/// borrowed [`BaseView`] so the merge has a stable `&ElementRecord` to borrow
/// for the [`Cow::Borrowed`] fast path.
///
/// The borrowed [`BaseView`] holds wire records (`ElementWire` with a label
/// offset/length), not owned [`ElementRecord`] values, so a base point read
/// cannot hand out `&ElementRecord` directly. This type owns the decoded records
/// once; [`MergedState`] then borrows from it for base-only ids (zero clone per
/// read).
///
/// # Phase note — stand-in; carries a standing ~2x base memory cost until P5
///
/// This owned realization is the P3 stand-in for the borrowed-from-wire record
/// accessor the reconciled design targets (a future `ElementRecord` reshaped
/// into a label-run-borrowing view). The merge logic, the [`Cow`] contract, and
/// the tests are identical either way; only the base record source changes.
///
/// The cost is real and per generation: [`Self::from_view`] duplicates the base
/// bytes the Yoke already holds zero-copy into owned `BTreeMap`s, so the standing
/// memory is ~2x the base (wire bytes + decoded records) for the life of every
/// pinned reader. It is built once per checkpoint generation (at [`Snapshot::new`]
/// — `open`/`create`/fold) and `Arc`-shared across every commit that pins that
/// generation via [`Snapshot::with_shared_base_records`], so a commit re-decodes
/// neither the base nor its index and stays `O(change)`. The build itself is a
/// full `O(base records + labels + property bytes)` that gates fold/open latency.
/// The roadmapped reshape (P5) removes [`BaseRecords`] entirely so this 2x base
/// memory is not load-bearing once readers pin many generations.
///
/// # Performance
///
/// [`Self::from_view`] is `O(base records + labels + property bytes)`; every
/// accessor below is `O(log n)` or `O(1)`.
#[derive(Clone, Debug)]
pub(crate) struct BaseRecords {
    /// Decoded elements keyed by id, ascending.
    elements: BTreeMap<ElementId, ElementRecord>,
    /// Decoded relations keyed by id, ascending.
    relations: BTreeMap<RelationId, RelationRecord>,
    /// Decoded incidences keyed by id, ascending.
    incidences: BTreeMap<IncidenceId, IncidenceRecord>,
    /// Decoded properties keyed by subject then key.
    properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>,
    /// Derived membership/equality postings for this generation, built once in
    /// RAM from the records above so a merged lookup is index-backed.
    index: BaseIndex,
}

impl BaseRecords {
    /// Builds the records of an empty base (the gen-0 base `create` writes): no
    /// elements, relations, incidences, properties, or index postings.
    ///
    /// This constructor is infallible: it allocates only empty maps and never
    /// decodes wire bytes.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn empty() -> Self {
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            index: BaseIndex::empty(),
        }
    }

    /// Materializes the owned records of `view`, decoding the wire arrays once.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidStore`] when a record references a label run,
    /// property text, or subject kind the wire layout cannot resolve.
    ///
    /// # Performance
    ///
    /// This function is `O(base records + labels + property bytes)`.
    pub(crate) fn from_view(view: &BaseView<'_>) -> Result<Self, DbError> {
        let mut elements = BTreeMap::new();
        for record in view.elements() {
            let labels = decode_labels(view.element_label_run(record))?;
            let id = ElementId::new(record.id.get());
            elements.insert(id, ElementRecord { id, labels });
        }
        let mut relations = BTreeMap::new();
        for record in view.relations() {
            let labels = decode_labels(view.relation_label_run(record))?;
            let id = RelationId::new(record.id.get());
            relations.insert(
                id,
                RelationRecord {
                    id,
                    relation_type: crate::wire::decode_relation_type(record.relation_type.get()),
                    labels,
                },
            );
        }
        let mut incidences = BTreeMap::new();
        for record in view.incidences() {
            let id = IncidenceId::new(record.id.get());
            incidences.insert(
                id,
                IncidenceRecord {
                    id,
                    relation: RelationId::new(record.relation.get()),
                    element: ElementId::new(record.element.get()),
                    role: RoleId::new(record.role.get()),
                },
            );
        }
        let properties = decode_base_properties(view)?;
        let index = BaseIndex::from_records(&elements, &relations, &properties);
        Ok(Self {
            elements,
            relations,
            incidences,
            properties,
            index,
        })
    }

    /// Returns this generation's derived index postings (membership + equality).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn index(&self) -> &BaseIndex {
        &self.index
    }

    /// Returns the base element record for `id`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn element(&self, id: ElementId) -> Option<&ElementRecord> {
        self.elements.get(&id)
    }

    /// Returns the base relation record for `id`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn relation(&self, id: RelationId) -> Option<&RelationRecord> {
        self.relations.get(&id)
    }

    /// Returns the base incidence record for `id`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn incidence(&self, id: IncidenceId) -> Option<&IncidenceRecord> {
        self.incidences.get(&id)
    }

    /// Returns the base property value for `(subject, key)`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn property(&self, subject: PropertySubject, key: PropertyKeyId) -> Option<&PropertyValue> {
        self.properties
            .get(&subject)
            .and_then(|keys| keys.get(&key))
    }
}

/// Decodes a borrowed label run into an owned label set.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the run slice is out of bounds.
///
/// # Performance
///
/// This function is `O(labels)`.
fn decode_labels(
    run: Option<&[zerocopy::byteorder::U64<zerocopy::byteorder::LE>]>,
) -> Result<BTreeSet<LabelId>, DbError> {
    let run = run.ok_or_else(|| DbError::invalid_store("base label run out of bounds"))?;
    Ok(run.iter().map(|word| LabelId::new(word.get())).collect())
}

/// Decodes the base property records into an owned subject/key map.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a record's subject kind, value tag, or
/// text slice cannot be resolved.
///
/// # Performance
///
/// This function is `O(properties + text bytes)`.
fn decode_base_properties(
    view: &BaseView<'_>,
) -> Result<BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>, DbError> {
    let mut properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> =
        BTreeMap::new();
    for record in view.properties() {
        let subject =
            crate::wire::decode_subject(record.subject_kind.get(), record.subject_id.get())
                .ok_or_else(|| DbError::invalid_store("base property subject kind out of range"))?;
        let value_type = crate::wire::property_type_from_tag(record.value_tag.get())
            .ok_or_else(|| DbError::invalid_store("base property value tag out of range"))?;
        let value = match value_type {
            PropertyType::Boolean => PropertyValue::Boolean(record.scalar.get() != 0),
            PropertyType::Integer => PropertyValue::Integer(record.scalar.get().cast_signed()),
            PropertyType::Text => {
                let bytes = view
                    .property_text(record)
                    .ok_or_else(|| DbError::invalid_store("base property text out of bounds"))?;
                let text = core::str::from_utf8(bytes)
                    .map_err(|_error| DbError::invalid_store("base property text is not UTF-8"))?;
                PropertyValue::Text(text.to_owned())
            }
        };
        properties
            .entry(subject)
            .or_default()
            .insert(PropertyKeyId::new(record.key.get()), value);
    }
    Ok(properties)
}

/// The immutable unit a read transaction pins: one base generation plus the
/// frozen overlay published over it, tagged by `(generation, lsn)` identity.
///
/// `begin_read` clones the database's current `Arc<Snapshot>`, so a reader holds
/// the whole triple by `Arc` and observes a fixed state even across later
/// commits/checkpoints. It is [`Send`] + [`Sync`] (asserted below)
/// because [`Base`] is `Send + Sync`, [`Overlay`] holds owned `Send + Sync`
/// value types, and the identity fields are `Copy`.
///
/// # Performance
///
/// Cloning a [`Snapshot`] is `O(1)` (it clones two `Arc`s and copies the
/// identity fields).
#[derive(Clone)]
pub(crate) struct Snapshot {
    /// Checkpoint generation of the pinned base.
    generation: CheckpointGeneration,
    /// Committed transaction sequence (the live frontier) of this snapshot.
    lsn: CommitSeq,
    /// The pinned, immutable base generation.
    base: Arc<Base>,
    /// The frozen overlay published over `base`.
    overlay: Arc<Overlay>,
    /// Owned realization of the base records the merge borrows from.
    base_records: Arc<BaseRecords>,
}

impl Snapshot {
    /// Builds a snapshot pinning `base` under `overlay`, tagged `(generation,
    /// lsn)`, materializing the base's owned records once for the merge.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidStore`] when the base's wire records cannot be
    /// decoded into owned records.
    ///
    /// # Performance
    ///
    /// This function is `O(base records + labels + property bytes)` for the
    /// one-time record realization.
    pub(crate) fn new(
        generation: CheckpointGeneration,
        lsn: CommitSeq,
        base: Arc<Base>,
        overlay: Arc<Overlay>,
    ) -> Result<Self, DbError> {
        let base_records = Arc::new(BaseRecords::from_view(base.get())?);
        Ok(Self {
            generation,
            lsn,
            base,
            overlay,
            base_records,
        })
    }

    /// Publishes a new snapshot over the SAME base generation as a parent,
    /// SHARING the parent's already-materialized [`BaseRecords`] (and its derived
    /// [`crate::index::BaseIndex`]) by `Arc` instead of re-decoding the base wire
    /// bytes and rebuilding the index.
    ///
    /// This is the commit publish path: a commit never folds, so its base bytes
    /// are byte-identical to the parent's within the generation, and the records +
    /// index derived from them are identical too. Sharing them makes a commit
    /// `O(change)` (frame encode + overlay freeze) rather than `O(base)` — the
    /// caller MUST pass `base` and `base_records` drawn from the same parent
    /// snapshot so the shared records match the pinned base.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`: it clones two `Arc`s and copies the identity
    /// fields, with no base decode or index rebuild.
    pub(crate) const fn with_shared_base_records(
        generation: CheckpointGeneration,
        lsn: CommitSeq,
        base: Arc<Base>,
        overlay: Arc<Overlay>,
        base_records: Arc<BaseRecords>,
    ) -> Self {
        Self {
            generation,
            lsn,
            base,
            overlay,
            base_records,
        }
    }

    /// Returns this snapshot's checkpoint generation.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn generation(&self) -> CheckpointGeneration {
        self.generation
    }

    /// Returns this snapshot's committed transaction sequence (the frontier).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn lsn(&self) -> CommitSeq {
        self.lsn
    }

    /// Returns the frozen overlay published over this snapshot's base.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn overlay(&self) -> &Arc<Overlay> {
        &self.overlay
    }

    /// Returns the pinned base generation.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn base(&self) -> &Arc<Base> {
        &self.base
    }

    /// Returns the owned base records this snapshot's merge borrows from. The
    /// commit path layers a fresh [`WriteOverlay`] over these for write reads.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn base_records(&self) -> &Arc<BaseRecords> {
        &self.base_records
    }

    /// Returns a borrowed merged read view over this snapshot: overlay-then-base
    /// with tombstones masked.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to construct; reads through it carry their own
    /// contracts.
    pub(crate) fn view(&self) -> MergedState<'_> {
        MergedState {
            base: &self.base_records,
            overlay: &self.overlay,
        }
    }
}

/// Asserts a type is `Send + Sync` at compile time.
///
/// # Performance
///
/// `perf: unspecified`; compile-time only.
const fn assert_send_sync<T: Send + Sync>() {}

/// `Snapshot` MUST be `Send + Sync`: a `ReadTransaction` pinning it (a later
/// phase) crosses threads, and the snapshot owns only `Arc`-shared `Send + Sync`
/// data plus `Copy` identity fields.
const _: () = assert_send_sync::<Snapshot>();
/// `Overlay` MUST be `Send + Sync`: it is shared via `Arc<Overlay>` across the
/// snapshots a reader pins.
const _: () = assert_send_sync::<Overlay>();

/// The read surface a merged state exposes. Two tiers compose:
///
/// * the POINT/ITER/CATALOG/WATERMARK base surface — point reads, full iterators, counts, the
///   merged catalog, and the watermark; point reads return [`Cow`] so a base-only id borrows (zero
///   clone) while an overlay-supplied id is owned (these are the required methods below);
/// * the adjacency / membership / typed-lookup surface the live consumers call, provided as default
///   methods over the base surface (the block after [`Self::next_ids`]).
///
/// # Phase note — adjacency/membership/lookup surface is now COMPLETE (P4a)
///
/// This trait now covers the full surface the live consumers
/// (`query.rs`/`projection.rs`/`database.rs`) call, so P4 can swap
/// `DatabaseState` for a merged view behind it:
///
/// * adjacency accessors — [`Self::relation_incidences`] / [`Self::element_incidences`] (incidence
///   adjacency; both projection builders walk them, e.g. `projection.rs` `from_state` via
///   `relation_incidences`);
/// * membership lookups — [`Self::elements_with_label`] / [`Self::relations_with_type`] (`query.rs`
///   `scan_elements_with_label` / `scan_relations_with_type`);
/// * the typed lookup family — [`Self::typed_property_equal`],
///   [`Self::typed_property_equal_for_family`], [`Self::validate_lookup_value_for_family`],
///   [`Self::property_equal`], [`Self::property_range`], [`Self::typed_property_range`],
///   [`Self::typed_property_composite_equal`] (`query.rs` property scans, `database.rs` index
///   lookups).
///
/// # Phase note — membership/property lookups are now INDEX-BACKED (P5)
///
/// The default methods below are merge-aware SCANS (overlay-over-base,
/// tombstones masked) — correct, and the differential ORACLE the index path is
/// tested against. But the live [`LayeredState`] impl OVERRIDES the membership
/// and property lookups ([`Self::elements_with_label`],
/// [`Self::relations_with_type`], [`Self::property_equal`],
/// [`Self::property_range`], [`Self::typed_property_composite_equal`]) with
/// index-backed implementations that run in `O(log n + matches)` instead of
/// `O(n)`: they probe the per-generation [`crate::index::BaseIndex`] postings
/// (`Arc`-shared, built once per [`Snapshot`]) merged with the overlay's
/// incremental [`crate::index::OverlayIndex`] deltas. The typed wrappers
/// ([`Self::typed_property_equal`], [`Self::typed_property_range`], …) validate
/// against the catalog then delegate to those overridden lookups, so they are
/// index-backed too. The scan oracles survive under `#[cfg(test)]` on
/// [`LayeredState`] (the `*_scan` methods) for the differential test.
///
/// This trait is the live read surface: `query.rs`, `projection.rs`, and
/// `database.rs` all reach state through it, with NO change to these signatures.
///
/// # Performance
///
/// `perf: unspecified`; each method carries its own contract in its impl.
pub(crate) trait StateView {
    /// Returns the visible element for `id`, or `None` when absent or
    /// tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn element(&self, id: ElementId) -> Option<Cow<'_, ElementRecord>>;

    /// Returns the visible relation for `id`, or `None` when absent/tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn relation(&self, id: RelationId) -> Option<Cow<'_, RelationRecord>>;

    /// Returns the visible incidence for `id`, or `None` when absent/tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn incidence(&self, id: IncidenceId) -> Option<Cow<'_, IncidenceRecord>>;

    /// Returns the visible property value for `(subject, key)`, or `None` when
    /// absent or removed.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn property(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'_, PropertyValue>>;

    /// Returns whether an element is visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn contains_element(&self, id: ElementId) -> bool {
        self.element(id).is_some()
    }

    /// Returns whether a relation is visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn contains_relation(&self, id: RelationId) -> bool {
        self.relation(id).is_some()
    }

    /// Returns whether an incidence is visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn contains_incidence(&self, id: IncidenceId) -> bool {
        self.incidence(id).is_some()
    }

    /// Iterates every visible element in ascending id order (overlay wins,
    /// tombstones masked).
    ///
    /// # Performance
    ///
    /// A full walk is `O(base + overlay change)`.
    fn elements(&self) -> impl Iterator<Item = Cow<'_, ElementRecord>>;

    /// Iterates every visible relation in ascending id order.
    ///
    /// # Performance
    ///
    /// A full walk is `O(base + overlay change)`.
    fn relations(&self) -> impl Iterator<Item = Cow<'_, RelationRecord>>;

    /// Iterates every visible incidence in ascending id order.
    ///
    /// # Performance
    ///
    /// A full walk is `O(base + overlay change)`.
    fn incidences(&self) -> impl Iterator<Item = Cow<'_, IncidenceRecord>>;

    /// Iterates every visible `(subject, key, value)` property triple in
    /// ascending `(subject, key)` order.
    ///
    /// # Performance
    ///
    /// A full walk is `O(base properties + overlay property change)`.
    fn properties(
        &self,
    ) -> impl Iterator<Item = (PropertySubject, PropertyKeyId, Cow<'_, PropertyValue>)>;

    /// Returns the number of visible elements.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    fn element_count(&self) -> usize {
        self.elements().count()
    }

    /// Returns the number of visible relations.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    fn relation_count(&self) -> usize {
        self.relations().count()
    }

    /// Returns the number of visible incidences.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    fn incidence_count(&self) -> usize {
        self.incidences().count()
    }

    /// Returns the merged catalog (overlay registrations folded over base).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn catalog(&self) -> &Catalog;

    /// Returns the nine monotonic id allocators (the watermark).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn next_ids(&self) -> NextIds;

    /// Returns every visible incidence attached to `element`, in ascending
    /// incidence-id order.
    ///
    /// This mirrors the former owned-state `element_incidences`; it
    /// returns an owned [`Vec`] (not a borrowed iterator) because the merged set
    /// mixes records owned by the overlay with records borrowed from the base, so
    /// no single borrow can span both. [`IncidenceRecord`] is [`Copy`], so the
    /// owned vector is cheap.
    ///
    /// # Performance
    ///
    /// This method is `O(base incidences + overlay incidence change)`: one merge
    /// scan filtered by element.
    fn element_incidences(&self, element: ElementId) -> Vec<IncidenceRecord> {
        self.incidences()
            .filter(|record| record.element == element)
            .map(Cow::into_owned)
            .collect()
    }

    /// Returns every visible incidence belonging to `relation`, in ascending
    /// incidence-id order.
    ///
    /// This mirrors the former owned-state `relation_incidences`; see
    /// [`Self::element_incidences`] for why the result is an owned [`Vec`].
    ///
    /// # Performance
    ///
    /// This method is `O(base incidences + overlay incidence change)`: one merge
    /// scan filtered by relation.
    fn relation_incidences(&self, relation: RelationId) -> Vec<IncidenceRecord> {
        self.incidences()
            .filter(|record| record.relation == relation)
            .map(Cow::into_owned)
            .collect()
    }

    /// Returns the ids of every visible element carrying `label`, in ascending
    /// element-id order.
    ///
    /// This mirrors the former owned-state `elements_with_label`. The
    /// live state keeps a derived membership index; the merged view scans instead
    /// (functional parity for now — P5b adds incremental membership maintenance).
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay element change)`: one merge scan testing
    /// each visible element's label set.
    fn elements_with_label(&self, label: LabelId) -> Vec<ElementId> {
        self.elements()
            .filter(|record| record.labels.contains(&label))
            .map(|record| record.id)
            .collect()
    }

    /// Returns the ids of every visible relation of `relation_type`, in ascending
    /// relation-id order.
    ///
    /// This mirrors the former owned-state `relations_with_type`; it scans
    /// the merged relations rather than consulting a derived index (P5b adds
    /// incremental maintenance).
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay relation change)`: one merge scan testing
    /// each visible relation's type.
    fn relations_with_type(&self, relation_type: RelationTypeId) -> Vec<RelationId> {
        self.relations()
            .filter(|record| record.relation_type == Some(relation_type))
            .map(|record| record.id)
            .collect()
    }

    /// Returns the subjects whose property under `key` equals `value`, in
    /// ascending subject order.
    ///
    /// This mirrors the former owned-state `property_equal`: an unvalidated
    /// scan over the merged visible properties.
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`: one merge
    /// scan filtered by `(key, value)`.
    fn property_equal(&self, key: PropertyKeyId, value: &PropertyValue) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, candidate_value)| {
                *candidate_key == key && candidate_value.as_ref() == value
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Returns the subjects whose typed property under `key` equals `value`, after
    /// validating `value` against the key schema.
    ///
    /// This mirrors the former owned-state `typed_property_equal`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`] when `key` is absent from the
    /// merged catalog, or [`DbError::PropertyTypeMismatch`] when `value`'s type
    /// does not match the key schema.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for the default scan; this
    /// method delegates to [`Self::property_equal`], so when the impl overrides
    /// that with an index — as [`LayeredState`] does (the live read path) — the
    /// realized cost is `O(log n + matches + overlay change)`.
    fn typed_property_equal(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value(key, value)?;
        Ok(self.property_equal(key, value))
    }

    /// Returns the subjects in `family` whose typed property under `key` equals
    /// `value`, after validating both the family and the value type.
    ///
    /// This mirrors
    /// the former owned-state `typed_property_equal_for_family`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`], [`DbError::WrongPropertyFamily`],
    /// or [`DbError::PropertyTypeMismatch`] when the merged-catalog check fails.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for the default scan; this
    /// method delegates to [`Self::property_equal`], so when the impl overrides
    /// that with an index — as [`LayeredState`] does (the live read path) — the
    /// realized cost is `O(log n + matches + overlay change)`.
    fn typed_property_equal_for_family(
        &self,
        key: PropertyKeyId,
        family: PropertyFamily,
        value: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value_for_family(key, family, value)?;
        Ok(self.property_equal(key, value))
    }

    /// Returns the subjects whose ordered property under `key` falls within the
    /// inclusive range `[min, max]`, in ascending subject order.
    ///
    /// This mirrors the former owned-state `property_range`: an unvalidated
    /// scan over the merged visible properties.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for this default scan;
    /// [`LayeredState`] overrides it with an index-backed walk that is
    /// `O(log n + matching postings + matches + overlay change)`.
    fn property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, value)| {
                *candidate_key == key && value.as_ref() >= min && value.as_ref() <= max
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Returns the subjects whose typed property under `key` falls within the
    /// inclusive range `[min, max]`, after validating both bounds.
    ///
    /// This mirrors the former owned-state `typed_property_range`; an
    /// inverted range (`min > max`) yields an empty result without scanning.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`] or [`DbError::PropertyTypeMismatch`]
    /// when either bound fails the merged-catalog check.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for the default scan; this
    /// method delegates to [`Self::property_range`], so when the impl overrides
    /// that with an index — as [`LayeredState`] does (the live read path) — the
    /// realized cost is `O(log n + matching postings + matches + overlay change)`.
    fn typed_property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value(key, min)?;
        self.validate_lookup_value(key, max)?;
        if min > max {
            return Ok(Vec::new());
        }
        Ok(self.property_range(key, min, max))
    }

    /// Returns the subjects matching an ordered typed property tuple: a subject is
    /// returned only when it carries every `keys[i]` equal to `values[i]`, in
    /// ascending subject order.
    ///
    /// This mirrors
    /// the former owned-state `typed_property_composite_equal`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::unsupported`] on a `keys`/`values` arity mismatch, and
    /// [`DbError::UnknownPropertyKey`] or [`DbError::PropertyTypeMismatch`] when a
    /// `(key, value)` pair fails the merged-catalog check.
    ///
    /// # Performance
    ///
    /// This method is `O((base subjects + overlay subject change) × tuple
    /// arity)`: one grouped merge scan per subject.
    fn typed_property_composite_equal(
        &self,
        keys: &[PropertyKeyId],
        values: &[PropertyValue],
    ) -> Result<Vec<PropertySubject>, DbError> {
        if keys.len() != values.len() {
            return Err(DbError::unsupported(
                "composite equality tuple arity mismatch",
            ));
        }
        for (key, value) in keys.iter().copied().zip(values) {
            self.validate_lookup_value(key, value)?;
        }
        Ok(self
            .properties_by_subject()
            .into_iter()
            .filter_map(|(subject, subject_values)| {
                keys.iter()
                    .copied()
                    .zip(values)
                    .all(|(key, value)| subject_values.get(&key) == Some(value))
                    .then_some(subject)
            })
            .collect())
    }

    /// Validates a lookup `value`'s type against the merged catalog's schema for
    /// `key`.
    ///
    /// This mirrors the private the former owned-state `validate_lookup_value`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`] when `key` is absent from the
    /// merged catalog, or [`DbError::PropertyTypeMismatch`] when `value`'s type
    /// does not match the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(log catalog keys)`.
    fn validate_lookup_value(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<(), DbError> {
        let definition = self
            .catalog()
            .property_key(key)
            .ok_or(DbError::UnknownPropertyKey { id: key })?;
        let actual = value.value_type();
        if definition.value_type != actual {
            return Err(DbError::PropertyTypeMismatch {
                expected: definition.value_type,
                actual,
            });
        }
        Ok(())
    }

    /// Validates a lookup `value`'s type and subject `family` against the merged
    /// catalog's schema for `key`.
    ///
    /// This mirrors
    /// the former owned-state `validate_lookup_value_for_family` and
    /// delegates entirely to the merged catalog returned by [`Self::catalog`].
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`], [`DbError::WrongPropertyFamily`],
    /// or [`DbError::PropertyTypeMismatch`].
    ///
    /// # Performance
    ///
    /// This method is `O(log catalog keys)`.
    fn validate_lookup_value_for_family(
        &self,
        key: PropertyKeyId,
        family: PropertyFamily,
        value: &PropertyValue,
    ) -> Result<(), DbError> {
        let definition = self
            .catalog()
            .property_key(key)
            .ok_or(DbError::UnknownPropertyKey { id: key })?;
        if definition.family != family {
            return Err(DbError::WrongPropertyFamily {
                expected: definition.family,
                actual: family,
            });
        }
        if definition.value_type != value.value_type() {
            return Err(DbError::PropertyTypeMismatch {
                expected: definition.value_type,
                actual: value.value_type(),
            });
        }
        Ok(())
    }

    /// Returns the merged visible properties grouped by subject, each subject's
    /// keys mapped to their visible value, ascending by `(subject, key)`.
    ///
    /// This materializes one entry per visible `(subject, key)` pair so a
    /// per-subject predicate (e.g. composite equality) can test every key of a
    /// subject together. It is the merged analogue of the nested property map the
    /// live `DatabaseState` already holds.
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`.
    fn properties_by_subject(
        &self,
    ) -> BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> {
        let mut grouped: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> =
            BTreeMap::new();
        for (subject, key, value) in self.properties() {
            grouped
                .entry(subject)
                .or_default()
                .insert(key, value.into_owned());
        }
        grouped
    }
}

/// The delta-map read surface a layered overlay exposes to the merge. Both the
/// frozen [`Overlay`] (published over a snapshot) and the in-flight
/// [`WriteOverlay`] (a single writer's private delta) implement it, so the merge
/// iterators and point reads are written once and serve both.
///
/// # Performance
///
/// Every accessor is `O(1)`.
pub(crate) trait OverlayLayer {
    /// Returns the element delta map (id -> add/override, or tombstone).
    fn elements(&self) -> &Delta<ElementRecord>;
    /// Returns the relation delta map.
    fn relations(&self) -> &Delta<RelationRecord>;
    /// Returns the incidence delta map.
    fn incidences(&self) -> &Delta<IncidenceRecord>;
    /// Returns the property delta map.
    fn properties(
        &self,
    ) -> &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>>;
    /// Returns the merged catalog (parent registrations plus this layer's).
    fn catalog(&self) -> &Catalog;
    /// Returns the nine monotonic id allocators (the watermark).
    fn next_ids(&self) -> NextIds;
    /// Returns this layer's incremental index deltas, so a merged lookup can be
    /// index-backed (`(base index ∪ added) \ removed`) rather than a full scan.
    fn index(&self) -> &OverlayIndex;
}

impl OverlayLayer for Overlay {
    fn elements(&self) -> &Delta<ElementRecord> {
        &self.elements
    }

    fn relations(&self) -> &Delta<RelationRecord> {
        &self.relations
    }

    fn incidences(&self) -> &Delta<IncidenceRecord> {
        &self.incidences
    }

    fn properties(
        &self,
    ) -> &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>> {
        &self.properties
    }

    fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn next_ids(&self) -> NextIds {
        self.next
    }

    fn index(&self) -> &OverlayIndex {
        &self.index
    }
}

impl OverlayLayer for WriteOverlay {
    fn elements(&self) -> &Delta<ElementRecord> {
        &self.elements
    }

    fn relations(&self) -> &Delta<RelationRecord> {
        &self.relations
    }

    fn incidences(&self) -> &Delta<IncidenceRecord> {
        &self.incidences
    }

    fn properties(
        &self,
    ) -> &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>> {
        &self.properties
    }

    fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn next_ids(&self) -> NextIds {
        self.next
    }

    fn index(&self) -> &OverlayIndex {
        &self.index
    }
}

/// A borrowed merged read view: an [`OverlayLayer`] layered over [`BaseRecords`].
///
/// Reads resolve overlay-first: a point read returns the overlay's record when
/// the overlay has an entry (a set value is [`Cow::Owned`]; a tombstone is
/// `None`), and otherwise borrows the base record ([`Cow::Borrowed`], the
/// zero-clone fast path) — `None` when the base lacks it too. Iterators k-way
/// merge the two ascending streams, yielding each id once with the overlay
/// winning and tombstones masked.
///
/// # Performance
///
/// Construction is `O(1)`; reads carry the contracts on [`StateView`].
#[derive(Clone, Copy)]
pub(crate) struct LayeredState<'a, L: OverlayLayer> {
    /// Owned base records the merge borrows from for base-only ids.
    base: &'a BaseRecords,
    /// The overlay layer (frozen or in-flight) layered over the base.
    overlay: &'a L,
}

impl<'a, L: OverlayLayer> LayeredState<'a, L> {
    /// Builds a merged view over an explicit `base`/`overlay` pair.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn new(base: &'a BaseRecords, overlay: &'a L) -> Self {
        Self { base, overlay }
    }

    /// Returns the visible element for `id`, borrowed from the DATA (lifetime
    /// `'a`) rather than from `&self`, so the returned [`Cow`] outlives this
    /// temporary view. The [`StateView`] impl delegates here.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn element_ref(&self, id: ElementId) -> Option<Cow<'a, ElementRecord>> {
        match self.overlay.elements().get(&id) {
            Some(Some(record)) => Some(Cow::Owned(record.clone())),
            Some(None) => None,
            None => self.base.element(id).map(Cow::Borrowed),
        }
    }

    /// Returns the visible relation for `id`, borrowed from the data (see
    /// [`Self::element_ref`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn relation_ref(&self, id: RelationId) -> Option<Cow<'a, RelationRecord>> {
        match self.overlay.relations().get(&id) {
            Some(Some(record)) => Some(Cow::Owned(record.clone())),
            Some(None) => None,
            None => self.base.relation(id).map(Cow::Borrowed),
        }
    }

    /// Returns the visible incidence for `id`, borrowed from the data (see
    /// [`Self::element_ref`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn incidence_ref(&self, id: IncidenceId) -> Option<Cow<'a, IncidenceRecord>> {
        match self.overlay.incidences().get(&id) {
            Some(Some(record)) => Some(Cow::Owned(*record)),
            Some(None) => None,
            None => self.base.incidence(id).map(Cow::Borrowed),
        }
    }

    /// Returns the visible property value for `(subject, key)`, borrowed from the
    /// data (see [`Self::element_ref`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn property_ref(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'a, PropertyValue>> {
        match self
            .overlay
            .properties()
            .get(&subject)
            .and_then(|keys| keys.get(&key))
        {
            Some(Some(value)) => Some(Cow::Owned(value.clone())),
            Some(None) => None,
            None => self.base.property(subject, key).map(Cow::Borrowed),
        }
    }

    /// Returns the merged catalog borrowed from the data (lifetime `'a`), so the
    /// reference outlives this temporary view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) fn catalog_ref(&self) -> &'a Catalog {
        self.overlay.catalog()
    }
}

/// The published merged read view: a frozen [`Overlay`] over [`BaseRecords`].
///
/// # Performance
///
/// `perf: unspecified`; an alias.
pub(crate) type MergedState<'a> = LayeredState<'a, Overlay>;

/// The in-flight merged read view: a single writer's [`WriteOverlay`] over
/// [`BaseRecords`], used by the write transaction to read its own pending state
/// for referential-integrity validation.
///
/// # Performance
///
/// `perf: unspecified`; an alias.
pub(crate) type WriteMergedState<'a> = LayeredState<'a, WriteOverlay>;

impl<L: OverlayLayer> StateView for LayeredState<'_, L> {
    fn element(&self, id: ElementId) -> Option<Cow<'_, ElementRecord>> {
        self.element_ref(id)
    }

    fn relation(&self, id: RelationId) -> Option<Cow<'_, RelationRecord>> {
        self.relation_ref(id)
    }

    fn incidence(&self, id: IncidenceId) -> Option<Cow<'_, IncidenceRecord>> {
        self.incidence_ref(id)
    }

    fn property(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'_, PropertyValue>> {
        self.property_ref(subject, key)
    }

    fn elements(&self) -> impl Iterator<Item = Cow<'_, ElementRecord>> {
        MergeIter::new(self.base.elements.values(), self.overlay.elements().iter())
    }

    fn relations(&self) -> impl Iterator<Item = Cow<'_, RelationRecord>> {
        MergeIter::new(
            self.base.relations.values(),
            self.overlay.relations().iter(),
        )
    }

    fn incidences(&self) -> impl Iterator<Item = Cow<'_, IncidenceRecord>> {
        MergeIter::new(
            self.base.incidences.values(),
            self.overlay.incidences().iter(),
        )
    }

    fn properties(
        &self,
    ) -> impl Iterator<Item = (PropertySubject, PropertyKeyId, Cow<'_, PropertyValue>)> {
        PropertyMergeIter::new(
            base_property_triples(self.base),
            overlay_property_triples(self.overlay.properties()),
        )
    }

    fn catalog(&self) -> &Catalog {
        self.overlay.catalog()
    }

    fn next_ids(&self) -> NextIds {
        self.overlay.next_ids()
    }

    /// Index-backed override of the membership lookup: merges the base label
    /// posting with the overlay's label deltas, instead of scanning every
    /// element. Equal to the scan oracle ([`Self::elements_with_label_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matches + overlay change)`.
    fn elements_with_label(&self, label: LabelId) -> Vec<ElementId> {
        self.overlay
            .index()
            .elements_with_label(self.base.index(), label)
    }

    /// Index-backed override of the relation-type membership lookup: merges the
    /// base relation-type posting with the overlay's deltas. Equal to the scan
    /// oracle ([`Self::relations_with_type_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matches + overlay change)`.
    fn relations_with_type(&self, relation_type: RelationTypeId) -> Vec<RelationId> {
        self.overlay
            .index()
            .relations_with_type(self.base.index(), relation_type)
    }

    /// Index-backed override of the equality lookup: probes the base equality
    /// posting for `(key, value)` and merges the overlay's deltas, instead of
    /// scanning every property. Equal to the scan oracle
    /// ([`Self::property_equal_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matches + overlay change)`.
    fn property_equal(&self, key: PropertyKeyId, value: &PropertyValue) -> Vec<PropertySubject> {
        self.overlay
            .index()
            .property_equal(self.base.index(), key, value)
    }

    /// Index-backed override of the range lookup: walks the contiguous ordered
    /// slice of the base equality map in `[min, max]` and merges the overlay's
    /// deltas, instead of scanning every property. Equal to the scan oracle
    /// ([`Self::property_range_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matching postings + matches + overlay change)`.
    fn property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.overlay
            .index()
            .property_range(self.base.index(), key, min, max)
    }

    /// Index-backed override of the composite-equality lookup: intersects the
    /// per-key merged equality postings of the validated `(key, value)` tuple,
    /// instead of grouping every subject's properties. Equal to the scan oracle
    /// ([`Self::typed_property_composite_equal_scan`]).
    ///
    /// # Errors
    ///
    /// Returns [`DbError::unsupported`] on a `keys`/`values` arity mismatch, and
    /// [`DbError::UnknownPropertyKey`] or [`DbError::PropertyTypeMismatch`] when a
    /// `(key, value)` pair fails the merged-catalog check.
    ///
    /// # Performance
    ///
    /// This method is `O(tuple arity × (matches + overlay change))`.
    fn typed_property_composite_equal(
        &self,
        keys: &[PropertyKeyId],
        values: &[PropertyValue],
    ) -> Result<Vec<PropertySubject>, DbError> {
        if keys.len() != values.len() {
            return Err(DbError::unsupported(
                "composite equality tuple arity mismatch",
            ));
        }
        for (key, value) in keys.iter().copied().zip(values) {
            self.validate_lookup_value(key, value)?;
        }
        let pairs: Vec<(PropertyKeyId, PropertyValue)> =
            keys.iter().copied().zip(values.iter().cloned()).collect();
        Ok(self
            .overlay
            .index()
            .property_composite_equal(self.base.index(), &pairs))
    }
}

/// Merge-SCAN oracles: the pre-P5 lookup implementations, kept under
/// `#[cfg(test)]` so the index-backed [`StateView`] overrides can be
/// differential-tested against them. Each scans the full merged visible set and
/// filters, exactly as the former default-trait implementations did; the
/// index-backed methods MUST return the same result for every input.
///
/// # Performance
///
/// `perf: unspecified`; test-only scan oracles, each `O(base + overlay change)`.
#[cfg(test)]
impl<L: OverlayLayer> LayeredState<'_, L> {
    /// Scan oracle for [`StateView::elements_with_label`].
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay element change)`.
    pub(crate) fn elements_with_label_scan(&self, label: LabelId) -> Vec<ElementId> {
        self.elements()
            .filter(|record| record.labels.contains(&label))
            .map(|record| record.id)
            .collect()
    }

    /// Scan oracle for [`StateView::relations_with_type`].
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay relation change)`.
    pub(crate) fn relations_with_type_scan(
        &self,
        relation_type: RelationTypeId,
    ) -> Vec<RelationId> {
        self.relations()
            .filter(|record| record.relation_type == Some(relation_type))
            .map(|record| record.id)
            .collect()
    }

    /// Scan oracle for [`StateView::property_equal`].
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`.
    pub(crate) fn property_equal_scan(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, candidate_value)| {
                *candidate_key == key && candidate_value.as_ref() == value
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Scan oracle for [`StateView::property_range`].
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`.
    pub(crate) fn property_range_scan(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, value)| {
                *candidate_key == key && value.as_ref() >= min && value.as_ref() <= max
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Scan oracle for [`StateView::typed_property_composite_equal`] (the inner
    /// grouped-scan path, without revalidating the catalog).
    ///
    /// # Performance
    ///
    /// This method is `O((base subjects + overlay subject change) × arity)`.
    pub(crate) fn property_composite_equal_scan(
        &self,
        keys: &[PropertyKeyId],
        values: &[PropertyValue],
    ) -> Vec<PropertySubject> {
        self.properties_by_subject()
            .into_iter()
            .filter_map(|(subject, subject_values)| {
                keys.iter()
                    .copied()
                    .zip(values)
                    .all(|(key, value)| subject_values.get(&key) == Some(value))
                    .then_some(subject)
            })
            .collect()
    }
}

/// K-way merge of an ascending base record stream and an ascending overlay delta
/// stream over the same id type, yielding each visible record once: the overlay
/// wins on a shared id, a `None` overlay entry (tombstone) masks the base, and
/// an overlay-only id appears as owned.
///
/// # Performance
///
/// Each `next` is `O(1)` amortized; a full walk is `O(base + overlay change)`.
struct MergeIter<'a, R: Keyed + Clone> {
    /// Peekable ascending base record iterator.
    base: std::iter::Peekable<btree_map::Values<'a, R::Id, R>>,
    /// Peekable ascending overlay delta iterator.
    overlay: std::iter::Peekable<btree_map::Iter<'a, R::Id, Option<R>>>,
}

impl<'a, R: Keyed + Clone> MergeIter<'a, R> {
    /// Builds a merge iterator from the two ascending sources.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn new(
        base: btree_map::Values<'a, R::Id, R>,
        overlay: btree_map::Iter<'a, R::Id, Option<R>>,
    ) -> Self {
        Self {
            base: base.peekable(),
            overlay: overlay.peekable(),
        }
    }

    /// Advances the base cursor and emits the consumed base record borrowed.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_base(&mut self) -> Option<Cow<'a, R>> {
        self.base.next().map(Cow::Borrowed)
    }

    /// Advances the overlay cursor: emits a set value owned, or `None` when the
    /// consumed entry is a tombstone (the caller loops to the next entry). When
    /// `mask_base` is set, the matching base record is consumed first so the
    /// overlay overrides it.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_overlay(&mut self, mask_base: bool) -> Step<Cow<'a, R>> {
        if mask_base {
            let _masked = self.base.next();
        }
        let Some((_id, entry)) = self.overlay.next() else {
            return Step::Done;
        };
        entry.as_ref().map_or(Step::Again, |record| {
            Step::Yield(Cow::Owned(record.clone()))
        })
    }

    /// Advances the merge by one decision over the two ascending peeks: a
    /// base-only or strictly-lower base id yields the borrowed base record; an
    /// overlay-lower or tied id consumes the overlay (masking the base on a tie),
    /// yielding the set value or looping past a tombstone.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn step(&mut self) -> Step<Cow<'a, R>> {
        let base_id = self.base.peek().map(|record| record.record_id());
        let overlay_id = self.overlay.peek().map(|(id, _entry)| **id);
        match (base_id, overlay_id) {
            (None, None) => Step::Done,
            (Some(_base), None) => self.take_base().map_or(Step::Done, Step::Yield),
            (Some(base), Some(overlay)) if base < overlay => {
                self.take_base().map_or(Step::Done, Step::Yield)
            }
            (_other, Some(overlay)) => self.take_overlay(base_id == Some(overlay)),
        }
    }
}

impl<'a, R: Keyed + Clone> Iterator for MergeIter<'a, R> {
    type Item = Cow<'a, R>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.step() {
                Step::Done => return None,
                Step::Yield(item) => return Some(item),
                Step::Again => {}
            }
        }
    }
}

/// One outcome of a single k-way merge step: terminate, yield an item, or loop
/// past a masked entry (a tombstone) to the next decision.
///
/// # Performance
///
/// `perf: unspecified`; this is a control-flow tag.
enum Step<T> {
    /// Both streams are exhausted; the merge is finished.
    Done,
    /// Emit this visible item.
    Yield(T),
    /// A tombstone was consumed; retry the next decision.
    Again,
}

/// K-way merge of the ascending base property triples and the ascending overlay
/// property delta, yielding each visible `(subject, key, value)` once in
/// ascending `(subject, key)` order: the overlay wins on a shared `(subject,
/// key)`, a `None` overlay entry (removal) masks the base, and an overlay-only
/// set value appears owned.
///
/// Both sides are pre-flattened into ascending vectors so the merge is a single
/// linear walk over two slices (the `BTreeMap`s are already sorted by `(subject,
/// key)`, so flattening preserves order). The cursors index into them.
///
/// # Performance
///
/// Each `next` is `O(1)`; a full walk is `O(base + overlay change)`.
struct PropertyMergeIter<'a> {
    /// Ascending base `(subject, key, &value)` triples.
    base: Vec<(PropertySubject, PropertyKeyId, &'a PropertyValue)>,
    /// Ascending overlay `(subject, key, set/remove)` triples.
    overlay: Vec<(PropertySubject, PropertyKeyId, &'a Option<PropertyValue>)>,
    /// Cursor into `base`.
    base_index: usize,
    /// Cursor into `overlay`.
    overlay_index: usize,
}

impl<'a> PropertyMergeIter<'a> {
    /// Builds a property merge iterator over two pre-flattened ascending sides.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn new(
        base: Vec<(PropertySubject, PropertyKeyId, &'a PropertyValue)>,
        overlay: Vec<(PropertySubject, PropertyKeyId, &'a Option<PropertyValue>)>,
    ) -> Self {
        Self {
            base,
            overlay,
            base_index: 0,
            overlay_index: 0,
        }
    }
}

impl<'a> PropertyMergeIter<'a> {
    /// Advances the base cursor and emits the consumed `(subject, key, value)`
    /// triple borrowed.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_base(&mut self) -> Option<(PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>)> {
        let (subject, key, value) = *self.base.get(self.base_index)?;
        self.base_index += 1;
        Some((subject, key, Cow::Borrowed(value)))
    }

    /// Advances the overlay cursor: emits a set value owned, or loops past a
    /// removal. When `mask_base` is set, the matching base triple is consumed
    /// first so the overlay overrides it.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_overlay(
        &mut self,
        mask_base: bool,
    ) -> Step<(PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>)> {
        if mask_base {
            self.base_index += 1;
        }
        let Some(&(subject, key, entry)) = self.overlay.get(self.overlay_index) else {
            return Step::Done;
        };
        self.overlay_index += 1;
        entry.as_ref().map_or(Step::Again, |value| {
            Step::Yield((subject, key, Cow::Owned(value.clone())))
        })
    }

    /// Advances the property merge by one decision over the two ascending peeks.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn step(&mut self) -> Step<(PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>)> {
        let base_pair = self
            .base
            .get(self.base_index)
            .map(|&(subject, key, _value)| (subject, key));
        let overlay_pair = self
            .overlay
            .get(self.overlay_index)
            .map(|&(subject, key, _entry)| (subject, key));
        match (base_pair, overlay_pair) {
            (None, None) => Step::Done,
            (Some(_base), None) => self.take_base().map_or(Step::Done, Step::Yield),
            (Some(base), Some(overlay)) if base < overlay => {
                self.take_base().map_or(Step::Done, Step::Yield)
            }
            (_other, Some(overlay)) => self.take_overlay(base_pair == Some(overlay)),
        }
    }
}

impl<'a> Iterator for PropertyMergeIter<'a> {
    type Item = (PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.step() {
                Step::Done => return None,
                Step::Yield(item) => return Some(item),
                Step::Again => {}
            }
        }
    }
}

/// Flattens the base property map into an ascending `(subject, key, &value)`
/// vector. The nested `BTreeMap`s are already sorted, so the flattened order is
/// ascending by `(subject, key)`.
///
/// # Performance
///
/// This function is `O(base properties)`.
fn base_property_triples(
    base: &BaseRecords,
) -> Vec<(PropertySubject, PropertyKeyId, &PropertyValue)> {
    base.properties
        .iter()
        .flat_map(|(subject, keys)| keys.iter().map(move |(key, value)| (*subject, *key, value)))
        .collect()
}

/// Flattens the overlay property delta into an ascending `(subject, key,
/// set/remove)` vector.
///
/// # Performance
///
/// This function is `O(overlay property change)`.
fn overlay_property_triples(
    properties: &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>>,
) -> Vec<(PropertySubject, PropertyKeyId, &Option<PropertyValue>)> {
    properties
        .iter()
        .flat_map(|(subject, keys)| keys.iter().map(move |(key, value)| (*subject, *key, value)))
        .collect()
}

/// Bounded, base-free constructors the kani proofs use to build small overlay
/// inputs directly (a frozen [`Base`] needs the container/snapshot machinery
/// kani cannot evaluate, so the proofs build [`BaseRecords`] and [`Overlay`]
/// element layers in memory and merge them).
///
/// # Performance
///
/// `perf: unspecified`; these are bounded proof helpers compiled only under
/// `cfg(kani)`.
#[cfg(kani)]
impl BaseRecords {
    /// Builds base records holding exactly the elements with the given ids (no
    /// labels), and no relations/incidences/properties.
    ///
    /// # Performance
    ///
    /// This function is `O(ids)`.
    pub(crate) fn proof_elements(ids: &[ElementId]) -> Self {
        let mut elements = BTreeMap::new();
        for id in ids.iter().copied() {
            elements.insert(
                id,
                ElementRecord {
                    id,
                    labels: BTreeSet::new(),
                },
            );
        }
        Self {
            elements,
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            index: BaseIndex::empty(),
        }
    }
}

#[cfg(kani)]
impl Overlay {
    /// Builds an overlay whose element layer holds the given `(id, present)`
    /// entries: `present` records an add/override (with no labels), `!present`
    /// records a tombstone. The watermark and catalog are empty.
    ///
    /// # Performance
    ///
    /// This function is `O(entries)`.
    pub(crate) fn proof_element_entries(entries: &[(ElementId, bool)]) -> Self {
        let mut elements = BTreeMap::new();
        for (id, present) in entries.iter().copied() {
            let entry = present.then(|| ElementRecord {
                id,
                labels: BTreeSet::new(),
            });
            elements.insert(id, entry);
        }
        Self {
            elements,
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog: Catalog::empty(),
            next: proof_zero_next_ids(),
            index: OverlayIndex::new(),
        }
    }

    /// Returns whether this overlay tombstones `id` at the element layer.
    ///
    /// # Performance
    ///
    /// This function is `O(log change)`.
    pub(crate) fn proof_is_element_tombstoned(&self, id: ElementId) -> bool {
        matches!(self.elements.get(&id), Some(None))
    }

    /// Builds an empty overlay whose watermark's element allocator is `next`
    /// (every other allocator starts at one). Used by the watermark proof to set
    /// a parent's element allocator without driving it through allocations.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) fn proof_with_next_element(next: u64) -> Self {
        let mut watermark = proof_zero_next_ids();
        watermark.element = ElementId::new(next);
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog: Catalog::empty(),
            next: watermark,
            index: OverlayIndex::new(),
        }
    }
}

/// The all-ones watermark used to seed proof overlays (the actual values are
/// irrelevant to the element-merge proofs).
///
/// # Performance
///
/// This function is `O(1)`.
#[cfg(kani)]
fn proof_zero_next_ids() -> NextIds {
    NextIds {
        element: ElementId::new(1),
        relation: RelationId::new(1),
        incidence: IncidenceId::new(1),
        role: RoleId::new(1),
        label: LabelId::new(1),
        relation_type: RelationTypeId::new(1),
        property_key: PropertyKeyId::new(1),
        projection: ProjectionId::new(1),
        index: IndexId::new(1),
    }
}

/// Test-only base builders shared by the freeze, backing, and overlay tests.
///
/// `DatabaseState` (the old owned whole-DB model) is gone, so test fixtures now
/// build a base by accumulating a [`WriteOverlay`] over an empty base and
/// freezing the merged view through [`crate::freeze::freeze_view`] — the exact
/// path `create`/`checkpoint` run. These helpers are compiled only under
/// `cfg(test)`.
///
/// # Performance
///
/// `perf: unspecified`; test helpers.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{BaseRecords, MergedState, Overlay, WriteOverlay};
    use crate::{
        PropertySubject,
        backing::Base,
        catalog::{Catalog, PropertyFamily},
        freeze::{FreezeStamps, freeze_view},
        state::NextIds,
        value::{PropertyType, PropertyValue},
    };

    /// Freezes a writer delta layered over an empty base into a [`Base`] over
    /// owned bytes (the miri-safe attach path), stamping fixed checkpoint
    /// values. The writer is the only source of records, so the frozen base
    /// holds exactly what the writer created.
    ///
    /// # Performance
    ///
    /// This function is `O(writer change)`.
    pub(crate) fn freeze_writer(write: WriteOverlay) -> Base {
        let base = BaseRecords::empty();
        let overlay = write.freeze();
        let view = MergedState::new(&base, &overlay);
        let bytes = freeze_view(
            &view,
            FreezeStamps {
                commit_seq: 1,
                transaction_id: 1,
                generation: 1,
            },
        )
        .expect("freeze writer view");
        Base::open_owned_bytes(bytes).expect("attach base")
    }

    /// Builds a small canonical base: two labels, one relation type, one role,
    /// two element-family property keys, one relation-family key, one
    /// incidence-family key, three elements, two relations, two incidences, and
    /// several properties — enough to exercise creates, labels, types, every
    /// property scalar shape, AND a typed/property-bearing relation plus a
    /// property-bearing incidence (so the relation/incidence tombstone index
    /// withdrawal paths have base postings to withdraw).
    ///
    /// # Performance
    ///
    /// This function is `O(fixture size)`.
    pub(crate) fn small_base() -> Base {
        let mut write = WriteOverlay::new(NextIds::INITIAL, Catalog::empty());
        let base = BaseRecords::empty();
        let person = write.register_label("Person".to_owned()).expect("label");
        let _robot = write.register_label("Robot".to_owned()).expect("label");
        let calls = write
            .register_relation_type("calls".to_owned())
            .expect("rtype");
        let caller = write.register_role("caller".to_owned()).expect("role");
        let name = write
            .register_property_key(
                "name".to_owned(),
                PropertyFamily::Element,
                PropertyType::Text,
            )
            .expect("name key");
        let rank = write
            .register_property_key(
                "rank".to_owned(),
                PropertyFamily::Element,
                PropertyType::Integer,
            )
            .expect("rank key");
        let weight = write
            .register_property_key(
                "weight".to_owned(),
                PropertyFamily::Relation,
                PropertyType::Integer,
            )
            .expect("weight key");
        let slot = write
            .register_property_key(
                "slot".to_owned(),
                PropertyFamily::Incidence,
                PropertyType::Integer,
            )
            .expect("slot key");

        let e1 = write.create_element().expect("e1");
        let e2 = write.create_element().expect("e2");
        let _e3 = write.create_element().expect("e3");
        write.add_element_label(&base, e1, person);
        write.add_element_label(&base, e2, person);

        let r1 = write.create_relation().expect("r1");
        let _r2 = write.create_relation().expect("r2");
        write.set_relation_type(&base, r1, calls);
        let inc1 = write.create_incidence(r1, e1, caller).expect("inc1");
        write.create_incidence(r1, e2, caller).expect("inc2");

        write.set_property(
            &base,
            PropertySubject::Element(e1),
            name,
            PropertyValue::Text("Alice".to_owned()),
        );
        // A relation-family property on the typed relation r1 and an
        // incidence-family property on inc1, so tombstoning each withdraws a real
        // base equality posting (and r1 additionally withdraws its `calls` type).
        write.set_property(
            &base,
            PropertySubject::Relation(r1),
            weight,
            PropertyValue::Integer(3),
        );
        write.set_property(
            &base,
            PropertySubject::Incidence(inc1),
            slot,
            PropertyValue::Integer(1),
        );
        write.set_property(
            &base,
            PropertySubject::Element(e1),
            rank,
            PropertyValue::Integer(-5),
        );
        write.set_property(
            &base,
            PropertySubject::Element(e2),
            name,
            PropertyValue::Text("Bob".to_owned()),
        );

        freeze_writer(write)
    }

    /// Builds a small `(BaseRecords, Overlay)` pair the freeze test merges: an
    /// empty base under an overlay holding two created elements and a property,
    /// so the frozen view is non-trivial without needing a `DatabaseState`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) fn base_view_from_ops() -> (BaseRecords, Overlay) {
        let mut write = WriteOverlay::new(NextIds::INITIAL, Catalog::empty());
        let base = BaseRecords::empty();
        let name = write
            .register_property_key(
                "name".to_owned(),
                PropertyFamily::Element,
                PropertyType::Text,
            )
            .expect("name key");
        let e1 = write.create_element().expect("e1");
        let _e2 = write.create_element().expect("e2");
        write.set_property(
            &base,
            PropertySubject::Element(e1),
            name,
            PropertyValue::Text("Alice".to_owned()),
        );
        (base, write.freeze())
    }
}

#[cfg(test)]
mod tests;
