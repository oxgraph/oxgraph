//! The single writer transaction: mutators, typed surface, and commit.

use std::{collections::BTreeSet, sync::Arc};

use super::{Db, open::open_log_for_append};
use crate::{
    Bound, CommitSeq, DbError, ElementId, GraphProjectionDefinition, GraphProjectionSpec,
    IncidenceId, IndexId, LabelId, ProjectionDefinition, ProjectionId, PropertyKeyId,
    PropertySubject, PropertyType, PropertyValue, RelationId, RelationTypeId, RoleId, Schema,
    TransactionId,
    catalog::{IndexDefinition, PropertyFamily},
    lock::WriterLock,
    overlay::{Snapshot, StateView, WriteOverlay},
    typed::{Assignable, EqualityIndex, Key, ValueType},
    wal,
};

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
pub struct Writer<'db> {
    /// Db receiving the commit.
    pub(super) database: &'db mut Db,
    /// Parent snapshot the writer layers over (its base + frozen overlay).
    pub(super) parent: Arc<Snapshot>,
    /// Private mutable delta this writer accumulates.
    pub(super) delta: WriteOverlay,
    /// Writer transaction id (session-local until a dirty commit makes it
    /// durable).
    pub(super) transaction_id: TransactionId,
    /// Held single-writer advisory lock. Its [`Drop`] releases the lock when this
    /// transaction ends (on `rollback`, or on any early-return error path); a
    /// successful dirty [`Self::commit`] releases it explicitly with `drop` so a
    /// triggered auto-checkpoint can re-acquire it.
    pub(super) lock: WriterLock,
}

