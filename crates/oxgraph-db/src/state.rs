//! Canonical incidence-first database state.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    Catalog, DbError, ElementId, IncidenceId, IndexId, LabelId, ProjectionDefinition, ProjectionId,
    PropertyKeyId, RelationId, RelationTypeId, RoleId,
    catalog::{IndexDefinition, PropertyFamily, PropertyKeyDefinition},
    value::PropertyValue,
};

/// One visible canonical element.
///
/// # Performance
///
/// Cloning is `O(label count)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ElementRecord {
    /// Stable element identifier.
    pub id: ElementId,
    /// Labels assigned to this element.
    pub labels: BTreeSet<LabelId>,
}

/// One visible canonical relation.
///
/// # Performance
///
/// Cloning is `O(label count)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelationRecord {
    /// Stable relation identifier.
    pub id: RelationId,
    /// Optional relation type.
    pub relation_type: Option<RelationTypeId>,
    /// Labels assigned to this relation.
    pub labels: BTreeSet<LabelId>,
}

/// One visible incidence in canonical database coordinates.
///
/// # Performance
///
/// Copying and comparing this record are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IncidenceRecord {
    /// Stable incidence id.
    pub id: IncidenceId,
    /// Stable relation id containing the incidence.
    pub relation: RelationId,
    /// Stable element id participating in the relation.
    pub element: ElementId,
    /// Structural role of the incidence.
    pub role: RoleId,
}

/// Subject that can own properties.
///
/// # Performance
///
/// Copying, comparing, ordering, and hashing are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PropertySubject {
    /// Element property subject.
    Element(ElementId),
    /// Relation property subject.
    Relation(RelationId),
    /// Incidence property subject.
    Incidence(IncidenceId),
}

impl PropertySubject {
    /// Returns the property family for this subject.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn family(self) -> PropertyFamily {
        match self {
            Self::Element(_id) => PropertyFamily::Element,
            Self::Relation(_id) => PropertyFamily::Relation,
            Self::Incidence(_id) => PropertyFamily::Incidence,
        }
    }
}

/// The nine monotonic id allocators, captured for the store header in a fixed
/// order.
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NextIds {
    /// Next element id candidate.
    pub(crate) element: ElementId,
    /// Next relation id candidate.
    pub(crate) relation: RelationId,
    /// Next incidence id candidate.
    pub(crate) incidence: IncidenceId,
    /// Next role id candidate.
    pub(crate) role: RoleId,
    /// Next label id candidate.
    pub(crate) label: LabelId,
    /// Next relation-type id candidate.
    pub(crate) relation_type: RelationTypeId,
    /// Next property-key id candidate.
    pub(crate) property_key: PropertyKeyId,
    /// Next projection id candidate.
    pub(crate) projection: ProjectionId,
    /// Next index id candidate.
    pub(crate) index: IndexId,
}

/// Decoded database parts handed to [`DatabaseState::from_parts`] by the store
/// open path; bundling them keeps the constructor within the argument budget.
pub(crate) struct DatabaseParts {
    /// Visible elements keyed by id.
    pub(crate) elements: BTreeMap<ElementId, ElementRecord>,
    /// Visible relations keyed by id.
    pub(crate) relations: BTreeMap<RelationId, RelationRecord>,
    /// Visible incidences keyed by id.
    pub(crate) incidences: BTreeMap<IncidenceId, IncidenceRecord>,
    /// Property values keyed by subject and property key.
    pub(crate) properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>,
    /// Catalog metadata.
    pub(crate) catalog: Catalog,
    /// The nine monotonic id allocators.
    pub(crate) next: NextIds,
}

/// Durable canonical database state.
#[derive(Clone, Debug)]
pub(crate) struct DatabaseState {
    /// Visible elements keyed by ID.
    elements: BTreeMap<ElementId, ElementRecord>,
    /// Visible relations keyed by ID.
    relations: BTreeMap<RelationId, RelationRecord>,
    /// Visible incidences keyed by ID.
    incidences: BTreeMap<IncidenceId, IncidenceRecord>,
    /// Property values keyed by subject and property key.
    properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>,
    /// Catalog metadata.
    catalog: Catalog,
    /// Next element ID candidate.
    next_element: ElementId,
    /// Next relation ID candidate.
    next_relation: RelationId,
    /// Next incidence ID candidate.
    next_incidence: IncidenceId,
    /// Next role ID candidate.
    next_role: RoleId,
    /// Next label ID candidate.
    next_label: LabelId,
    /// Next relation type ID candidate.
    next_relation_type: RelationTypeId,
    /// Next property key ID candidate.
    next_property_key: PropertyKeyId,
    /// Next projection ID candidate.
    next_projection: ProjectionId,
    /// Next index ID candidate.
    next_index: IndexId,
    /// Derived membership index: element ids per label (rebuilt on open/commit).
    label_members: BTreeMap<LabelId, BTreeSet<ElementId>>,
    /// Derived membership index: relation ids per relation type.
    relation_type_members: BTreeMap<RelationTypeId, BTreeSet<RelationId>>,
}

