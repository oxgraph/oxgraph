//! Catalog metadata for names, projections, property keys, and indexes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    DbError, IndexId, LabelId, ProjectionId, PropertyKeyId, RelationTypeId, RoleId,
    value::PropertyType,
};

/// Catalog entry for one structural incidence role.
///
/// # Performance
///
/// Cloning is `O(name length)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleDefinition {
    /// Stable role identifier.
    pub id: RoleId,
    /// Human-readable unique role name.
    pub name: String,
}

/// Catalog entry for one element or relation label.
///
/// # Performance
///
/// Cloning is `O(name length)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LabelDefinition {
    /// Stable label identifier.
    pub id: LabelId,
    /// Human-readable unique label name.
    pub name: String,
}

/// Catalog entry for one relation type.
///
/// # Performance
///
/// Cloning is `O(name length)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelationTypeDefinition {
    /// Stable relation type identifier.
    pub id: RelationTypeId,
    /// Human-readable unique relation type name.
    pub name: String,
}

/// Subject family accepted by a property key.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PropertyFamily {
    /// Property applies to canonical elements.
    Element,
    /// Property applies to canonical relations.
    Relation,
    /// Property applies to canonical incidences.
    Incidence,
}

/// Catalog entry for one typed property key.
///
/// # Performance
///
/// Cloning is `O(name length)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PropertyKeyDefinition {
    /// Stable property key identifier.
    pub id: PropertyKeyId,
    /// Human-readable unique key name.
    pub name: String,
    /// Subject family this key can be attached to.
    pub family: PropertyFamily,
    /// Required scalar value type.
    pub value_type: PropertyType,
}

/// Graph projection definition.
///
/// Graph projections materialize binary relations as CSR outgoing and CSC
/// incoming arrays over canonical topology IDs.
///
/// # Performance
///
/// Cloning is `O(r)` for `r` selected relation types plus the name length.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphProjectionDefinition {
    /// Unique projection name.
    pub name: String,
    /// Relation types visible as binary graph edges.
    pub relation_types: BTreeSet<RelationTypeId>,
    /// Role identifying the source endpoint.
    pub source_role: RoleId,
    /// Role identifying the target endpoint.
    pub target_role: RoleId,
}

/// Hypergraph projection definition.
///
/// Hypergraph projections materialize many-participant directed relations as
/// BCSR-style relation-major and vertex-major arrays.
///
/// # Performance
///
/// Cloning is `O(r + s + t)` for relation type and role set sizes plus the
/// name length.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HypergraphProjectionDefinition {
    /// Unique projection name.
    pub name: String,
    /// Relation types visible as hyperedges.
    pub relation_types: BTreeSet<RelationTypeId>,
    /// Roles treated as source-side participants.
    pub source_roles: BTreeSet<RoleId>,
    /// Roles treated as target-side participants.
    pub target_roles: BTreeSet<RoleId>,
}

/// Physical projection definition stored in the catalog.
///
/// # Performance
///
/// Cloning is `O(definition size)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectionDefinition {
    /// Binary graph projection.
    Graph(GraphProjectionDefinition),
    /// Directed hypergraph projection.
    Hypergraph(HypergraphProjectionDefinition),
}

impl ProjectionDefinition {
    /// Returns the unique projection name.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Graph(definition) => &definition.name,
            Self::Hypergraph(definition) => &definition.name,
        }
    }
}

/// Index definition stored in the catalog.
///
/// # Performance
///
/// Cloning is `O(key count)` for composite indexes and `O(1)` otherwise.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IndexDefinition {
    /// Element label membership index.
    Label {
        /// Indexed label.
        label: LabelId,
    },
    /// Relation type membership index.
    RelationType {
        /// Indexed relation type.
        relation_type: RelationTypeId,
    },
    /// Equality index over one property key.
    PropertyEquality {
        /// Indexed property key.
        key: PropertyKeyId,
    },
    /// Range index over one ordered property key.
    PropertyRange {
        /// Indexed property key.
        key: PropertyKeyId,
    },
    /// Composite equality index over ordered property keys.
    CompositeEquality {
        /// Indexed property keys in tuple order.
        keys: Vec<PropertyKeyId>,
    },
    /// Projection-materialization index metadata.
    Projection {
        /// Indexed projection.
        projection: ProjectionId,
    },
}

/// Catalog entry for one index.
///
/// # Performance
///
/// Cloning is `O(name length + definition size)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexEntry {
    /// Stable index identifier.
    pub id: IndexId,
    /// Human-readable unique index name.
    pub name: String,
    /// Logical index definition.
    pub definition: IndexDefinition,
}

/// Catalog entry for one projection.
///
/// # Performance
///
/// Cloning is `O(name length + definition size)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionEntry {
    /// Stable projection identifier.
    pub id: ProjectionId,
    /// Physical projection definition.
    pub definition: ProjectionDefinition,
}