impl Writer<'_> {
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

    /// Applies a declarative [`Schema`] idempotently (register-or-get every
    /// declared item), returning the resolved [`Bound`] handle bag. Re-applying
    /// the same schema reuses existing ids; a name that already exists with a
    /// conflicting shape is a [`DbError::SchemaConflict`].
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on a shape conflict, an undeclared referenced name (an
    /// index's key, a projection's role/type), or id-allocation failure.
    ///
    /// # Performance
    ///
    /// This method is `O(declared items × log catalog)`.
    pub fn apply_schema(&mut self, schema: &Schema) -> Result<Bound, DbError> {
        let mut bound = Bound::default();
        for name in &schema.roles {
            let id = match self.merged().catalog().role_id(name) {
                Some(id) => id,
                None => self.register_role(name.clone())?,
            };
            bound.roles.insert(name.clone(), id);
        }
        for name in &schema.labels {
            let id = match self.merged().catalog().label_id(name) {
                Some(id) => id,
                None => self.register_label(name.clone())?,
            };
            bound.labels.insert(name.clone(), id);
        }
        for name in &schema.relation_types {
            let id = match self.merged().catalog().relation_type_id(name) {
                Some(id) => id,
                None => self.register_relation_type(name.clone())?,
            };
            bound.relation_types.insert(name.clone(), id);
        }
        for (name, family, value_type) in &schema.keys {
            let id = self.register_key_or_get(name, *family, *value_type)?;
            bound.keys.insert(name.clone(), (id, *value_type));
        }
        for (name, key_name) in &schema.equality_indexes {
            let (key_id, value_type) = *bound.keys.get(key_name).ok_or_else(|| {
                DbError::Catalog(crate::error::CatalogError::UnknownName {
                    kind: "property key",
                    name: key_name.clone(),
                })
            })?;
            let id = match self.merged().catalog().index_id(name) {
                Some(id) => id,
                None => self.define_index(
                    name.clone(),
                    IndexDefinition::PropertyEquality { key: key_id },
                )?,
            };
            bound
                .equality_indexes
                .insert(name.clone(), (id, value_type));
        }
        for spec in &schema.graph_projections {
            let id = match self.merged().catalog().projection_id(&spec.name) {
                Some(id) => id,
                None => self.define_graph_projection(spec, &bound)?,
            };
            bound.projections.insert(spec.name.clone(), id);
        }
        Ok(bound)
    }

    /// Registers a property key, or returns the existing id when the name is
    /// already present with a matching family and value type.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::SchemaConflict`] when the name exists with a different
    /// family or value type.
    ///
    /// # Performance
    ///
    /// This method is `O(log catalog)`.
    fn register_key_or_get(
        &mut self,
        name: &str,
        family: PropertyFamily,
        value_type: PropertyType,
    ) -> Result<PropertyKeyId, DbError> {
        let Some(existing) = self.merged().catalog().property_key_id(name) else {
            return self.register_property_key(name.to_owned(), family, value_type);
        };
        let matches = self
            .merged()
            .catalog()
            .property_key(existing)
            .is_some_and(|def| def.family == family && def.value_type == value_type);
        if matches {
            Ok(existing)
        } else {
            Err(DbError::Catalog(
                crate::error::CatalogError::SchemaConflict {
                    name: name.to_owned(),
                    reason: "property key family/value type differs from the existing catalog entry",
                },
            ))
        }
    }

    /// Defines a graph projection from a spec, resolving its relation-type and
    /// role names through `bound`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownName`] when a referenced role/type is unbound, or
    /// a definition error.
    ///
    /// # Performance
    ///
    /// This method is `O(relation-type count × log catalog)`.
    fn define_graph_projection(
        &mut self,
        spec: &GraphProjectionSpec,
        bound: &Bound,
    ) -> Result<ProjectionId, DbError> {
        let mut relation_types = BTreeSet::new();
        for name in &spec.relation_types {
            relation_types.insert(bound.relation_type(name)?);
        }
        let source_role = bound.role(&spec.source_role)?;
        let target_role = bound.role(&spec.target_role)?;
        self.define_projection(ProjectionDefinition::Graph(GraphProjectionDefinition {
            name: spec.name.clone(),
            relation_types,
            source_role,
            target_role,
        }))
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
    /// This method is `O(log n + degree)` via the reverse-adjacency index.
    pub(crate) fn tombstone_element(&mut self, id: ElementId) -> Result<(), DbError> {
        self.require_element(id)?;
        // Cascade: every incidence on the element — resolved in O(log n + degree)
        // through the reverse-adjacency index, not a full incidence scan — is
        // tombstoned too.
        let incidences: Vec<IncidenceId> = self
            .merged()
            .element_incidences(id)
            .into_iter()
            .map(|record| record.id)
            .collect();
        let base = self.parent.base_records();
        self.delta.tombstone_element(base, id);
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
    /// This method is `O(log n + degree)` via the reverse-adjacency index.
    pub(crate) fn tombstone_relation(&mut self, id: RelationId) -> Result<(), DbError> {
        self.require_relation(id)?;
        // Cascade: every incidence in the relation — resolved in O(log n + degree)
        // through the reverse-adjacency index, not a full incidence scan.
        let incidences: Vec<IncidenceId> = self
            .merged()
            .relation_incidences(id)
            .into_iter()
            .map(|record| record.id)
            .collect();
        let base = self.parent.base_records();
        self.delta.tombstone_relation(base, id);
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
    pub(crate) fn tombstone_incidence(&mut self, id: IncidenceId) -> Result<(), DbError> {
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
    pub(crate) fn add_element_label(
        &mut self,
        element: ElementId,
        label: LabelId,
    ) -> Result<(), DbError> {
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
    pub(crate) fn add_relation_label(
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
    pub(crate) fn set_property(
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
            .ok_or_else(|| DbError::unknown(key))?;
        if definition.family != subject.family() {
            return Err(DbError::Query(
                crate::error::QueryError::WrongPropertyFamily {
                    expected: definition.family,
                    actual: subject.family(),
                },
            ));
        }
        if definition.value_type != value.value_type() {
            return Err(DbError::Query(
                crate::error::QueryError::PropertyTypeMismatch {
                    expected: definition.value_type,
                    actual: value.value_type(),
                },
            ));
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
    pub(crate) fn remove_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Result<(), DbError> {
        self.require_subject(subject)?;
        if self.merged().catalog().property_key(key).is_none() {
            return Err(DbError::unknown(key));
        }
        self.delta
            .remove_property(self.parent.base_records(), subject, key);
        Ok(())
    }

    /// Resolves the property key an equality index covers.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownIndex`] when `index` is unknown, or an
    /// unsupported-query error when it is not a property-equality index.
    ///
    /// # Performance
    ///
    /// This method is `O(log index count)`.
    fn equality_index_key(&self, index: IndexId) -> Result<PropertyKeyId, DbError> {
        let view = self.merged();
        let entry = view
            .catalog()
            .index(index)
            .ok_or_else(|| DbError::unknown(index))?;
        match &entry.definition {
            IndexDefinition::PropertyEquality { key } => Ok(*key),
            _other => Err(DbError::unsupported(
                "reconcile requires a property-equality index",
            )),
        }
    }

    /// Inserts or updates the element whose value under `index` equals `value`,
    /// returning its canonical id — reused when an element already carries that
    /// identity value (id stable across reconcile), freshly minted (a never-reused
    /// id, with the identity property set) otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when `index` is not an equality index or the value
    /// type mismatches the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + value length)` — a probe plus, on a miss, a mint.
    pub fn upsert_element<T: ValueType>(
        &mut self,
        index: EqualityIndex<T>,
        value: impl Assignable<T>,
    ) -> Result<ElementId, DbError> {
        let value = value.into_value()?;
        let key = self.equality_index_key(index.id())?;
        let existing = self
            .merged()
            .property_equal(key, &value)
            .into_iter()
            .find_map(|subject| match subject {
                PropertySubject::Element(id) => Some(id),
                PropertySubject::Relation(_) | PropertySubject::Incidence(_) => None,
            });
        if let Some(id) = existing {
            return Ok(id);
        }
        let element = self.create_element()?;
        self.set_property(PropertySubject::Element(element), key, value)?;
        Ok(element)
    }

    /// Inserts or updates the relation whose value under `index` equals `value`,
    /// returning its canonical id. On a miss it mints the relation, sets its type
    /// and identity property, and creates one incidence per `(element, role)`
    /// endpoint; on a hit the existing relation (with its endpoints) is reused
    /// unchanged — the identity value encodes the endpoints, so they are immutable.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when `index` is not an equality index, the value type
    /// mismatches, or an endpoint element does not exist.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + endpoints)` — a probe plus, on a miss, a mint.
    pub fn upsert_relation<T: ValueType>(
        &mut self,
        index: EqualityIndex<T>,
        value: impl Assignable<T>,
        relation_type: RelationTypeId,
        endpoints: &[(ElementId, RoleId)],
    ) -> Result<RelationId, DbError> {
        let value = value.into_value()?;
        let key = self.equality_index_key(index.id())?;
        let existing = self
            .merged()
            .property_equal(key, &value)
            .into_iter()
            .find_map(|subject| match subject {
                PropertySubject::Relation(id) => Some(id),
                PropertySubject::Element(_) | PropertySubject::Incidence(_) => None,
            });
        if let Some(id) = existing {
            return Ok(id);
        }
        let relation = self.create_relation()?;
        self.set_relation_type(relation, relation_type)?;
        self.set_property(PropertySubject::Relation(relation), key, value)?;
        for (element, role) in endpoints {
            self.create_incidence(relation, *element, *role)?;
        }
        Ok(relation)
    }

    /// Tombstones every subject carried by `index` whose identity value is NOT in
    /// `keep`, cascading each subject's incidences in `O(degree)` via the
    /// reverse-adjacency index. The prune half of a reconcile: after upserting
    /// every desired subject, `retain` removes the vanished complement.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when `index` is not an equality index or a `keep` value
    /// type mismatches the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(family size + removed × degree)`.
    pub fn retain<T: ValueType, V: Assignable<T> + Copy>(
        &mut self,
        index: EqualityIndex<T>,
        keep: &[V],
    ) -> Result<(), DbError> {
        let key = self.equality_index_key(index.id())?;
        let mut keep_values: BTreeSet<PropertyValue> = BTreeSet::new();
        for value in keep {
            keep_values.insert((*value).into_value()?);
        }
        let stale: Vec<PropertySubject> = self
            .merged()
            .property_key_subjects(key)
            .into_iter()
            .filter(|(_subject, value)| !keep_values.contains(value))
            .map(|(subject, _value)| subject)
            .collect();
        for subject in stale {
            match subject {
                PropertySubject::Element(id) => self.tombstone_element(id)?,
                PropertySubject::Relation(id) => self.tombstone_relation(id)?,
                PropertySubject::Incidence(id) => self.tombstone_incidence(id)?,
            }
        }
        Ok(())
    }

    /// Sets a typed property on a subject; the value type is checked at compile
    /// time against the key.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the subject is absent, the value is out of range,
    /// or the value type mismatches the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log keys)`.
    pub fn set<T: ValueType>(
        &mut self,
        subject: impl Into<PropertySubject>,
        key: Key<T>,
        value: impl Assignable<T>,
    ) -> Result<(), DbError> {
        self.set_property(subject.into(), key.id(), value.into_value()?)
    }

    /// Removes a typed property from a subject.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the subject is absent or the key is unknown.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log keys)`.
    pub fn unset<T: ValueType>(
        &mut self,
        subject: impl Into<PropertySubject>,
        key: Key<T>,
    ) -> Result<(), DbError> {
        self.remove_property(subject.into(), key.id())
    }

    /// Adds a label to an element or relation subject.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the subject is absent, the label is unknown, or
    /// the subject is an incidence (incidences carry no labels).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log labels)`.
    pub fn add_label(
        &mut self,
        subject: impl Into<PropertySubject>,
        label: LabelId,
    ) -> Result<(), DbError> {
        match subject.into() {
            PropertySubject::Element(id) => self.add_element_label(id, label),
            PropertySubject::Relation(id) => self.add_relation_label(id, label),
            PropertySubject::Incidence(_) => {
                Err(DbError::unsupported("incidences do not carry labels"))
            }
        }
    }

    /// Tombstones any subject by id, cascading a relation's or element's
    /// incidences in `O(degree)` via the reverse-adjacency index.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the subject is not visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + degree)`.
    pub fn tombstone(&mut self, subject: impl Into<PropertySubject>) -> Result<(), DbError> {
        match subject.into() {
            PropertySubject::Element(id) => self.tombstone_element(id),
            PropertySubject::Relation(id) => self.tombstone_relation(id),
            PropertySubject::Incidence(id) => self.tombstone_incidence(id),
        }
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
    pub(crate) fn commit(mut self) -> Result<CommitSeq, DbError> {
        if self.delta.is_empty() {
            // Non-dirty commit: no append, no publish, no durable id advance.
            return Ok(self.parent.lsn());
        }
        let lsn = self
            .parent
            .lsn()
            .checked_next()
            .ok_or(DbError::Txn(crate::error::TxnError::CommitSeqOverflow))?;
        let (ops, blob) = self.delta.take_frame();
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
        // it (a partial move out of `self`, legal because `Writer` has
        // no `Drop` impl; the remaining `&mut Db` borrow stays live).
        drop(self.lock);
        self.database.maybe_auto_checkpoint()?;
        Ok(lsn)
    }

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
            Err(DbError::unknown(id))
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
            Err(DbError::unknown(id))
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
            Err(DbError::unknown(id))
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
            Err(DbError::unknown(id))
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
            Err(DbError::unknown(id))
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
            Err(DbError::unknown(id))
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
                .ok_or_else(|| DbError::unknown(*projection)),
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
            Err(DbError::unknown(id))
        }
    }
}