impl PartialEq for DatabaseState {
    fn eq(&self, other: &Self) -> bool {
        // The membership indexes are derived from the records, so equality
        // compares only the canonical data and id allocators.
        self.elements == other.elements
            && self.relations == other.relations
            && self.incidences == other.incidences
            && self.properties == other.properties
            && self.catalog == other.catalog
            && self.next_element == other.next_element
            && self.next_relation == other.next_relation
            && self.next_incidence == other.next_incidence
            && self.next_role == other.next_role
            && self.next_label == other.next_label
            && self.next_relation_type == other.next_relation_type
            && self.next_property_key == other.next_property_key
            && self.next_projection == other.next_projection
            && self.next_index == other.next_index
    }
}

impl Eq for DatabaseState {}

impl DatabaseState {
    /// Creates an empty canonical state.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub(crate) const fn empty() -> Self {
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog: Catalog::empty(),
            next_element: ElementId::new(1),
            next_relation: RelationId::new(1),
            next_incidence: IncidenceId::new(1),
            next_role: RoleId::new(1),
            next_label: LabelId::new(1),
            next_relation_type: RelationTypeId::new(1),
            next_property_key: PropertyKeyId::new(1),
            next_projection: ProjectionId::new(1),
            next_index: IndexId::new(1),
            label_members: BTreeMap::new(),
            relation_type_members: BTreeMap::new(),
        }
    }

    /// Reconstructs a state from decoded, canonical-id-preserving parts. Used by
    /// the store open path; callers must pass internally consistent collections.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`; it moves the supplied collections.
    pub(crate) fn from_parts(parts: DatabaseParts) -> Self {
        let mut state = Self {
            elements: parts.elements,
            relations: parts.relations,
            incidences: parts.incidences,
            properties: parts.properties,
            catalog: parts.catalog,
            next_element: parts.next.element,
            next_relation: parts.next.relation,
            next_incidence: parts.next.incidence,
            next_role: parts.next.role,
            next_label: parts.next.label,
            next_relation_type: parts.next.relation_type,
            next_property_key: parts.next.property_key,
            next_projection: parts.next.projection,
            next_index: parts.next.index,
            label_members: BTreeMap::new(),
            relation_type_members: BTreeMap::new(),
        };
        state.rebuild_membership_indexes();
        state
    }

    /// Rebuilds the derived label and relation-type membership indexes from the
    /// canonical records. Called after a batch of mutations (commit) and on open.
    ///
    /// # Performance
    ///
    /// This method is `O(elements + relations + their labels)`.
    pub(crate) fn rebuild_membership_indexes(&mut self) {
        let mut label_members: BTreeMap<LabelId, BTreeSet<ElementId>> = BTreeMap::new();
        for record in self.elements.values() {
            for label in &record.labels {
                label_members.entry(*label).or_default().insert(record.id);
            }
        }
        let mut relation_type_members: BTreeMap<RelationTypeId, BTreeSet<RelationId>> =
            BTreeMap::new();
        for record in self.relations.values() {
            if let Some(relation_type) = record.relation_type {
                relation_type_members
                    .entry(relation_type)
                    .or_default()
                    .insert(record.id);
            }
        }
        self.label_members = label_members;
        self.relation_type_members = relation_type_members;
    }

    /// Returns the nine monotonic id allocators.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn next_ids(&self) -> NextIds {
        NextIds {
            element: self.next_element,
            relation: self.next_relation,
            incidence: self.next_incidence,
            role: self.next_role,
            label: self.next_label,
            relation_type: self.next_relation_type,
            property_key: self.next_property_key,
            projection: self.next_projection,
            index: self.next_index,
        }
    }

    /// Iterates every visible `(subject, key, value)` property triple in
    /// subject-then-key order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`; a full walk is `O(total properties)`.
    pub(crate) fn property_iter(
        &self,
    ) -> impl Iterator<Item = (PropertySubject, PropertyKeyId, &PropertyValue)> {
        self.properties.iter().flat_map(|(subject, values)| {
            values
                .iter()
                .map(move |(key, value)| (*subject, *key, value))
        })
    }

    /// Returns catalog metadata.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Returns whether an element exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub(crate) fn contains_element(&self, id: ElementId) -> bool {
        self.elements.contains_key(&id)
    }

    /// Returns whether a relation exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub(crate) fn contains_relation(&self, id: RelationId) -> bool {
        self.relations.contains_key(&id)
    }

    /// Returns whether an incidence exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub(crate) fn contains_incidence(&self, id: IncidenceId) -> bool {
        self.incidences.contains_key(&id)
    }

    /// Returns the number of visible elements.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) fn element_count(&self) -> usize {
        self.elements.len()
    }

    /// Returns the number of visible relations.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) fn relation_count(&self) -> usize {
        self.relations.len()
    }

    /// Returns the number of visible incidences.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) fn incidence_count(&self) -> usize {
        self.incidences.len()
    }

    /// Iterates elements in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub(crate) fn elements(&self) -> impl Iterator<Item = &ElementRecord> {
        self.elements.values()
    }

    /// Iterates relations in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub(crate) fn relations(&self) -> impl Iterator<Item = &RelationRecord> {
        self.relations.values()
    }

    /// Iterates incidences in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub(crate) fn incidences(&self) -> impl Iterator<Item = &IncidenceRecord> {
        self.incidences.values()
    }

    /// Returns one element record.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub(crate) fn element(&self, id: ElementId) -> Option<&ElementRecord> {
        self.elements.get(&id)
    }

    /// Returns one relation record.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub(crate) fn relation(&self, id: RelationId) -> Option<&RelationRecord> {
        self.relations.get(&id)
    }

    /// Returns one incidence record.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub(crate) fn incidence(&self, id: IncidenceId) -> Option<&IncidenceRecord> {
        self.incidences.get(&id)
    }

    /// Returns all incidences for a relation.
    ///
    /// # Performance
    ///
    /// This method is `O(i)` for visible incidence count.
    pub(crate) fn relation_incidences(
        &self,
        id: RelationId,
    ) -> impl Iterator<Item = &IncidenceRecord> {
        self.incidences
            .values()
            .filter(move |record| record.relation == id)
    }

    /// Returns all incidences for an element.
    ///
    /// # Performance
    ///
    /// This method is `O(i)` for visible incidence count.
    pub(crate) fn element_incidences(
        &self,
        id: ElementId,
    ) -> impl Iterator<Item = &IncidenceRecord> {
        self.incidences
            .values()
            .filter(move |record| record.element == id)
    }

    /// Creates an element.
    pub(crate) fn create_element(&mut self) -> Result<ElementId, DbError> {
        let id = self.next_element;
        self.next_element = id.checked_next().ok_or(DbError::IdOverflow)?;
        let previous = self.elements.insert(
            id,
            ElementRecord {
                id,
                labels: BTreeSet::new(),
            },
        );
        if previous.is_some() {
            return Err(DbError::DuplicateId);
        }
        Ok(id)
    }

    /// Creates a relation.
    pub(crate) fn create_relation(&mut self) -> Result<RelationId, DbError> {
        let id = self.next_relation;
        self.next_relation = id.checked_next().ok_or(DbError::IdOverflow)?;
        let previous = self.relations.insert(
            id,
            RelationRecord {
                id,
                relation_type: None,
                labels: BTreeSet::new(),
            },
        );
        if previous.is_some() {
            return Err(DbError::DuplicateId);
        }
        Ok(id)
    }

    /// Creates an incidence.
    pub(crate) fn create_incidence(
        &mut self,
        relation: RelationId,
        element: ElementId,
        role: RoleId,
    ) -> Result<IncidenceId, DbError> {
        self.require_relation(relation)?;
        self.require_element(element)?;
        self.require_role(role)?;
        let id = self.next_incidence;
        self.next_incidence = id.checked_next().ok_or(DbError::IdOverflow)?;
        let previous = self.incidences.insert(
            id,
            IncidenceRecord {
                id,
                relation,
                element,
                role,
            },
        );
        if previous.is_some() {
            return Err(DbError::DuplicateId);
        }
        Ok(id)
    }

    /// Tombstones an element and its incidences.
    pub(crate) fn tombstone_element(&mut self, id: ElementId) -> Result<(), DbError> {
        self.elements
            .remove(&id)
            .ok_or(DbError::UnknownElement { id })?;
        self.properties.remove(&PropertySubject::Element(id));
        self.remove_incidences(|record| record.element == id);
        Ok(())
    }

    /// Tombstones a relation and its incidences.
    pub(crate) fn tombstone_relation(&mut self, id: RelationId) -> Result<(), DbError> {
        self.relations
            .remove(&id)
            .ok_or(DbError::UnknownRelation { id })?;
        self.properties.remove(&PropertySubject::Relation(id));
        self.remove_incidences(|record| record.relation == id);
        Ok(())
    }

    /// Tombstones one incidence.
    pub(crate) fn tombstone_incidence(&mut self, id: IncidenceId) -> Result<(), DbError> {
        self.incidences
            .remove(&id)
            .ok_or(DbError::UnknownIncidence { id })?;
        self.properties.remove(&PropertySubject::Incidence(id));
        Ok(())
    }

    /// Registers a role.
    pub(crate) fn register_role(&mut self, name: String) -> Result<RoleId, DbError> {
        let id = self.next_role;
        self.next_role = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.catalog.insert_role(id, name)?;
        Ok(id)
    }

    /// Registers a label.
    pub(crate) fn register_label(&mut self, name: String) -> Result<LabelId, DbError> {
        let id = self.next_label;
        self.next_label = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.catalog.insert_label(id, name)?;
        Ok(id)
    }

    /// Registers a relation type.
    pub(crate) fn register_relation_type(
        &mut self,
        name: String,
    ) -> Result<RelationTypeId, DbError> {
        let id = self.next_relation_type;
        self.next_relation_type = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.catalog.insert_relation_type(id, name)?;
        Ok(id)
    }

    /// Registers a property key.
    pub(crate) fn register_property_key(
        &mut self,
        name: String,
        family: PropertyFamily,
        value_type: crate::PropertyType,
    ) -> Result<PropertyKeyId, DbError> {
        let id = self.next_property_key;
        self.next_property_key = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.catalog.insert_property_key(PropertyKeyDefinition {
            id,
            name,
            family,
            value_type,
        })?;
        Ok(id)
    }

    /// Defines a physical projection.
    pub(crate) fn define_projection(
        &mut self,
        definition: ProjectionDefinition,
    ) -> Result<ProjectionId, DbError> {
        self.validate_projection_definition(&definition)?;
        let id = self.next_projection;
        self.next_projection = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.catalog.insert_projection(id, definition)?;
        Ok(id)
    }

    /// Defines an index.
    pub(crate) fn define_index(
        &mut self,
        name: String,
        definition: IndexDefinition,
    ) -> Result<IndexId, DbError> {
        self.validate_index_definition(&definition)?;
        let id = self.next_index;
        self.next_index = id.checked_next().ok_or(DbError::IdOverflow)?;
        self.catalog.insert_index(id, name, definition)?;
        Ok(id)
    }

    /// Assigns a label to an element.
    pub(crate) fn add_element_label(
        &mut self,
        element: ElementId,
        label: LabelId,
    ) -> Result<(), DbError> {
        self.require_label(label)?;
        let record = self
            .elements
            .get_mut(&element)
            .ok_or(DbError::UnknownElement { id: element })?;
        record.labels.insert(label);
        Ok(())
    }

    /// Assigns a label to a relation.
    pub(crate) fn add_relation_label(
        &mut self,
        relation: RelationId,
        label: LabelId,
    ) -> Result<(), DbError> {
        self.require_label(label)?;
        let record = self
            .relations
            .get_mut(&relation)
            .ok_or(DbError::UnknownRelation { id: relation })?;
        record.labels.insert(label);
        Ok(())
    }

    /// Sets a relation type.
    pub(crate) fn set_relation_type(
        &mut self,
        relation: RelationId,
        relation_type: RelationTypeId,
    ) -> Result<(), DbError> {
        self.require_relation_type(relation_type)?;
        let record = self
            .relations
            .get_mut(&relation)
            .ok_or(DbError::UnknownRelation { id: relation })?;
        record.relation_type = Some(relation_type);
        Ok(())
    }

    /// Sets a typed property value.
    pub(crate) fn set_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: PropertyValue,
    ) -> Result<(), DbError> {
        self.require_subject(subject)?;
        let definition = self
            .catalog
            .property_key(key)
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
        self.properties
            .entry(subject)
            .or_default()
            .insert(key, value);
        Ok(())
    }

    /// Removes one property value.
    pub(crate) fn remove_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Result<(), DbError> {
        self.require_subject(subject)?;
        self.require_property_key(key)?;
        if let Some(values) = self.properties.get_mut(&subject) {
            values.remove(&key);
            if values.is_empty() {
                self.properties.remove(&subject);
            }
        }
        Ok(())
    }

    /// Returns one property value.
    #[must_use]
    pub(crate) fn property(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<&PropertyValue> {
        self.properties
            .get(&subject)
            .and_then(|values| values.get(&key))
    }

    /// Returns subjects whose property equals `value`.
    pub(crate) fn property_equal(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties
            .iter()
            .filter_map(|(subject, values)| (values.get(&key) == Some(value)).then_some(*subject))
            .collect()
    }

    /// Returns subjects whose typed property equals `value`.
    pub(crate) fn typed_property_equal(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value(key, value)?;
        Ok(self.property_equal(key, value))
    }

    /// Returns subjects in `family` whose typed property equals `value`.
    pub(crate) fn typed_property_equal_for_family(
        &self,
        key: PropertyKeyId,
        family: PropertyFamily,
        value: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value_for_family(key, family, value)?;
        Ok(self.property_equal(key, value))
    }

    /// Returns subjects whose ordered property falls inside an inclusive range.
    pub(crate) fn property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties
            .iter()
            .filter_map(|(subject, values)| {
                let value = values.get(&key)?;
                (value >= min && value <= max).then_some(*subject)
            })
            .collect()
    }

    /// Returns subjects whose typed property falls inside an inclusive range.
    pub(crate) fn typed_property_range(
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

    /// Returns subjects matching an ordered typed property tuple.
    pub(crate) fn typed_property_composite_equal(
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
            .properties
            .iter()
            .filter_map(|(subject, property_values)| {
                keys.iter()
                    .copied()
                    .zip(values)
                    .all(|(key, value)| property_values.get(&key) == Some(value))
                    .then_some(*subject)
            })
            .collect())
    }

    /// Returns elements that have `label`.
    pub(crate) fn elements_with_label(&self, label: LabelId) -> Vec<ElementId> {
        self.label_members
            .get(&label)
            .map(|members| members.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Returns relations that have `relation_type`.
    pub(crate) fn relations_with_type(&self, relation_type: RelationTypeId) -> Vec<RelationId> {
        self.relation_type_members
            .get(&relation_type)
            .map(|members| members.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Validates all persisted references.
    pub(crate) fn validate(&self) -> Result<(), DbError> {
        for record in self.incidences.values() {
            self.require_relation(record.relation)?;
            self.require_element(record.element)?;
            self.require_role(record.role)?;
        }
        for record in self.elements.values() {
            self.require_labels(&record.labels)?;
        }
        for record in self.relations.values() {
            self.require_labels(&record.labels)?;
            if let Some(relation_type) = record.relation_type {
                self.require_relation_type(relation_type)?;
            }
        }
        self.validate_properties()?;
        self.validate_catalog_definitions()
    }

    /// Requires an element to exist.
    fn require_element(&self, id: ElementId) -> Result<(), DbError> {
        self.elements
            .contains_key(&id)
            .then_some(())
            .ok_or(DbError::UnknownElement { id })
    }

    /// Requires a relation to exist.
    fn require_relation(&self, id: RelationId) -> Result<(), DbError> {
        self.relations
            .contains_key(&id)
            .then_some(())
            .ok_or(DbError::UnknownRelation { id })
    }

    /// Requires an incidence to exist.
    fn require_incidence(&self, id: IncidenceId) -> Result<(), DbError> {
        self.incidences
            .contains_key(&id)
            .then_some(())
            .ok_or(DbError::UnknownIncidence { id })
    }

    /// Requires a role to exist.
    fn require_role(&self, id: RoleId) -> Result<(), DbError> {
        self.catalog
            .role(id)
            .is_some()
            .then_some(())
            .ok_or(DbError::UnknownRole { id })
    }

    /// Requires a label to exist.
    fn require_label(&self, id: LabelId) -> Result<(), DbError> {
        self.catalog
            .label(id)
            .is_some()
            .then_some(())
            .ok_or(DbError::UnknownLabel { id })
    }

    /// Requires a relation type to exist.
    fn require_relation_type(&self, id: RelationTypeId) -> Result<(), DbError> {
        self.catalog
            .relation_type(id)
            .is_some()
            .then_some(())
            .ok_or(DbError::UnknownRelationType { id })
    }

    /// Requires a property key to exist.
    fn require_property_key(&self, id: PropertyKeyId) -> Result<(), DbError> {
        self.catalog
            .property_key(id)
            .is_some()
            .then_some(())
            .ok_or(DbError::UnknownPropertyKey { id })
    }

    /// Requires a property subject to exist.
    fn require_subject(&self, subject: PropertySubject) -> Result<(), DbError> {
        match subject {
            PropertySubject::Element(id) => self.require_element(id),
            PropertySubject::Relation(id) => self.require_relation(id),
            PropertySubject::Incidence(id) => self.require_incidence(id),
        }
    }

    /// Requires all labels to exist.
    fn require_labels(&self, labels: &BTreeSet<LabelId>) -> Result<(), DbError> {
        for label in labels {
            self.require_label(*label)?;
        }
        Ok(())
    }

    /// Removes matching incidences and their properties.
    fn remove_incidences(&mut self, mut should_remove: impl FnMut(&IncidenceRecord) -> bool) {
        let removed: Vec<_> = self
            .incidences
            .values()
            .filter_map(|record| should_remove(record).then_some(record.id))
            .collect();
        for id in removed {
            self.incidences.remove(&id);
            self.properties.remove(&PropertySubject::Incidence(id));
        }
    }

    /// Validates one projection definition.
    fn validate_projection_definition(
        &self,
        definition: &ProjectionDefinition,
    ) -> Result<(), DbError> {
        match definition {
            ProjectionDefinition::Graph(graph) => {
                self.require_role(graph.source_role)?;
                self.require_role(graph.target_role)?;
                self.require_relation_types(&graph.relation_types)
            }
            ProjectionDefinition::Hypergraph(hypergraph) => {
                self.require_roles(&hypergraph.source_roles)?;
                self.require_roles(&hypergraph.target_roles)?;
                self.require_relation_types(&hypergraph.relation_types)
            }
        }
    }

    /// Validates one index definition.
    fn validate_index_definition(&self, definition: &IndexDefinition) -> Result<(), DbError> {
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
            IndexDefinition::Projection { projection } => self
                .catalog
                .projection(*projection)
                .is_some()
                .then_some(())
                .ok_or(DbError::UnknownProjection { id: *projection }),
        }
    }

    /// Requires every role in a set to exist.
    fn require_roles(&self, roles: &BTreeSet<RoleId>) -> Result<(), DbError> {
        for role in roles {
            self.require_role(*role)?;
        }
        Ok(())
    }

    /// Requires every relation type in a set to exist.
    fn require_relation_types(
        &self,
        relation_types: &BTreeSet<RelationTypeId>,
    ) -> Result<(), DbError> {
        for relation_type in relation_types {
            self.require_relation_type(*relation_type)?;
        }
        Ok(())
    }

    /// Validates property maps.
    fn validate_properties(&self) -> Result<(), DbError> {
        for (subject, values) in &self.properties {
            self.require_subject(*subject)?;
            for (key, value) in values {
                self.validate_property_value(*subject, *key, value)?;
            }
        }
        Ok(())
    }

    /// Validates a lookup value against a property key schema.
    fn validate_lookup_value(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<(), DbError> {
        let definition = self
            .catalog
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

    /// Validates a lookup value and subject family against a property key schema.
    pub(crate) fn validate_lookup_value_for_family(
        &self,
        key: PropertyKeyId,
        family: PropertyFamily,
        value: &PropertyValue,
    ) -> Result<(), DbError> {
        let definition = self
            .catalog
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

    /// Validates one property value against the catalog schema.
    fn validate_property_value(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<(), DbError> {
        let definition = self
            .catalog
            .property_key(key)
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
        Ok(())
    }

    /// Validates catalog projection and index references.
    fn validate_catalog_definitions(&self) -> Result<(), DbError> {
        for entry in self.catalog.projections() {
            self.validate_projection_definition(&entry.definition)?;
        }
        for entry in self.catalog.indexes() {
            self.validate_index_definition(&entry.definition)?;
        }
        Ok(())
    }
}