/// Database catalog for names, schemas, projections, and indexes.
///
/// # Performance
///
/// Cloning is `O(catalog entries + string bytes)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Catalog {
    /// Roles by stable ID.
    #[serde(with = "serde_btree_map_vec")]
    roles: BTreeMap<RoleId, RoleDefinition>,
    /// Role IDs by name.
    role_names: BTreeMap<String, RoleId>,
    /// Labels by stable ID.
    #[serde(with = "serde_btree_map_vec")]
    labels: BTreeMap<LabelId, LabelDefinition>,
    /// Label IDs by name.
    label_names: BTreeMap<String, LabelId>,
    /// Relation types by stable ID.
    #[serde(with = "serde_btree_map_vec")]
    relation_types: BTreeMap<RelationTypeId, RelationTypeDefinition>,
    /// Relation type IDs by name.
    relation_type_names: BTreeMap<String, RelationTypeId>,
    /// Property keys by stable ID.
    #[serde(with = "serde_btree_map_vec")]
    property_keys: BTreeMap<PropertyKeyId, PropertyKeyDefinition>,
    /// Property key IDs by name.
    property_key_names: BTreeMap<String, PropertyKeyId>,
    /// Projections by stable ID.
    #[serde(with = "serde_btree_map_vec")]
    projections: BTreeMap<ProjectionId, ProjectionEntry>,
    /// Projection IDs by name.
    projection_names: BTreeMap<String, ProjectionId>,
    /// Indexes by stable ID.
    #[serde(with = "serde_btree_map_vec")]
    indexes: BTreeMap<IndexId, IndexEntry>,
    /// Index IDs by name.
    index_names: BTreeMap<String, IndexId>,
}

/// Serde helper for `BTreeMap` values keyed by non-string IDs.
mod serde_btree_map_vec {
    /// Serializes a map as an ordered entry array.
    pub(super) fn serialize<S, K, V>(
        map: &std::collections::BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
        K: serde::Serialize,
        V: serde::Serialize,
    {
        serde::Serialize::serialize(&map.iter().collect::<Vec<_>>(), serializer)
    }

    /// Deserializes a map from an ordered entry array.
    pub(super) fn deserialize<'de, D, K, V>(
        deserializer: D,
    ) -> Result<std::collections::BTreeMap<K, V>, D::Error>
    where
        D: serde::Deserializer<'de>,
        K: Ord + serde::de::DeserializeOwned,
        V: serde::de::DeserializeOwned,
    {
        <Vec<(K, V)> as serde::Deserialize>::deserialize(deserializer)
            .map(|entries| entries.into_iter().collect())
    }
}

