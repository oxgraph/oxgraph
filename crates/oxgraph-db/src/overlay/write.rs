//! The mutable delta a single in-flight write transaction accumulates.
//!
//! [`WriteOverlay`] records creates, tombstones, property sets/removes, and
//! catalog registrations into per-family delta maps without touching the
//! base, logging each mutation for the WAL; [`WriteOverlay::freeze`] turns
//! the accumulated delta into a published [`Overlay`].

use std::{
    collections::{BTreeMap, btree_map},
    sync::Arc,
};

use super::{
    Delta, SubjectDelta,
    frozen::{BaseRecords, Overlay},
    log::{MutationLog, blob_str, decode_def_words},
};
use crate::{
    Catalog, DbError, ElementId, IdFamily, IncidenceId, IndexId, LabelId, ProjectionId,
    PropertyKeyId, RelationId, RelationTypeId, RoleId, StorageError, TxnError,
    catalog::{IndexDefinition, ProjectionDefinition, PropertyFamily, PropertyKeyDefinition},
    index::OverlayIndex,
    state::{ElementRecord, IncidenceRecord, LabelSet, NextIds, PropertySubject, RelationRecord},
    value::{PropertyType, PropertyValue},
    wire::{self, MutationOp},
};

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
    pub(super) elements: Delta<ElementRecord>,
    /// Relation delta: id -> add/override record, or tombstone.
    pub(super) relations: Delta<RelationRecord>,
    /// Incidence delta: id -> add/override record, or tombstone.
    pub(super) incidences: Delta<IncidenceRecord>,
    /// Property delta: subject -> key -> set value, or remove (tombstone).
    /// Each per-subject inner map is `Arc`-shared copy-on-write with the parent
    /// overlay this writer was seeded from; a mutation copies only the touched
    /// subject's map (via [`Arc::make_mut`]).
    pub(super) properties: BTreeMap<PropertySubject, SubjectDelta>,
    /// Catalog additions accumulated by this writer (register-only in v1).
    pub(super) catalog: Catalog,
    /// The nine monotonic id allocators (the watermark) after this delta.
    pub(super) next: NextIds,
    /// Ordered, replayable log of every mutation this writer applied.
    log: MutationLog,
    /// Incremental membership/equality index deltas this writer maintains, kept
    /// in lockstep with the record/property deltas so a merged lookup is
    /// index-backed (`O(log n + matches)`), not a full scan.
    pub(super) index: OverlayIndex,
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
    /// This function is `O(parent change)` map entries with `O(1)` per entry:
    /// the delta map structure is cloned, while label sets, text values,
    /// per-subject property delta maps, and index postings are `Arc`-shared
    /// copy-on-write (no payload bytes are duplicated; only the entries this
    /// writer mutates are copied later).
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

    /// Takes this writer's mutation log as a WAL frame: the ordered ops with
    /// the nine-value watermark appended as the final op, plus the interned
    /// blob. The log is left empty — commit calls this exactly once, and the
    /// subsequent freeze does not carry the log — so no frame bytes are
    /// cloned.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`; it moves the log out.
    pub(crate) fn take_frame(&mut self) -> (Vec<MutationOp>, Vec<u8>) {
        let mut log = core::mem::take(&mut self.log);
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
    /// Returns [`StorageError::LogCorrupt`] when an op kind is unknown, a payload tag
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
                let subject = wire::decode_subject(subject_kind, word(0)).ok_or(
                    StorageError::LogCorrupt {
                        lsn,
                        reason: "remove-property subject kind",
                    },
                )?;
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
                return Err(StorageError::LogCorrupt {
                    lsn,
                    reason: "unknown mutation op kind",
                }
                .into());
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
                        labels: LabelSet::default(),
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
                        labels: LabelSet::default(),
                    }),
                );
            }
            wire::OP_CREATE_INCIDENCE => {
                let id = IncidenceId::new(word(0));
                let relation = RelationId::new(word(1));
                let element = ElementId::new(word(2));
                let role = RoleId::new(word(3));
                self.incidences.insert(
                    id,
                    Some(IncidenceRecord {
                        id,
                        relation,
                        element,
                        role,
                    }),
                );
                self.index.on_create_incidence(id, relation, element);
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
    /// Returns [`StorageError::LogCorrupt`] when a blob slice, tag, or definition body
    /// is malformed, or [`crate::CatalogError::DuplicateName`] when the catalog
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
        let registered = match kind {
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
                    wire::property_family_from_tag(family_tag).ok_or(StorageError::LogCorrupt {
                        lsn,
                        reason: "property-key family tag",
                    })?;
                let value_type =
                    wire::property_type_from_tag(value_tag).ok_or(StorageError::LogCorrupt {
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
                let definition = wire::defs::decode_projection_body(op.flags.get(), name, &words)
                    .map_err(|error| StorageError::LogCorrupt {
                    lsn,
                    reason: error.reason,
                })?;
                self.catalog
                    .insert_projection(ProjectionId::new(word(0)), definition)
            }
            // OP_CATALOG_REGISTER_INDEX (the only remaining catalog kind).
            _index => {
                let words = decode_def_words(blob, word(3), word(4), lsn)?;
                let definition =
                    wire::defs::decode_index_body(op.flags.get(), &words).map_err(|error| {
                        StorageError::LogCorrupt {
                            lsn,
                            reason: error.reason,
                        }
                    })?;
                self.catalog
                    .insert_index(IndexId::new(word(0)), name, definition)
            }
        };
        Ok(registered?)
    }

    /// Applies a decoded `OP_SET_PROPERTY` op against the blob (replay path).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::LogCorrupt`] when the subject kind or value tag is out
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
        let subject =
            wire::decode_subject(subject_kind, word(0)).ok_or(StorageError::LogCorrupt {
                lsn,
                reason: "set-property subject kind",
            })?;
        let value_type =
            wire::property_type_from_tag(value_tag).ok_or(StorageError::LogCorrupt {
                lsn,
                reason: "set-property value tag",
            })?;
        let value = match value_type {
            PropertyType::Boolean => PropertyValue::Boolean(word(2) != 0),
            PropertyType::Integer => PropertyValue::Integer(word(2).cast_signed()),
            PropertyType::Text => PropertyValue::from(blob_str(blob, word(3), word(4), lsn)?),
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
    /// property mutation, borrowed from the writer's property delta or the
    /// base: the overlay value when this writer already set it, the base value
    /// when the overlay has not touched it, or `None` when the overlay removed
    /// it or neither layer has it. An associated function over the delta map
    /// (not `&self`), so callers hold the borrow while mutating the disjoint
    /// index field — the index maintainer receives the leaving posting without
    /// cloning it.
    ///
    /// # Performance
    ///
    /// This function is `O(log change + log n)`.
    fn visible_property_in<'state>(
        properties: &'state BTreeMap<PropertySubject, SubjectDelta>,
        base: &'state BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<&'state PropertyValue> {
        properties
            .get(&subject)
            .and_then(|keys| keys.get(&key))
            .map_or_else(|| base.property(subject, key), Option::as_ref)
    }

    /// Returns whether the merged-visible value of `(subject, key)` already equals
    /// `value`, comparing in place without cloning the existing value. The overlay
    /// wins (a tombstone means "no value", so never equal); otherwise the base
    /// value is consulted.
    ///
    /// This is the no-op gate for [`Self::set_property`]: re-asserting an unchanged
    /// value must not log a mutation, so an incremental reconcile that re-sets
    /// every property of an unchanged subject stays `O(change)`.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n + value length)`.
    fn visible_property_is(
        &self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> bool {
        self.properties
            .get(&subject)
            .and_then(|keys| keys.get(&key))
            .map_or_else(
                || base.property(subject, key) == Some(value),
                |slot| slot.as_ref() == Some(value),
            )
    }

    /// Returns the merged-visible labels of `element` BEFORE a pending mutation
    /// (overlay record wins; a tombstone is empty; otherwise the base labels).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`: the returned [`LabelSet`] is a
    /// shared `O(1)` clone, never a deep copy.
    fn visible_element_labels(&self, base: &BaseRecords, element: ElementId) -> LabelSet {
        match self.elements.get(&element) {
            Some(Some(record)) => record.labels.clone(),
            Some(None) => LabelSet::default(),
            None => base
                .element(element)
                .map_or_else(LabelSet::default, |record| record.labels.clone()),
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

    /// Returns the merged-visible incidence record of `id` BEFORE a pending
    /// mutation (overlay record wins; a tombstone is `None`; otherwise base).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn visible_incidence(&self, base: &BaseRecords, id: IncidenceId) -> Option<IncidenceRecord> {
        match self.incidences.get(&id) {
            Some(Some(record)) => Some(*record),
            Some(None) => None,
            None => base.incidence(id).copied(),
        }
    }

    /// Returns every merged-visible property of `subject` BEFORE a pending
    /// tombstone, borrowed from the writer's property delta or the base, so
    /// the index can withdraw each `(key, value)` posting it leaves without
    /// cloning the values. An associated function over the delta map so the
    /// caller can hold the borrow while mutating the disjoint index field.
    ///
    /// # Performance
    ///
    /// This function is `O(subject's base + overlay properties)`.
    fn visible_subject_properties_in<'state>(
        properties: &'state BTreeMap<PropertySubject, SubjectDelta>,
        base: &'state BaseRecords,
        subject: PropertySubject,
    ) -> BTreeMap<PropertyKeyId, &'state PropertyValue> {
        let mut visible: BTreeMap<PropertyKeyId, &PropertyValue> = BTreeMap::new();
        if let Some(keys) = base.properties.get(&subject) {
            for (key, value) in keys {
                visible.insert(*key, value);
            }
        }
        let Some(keys) = properties.get(&subject) else {
            return visible;
        };
        for (key, entry) in keys.iter() {
            // A set overrides the base value; a removal masks it.
            if let Some(value) = entry {
                visible.insert(*key, value);
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
    /// Returns [`TxnError::IdOverflow`] when the element id space is exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)`.
    pub(crate) fn create_element(&mut self) -> Result<ElementId, DbError> {
        let id = self.next.element;
        self.next.element = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::Element,
        })?;
        self.elements.insert(
            id,
            Some(ElementRecord {
                id,
                labels: LabelSet::default(),
            }),
        );
        self.log.push(wire::OP_CREATE_ELEMENT, 0, &[id.get()]);
        Ok(id)
    }

    /// Records a fresh relation, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`TxnError::IdOverflow`] when the relation id space is exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log change)`.
    pub(crate) fn create_relation(&mut self) -> Result<RelationId, DbError> {
        let id = self.next.relation;
        self.next.relation = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::Relation,
        })?;
        self.relations.insert(
            id,
            Some(RelationRecord {
                id,
                relation_type: None,
                labels: LabelSet::default(),
            }),
        );
        self.log.push(wire::OP_CREATE_RELATION, 0, &[id.get()]);
        Ok(id)
    }

    /// Records a fresh incidence, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`TxnError::IdOverflow`] when the incidence id space is exhausted.
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
        self.next.incidence = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::Incidence,
        })?;
        self.incidences.insert(
            id,
            Some(IncidenceRecord {
                id,
                relation,
                element,
                role,
            }),
        );
        self.index.on_create_incidence(id, relation, element);
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
        let properties = Self::visible_subject_properties_in(&self.properties, base, subject);
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
        let properties = Self::visible_subject_properties_in(&self.properties, base, subject);
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
        let record = self.visible_incidence(base, id);
        let properties = Self::visible_subject_properties_in(&self.properties, base, subject);
        if let Some(record) = record {
            self.index
                .on_tombstone_incidence(id, record.relation, record.element, &properties);
        }
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
            let entry = Arc::make_mut(self.properties.entry(subject).or_default());
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
    /// The overlay is an unvalidated delta: the property layer and the record
    /// layer are independent maps, so this records a visible property even when
    /// the same writer has tombstoned the subject's element/relation/incidence
    /// (or never created it) — the property merge and the record merge do not
    /// consult each other. Referential-integrity validation lives in the
    /// write-transaction layer: the database's `set_property` calls
    /// `require_subject` to reject a property whose subject is absent before
    /// recording it here, so orphan properties never reach the live path.
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
        // Re-asserting the currently-visible value is a no-op: skip the log frame
        // and the index/property mutation entirely. This is what keeps an
        // incremental reconcile (which re-sets every property of every subject,
        // most unchanged) `O(change)` rather than `O(properties touched)` — without
        // it the commit logs the whole graph every reindex.
        if self.visible_property_is(base, subject, key, &value) {
            return;
        }
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
    /// This method is `O(log change + log n)`, plus a one-time `O(subject's
    /// delta entries)` copy when the subject's map is still shared with the
    /// parent overlay (`Arc`-COW).
    fn set_property_inner(
        &mut self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: PropertyValue,
    ) {
        let previous = Self::visible_property_in(&self.properties, base, subject, key);
        self.index.on_set_property(subject, key, previous, &value);
        Arc::make_mut(self.properties.entry(subject).or_default()).insert(key, Some(value));
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
    /// This method is `O(log change + log n)`, plus a one-time `O(subject's
    /// delta entries)` copy when the subject's map is still shared with the
    /// parent overlay (`Arc`-COW).
    fn remove_property_inner(
        &mut self,
        base: &BaseRecords,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) {
        let previous = Self::visible_property_in(&self.properties, base, subject, key);
        self.index.on_remove_property(subject, key, previous);
        Arc::make_mut(self.properties.entry(subject).or_default()).insert(key, None);
    }

    /// Registers a role, drawing its id from the watermark.
    ///
    /// # Errors
    ///
    /// Returns [`TxnError::IdOverflow`] when the role id space is exhausted or
    /// [`crate::CatalogError::DuplicateName`] when the name is taken.
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    pub(crate) fn register_role(&mut self, name: String) -> Result<RoleId, DbError> {
        let id = self.next.role;
        self.next.role = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::Role,
        })?;
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
    /// Returns [`TxnError::IdOverflow`] or [`crate::CatalogError::DuplicateName`].
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    pub(crate) fn register_label(&mut self, name: String) -> Result<LabelId, DbError> {
        let id = self.next.label;
        self.next.label = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::Label,
        })?;
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
    /// Returns [`TxnError::IdOverflow`] or [`crate::CatalogError::DuplicateName`].
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    pub(crate) fn register_relation_type(
        &mut self,
        name: String,
    ) -> Result<RelationTypeId, DbError> {
        let id = self.next.relation_type;
        self.next.relation_type = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::RelationType,
        })?;
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
    /// Returns [`TxnError::IdOverflow`] or [`crate::CatalogError::DuplicateName`].
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
        self.next.property_key = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::PropertyKey,
        })?;
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
    /// Returns [`TxnError::IdOverflow`] or [`crate::CatalogError::DuplicateName`].
    ///
    /// # Performance
    ///
    /// This method is `O(definition size)`.
    pub(crate) fn register_projection(
        &mut self,
        definition: ProjectionDefinition,
    ) -> Result<ProjectionId, DbError> {
        let id = self.next.projection;
        self.next.projection = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::Projection,
        })?;
        let (name_off, name_len) = self.log.intern(definition.name().as_bytes());
        let (kind, words) = wire::defs::encode_projection_body(&definition);
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
    /// Returns [`TxnError::IdOverflow`] or [`crate::CatalogError::DuplicateName`].
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
        self.next.index = id.checked_next().ok_or(TxnError::IdOverflow {
            family: IdFamily::Index,
        })?;
        let (name_off, name_len) = self.log.intern(name.as_bytes());
        let (kind, words) = wire::defs::encode_index_body(&definition);
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