impl Catalog {
    /// Creates an empty catalog.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub(crate) const fn empty() -> Self {
        Self {
            roles: BTreeMap::new(),
            role_names: BTreeMap::new(),
            labels: BTreeMap::new(),
            label_names: BTreeMap::new(),
            relation_types: BTreeMap::new(),
            relation_type_names: BTreeMap::new(),
            property_keys: BTreeMap::new(),
            property_key_names: BTreeMap::new(),
            projections: BTreeMap::new(),
            projection_names: BTreeMap::new(),
            indexes: BTreeMap::new(),
            index_names: BTreeMap::new(),
        }
    }

    /// Returns a role definition.
    ///
    /// # Performance
    ///
    /// This method is `O(log r)`.
    #[must_use]
    pub fn role(&self, id: RoleId) -> Option<&RoleDefinition> {
        self.roles.get(&id)
    }

    /// Returns a label definition.
    ///
    /// # Performance
    ///
    /// This method is `O(log l)`.
    #[must_use]
    pub fn label(&self, id: LabelId) -> Option<&LabelDefinition> {
        self.labels.get(&id)
    }

    /// Returns a relation type definition.
    ///
    /// # Performance
    ///
    /// This method is `O(log t)`.
    #[must_use]
    pub fn relation_type(&self, id: RelationTypeId) -> Option<&RelationTypeDefinition> {
        self.relation_types.get(&id)
    }

    /// Returns a property key definition.
    ///
    /// # Performance
    ///
    /// This method is `O(log p)`.
    #[must_use]
    pub fn property_key(&self, id: PropertyKeyId) -> Option<&PropertyKeyDefinition> {
        self.property_keys.get(&id)
    }

    /// Returns a projection entry.
    ///
    /// # Performance
    ///
    /// This method is `O(log p)`.
    #[must_use]
    pub fn projection(&self, id: ProjectionId) -> Option<&ProjectionEntry> {
        self.projections.get(&id)
    }

    /// Returns an index entry.
    ///
    /// # Performance
    ///
    /// This method is `O(log i)`.
    #[must_use]
    pub fn index(&self, id: IndexId) -> Option<&IndexEntry> {
        self.indexes.get(&id)
    }

    /// Resolves a role name.
    ///
    /// # Performance
    ///
    /// This method is `O(log r + name length)`.
    #[must_use]
    pub fn role_id(&self, name: &str) -> Option<RoleId> {
        self.role_names.get(name).copied()
    }

    /// Resolves a label name.
    ///
    /// # Performance
    ///
    /// This method is `O(log l + name length)`.
    #[must_use]
    pub fn label_id(&self, name: &str) -> Option<LabelId> {
        self.label_names.get(name).copied()
    }

    /// Resolves a relation type name.
    ///
    /// # Performance
    ///
    /// This method is `O(log t + name length)`.
    #[must_use]
    pub fn relation_type_id(&self, name: &str) -> Option<RelationTypeId> {
        self.relation_type_names.get(name).copied()
    }

    /// Resolves a property key name.
    ///
    /// # Performance
    ///
    /// This method is `O(log p + name length)`.
    #[must_use]
    pub fn property_key_id(&self, name: &str) -> Option<PropertyKeyId> {
        self.property_key_names.get(name).copied()
    }

    /// Resolves a projection name.
    ///
    /// # Performance
    ///
    /// This method is `O(log p + name length)`.
    #[must_use]
    pub fn projection_id(&self, name: &str) -> Option<ProjectionId> {
        self.projection_names.get(name).copied()
    }

    /// Resolves an index name.
    ///
    /// # Performance
    ///
    /// This method is `O(log i + name length)`.
    #[must_use]
    pub fn index_id(&self, name: &str) -> Option<IndexId> {
        self.index_names.get(name).copied()
    }

    /// Iterates role definitions in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub fn roles(&self) -> impl Iterator<Item = &RoleDefinition> {
        self.roles.values()
    }

    /// Iterates label definitions in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub fn labels(&self) -> impl Iterator<Item = &LabelDefinition> {
        self.labels.values()
    }

    /// Iterates relation type definitions in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub fn relation_types(&self) -> impl Iterator<Item = &RelationTypeDefinition> {
        self.relation_types.values()
    }

    /// Iterates property key definitions in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub fn property_keys(&self) -> impl Iterator<Item = &PropertyKeyDefinition> {
        self.property_keys.values()
    }

    /// Iterates projection entries in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub fn projections(&self) -> impl Iterator<Item = &ProjectionEntry> {
        self.projections.values()
    }

    /// Iterates index entries in ID order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`.
    pub fn indexes(&self) -> impl Iterator<Item = &IndexEntry> {
        self.indexes.values()
    }

    /// Registers a structural role.
    pub(crate) fn insert_role(&mut self, id: RoleId, name: String) -> Result<(), DbError> {
        insert_named(&mut self.role_names, &name, id)?;
        self.roles.insert(id, RoleDefinition { id, name });
        Ok(())
    }

    /// Registers a label.
    pub(crate) fn insert_label(&mut self, id: LabelId, name: String) -> Result<(), DbError> {
        insert_named(&mut self.label_names, &name, id)?;
        self.labels.insert(id, LabelDefinition { id, name });
        Ok(())
    }

    /// Registers a relation type.
    pub(crate) fn insert_relation_type(
        &mut self,
        id: RelationTypeId,
        name: String,
    ) -> Result<(), DbError> {
        insert_named(&mut self.relation_type_names, &name, id)?;
        self.relation_types
            .insert(id, RelationTypeDefinition { id, name });
        Ok(())
    }

    /// Registers a typed property key.
    pub(crate) fn insert_property_key(
        &mut self,
        definition: PropertyKeyDefinition,
    ) -> Result<(), DbError> {
        insert_named(
            &mut self.property_key_names,
            &definition.name,
            definition.id,
        )?;
        self.property_keys.insert(definition.id, definition);
        Ok(())
    }

    /// Registers a projection definition.
    pub(crate) fn insert_projection(
        &mut self,
        id: ProjectionId,
        definition: ProjectionDefinition,
    ) -> Result<(), DbError> {
        insert_named(&mut self.projection_names, definition.name(), id)?;
        self.projections
            .insert(id, ProjectionEntry { id, definition });
        Ok(())
    }

    /// Registers an index definition.
    pub(crate) fn insert_index(
        &mut self,
        id: IndexId,
        name: String,
        definition: IndexDefinition,
    ) -> Result<(), DbError> {
        insert_named(&mut self.index_names, &name, id)?;
        self.indexes.insert(
            id,
            IndexEntry {
                id,
                name,
                definition,
            },
        );
        Ok(())
    }
}

/// Inserts one unique name into a catalog name map.
fn insert_named<Id: Copy>(
    names: &mut BTreeMap<String, Id>,
    name: &str,
    id: Id,
) -> Result<(), DbError> {
    if names.contains_key(name) {
        return Err(DbError::DuplicateCatalogName);
    }
    names.insert(name.to_owned(), id);
    Ok(())
}
