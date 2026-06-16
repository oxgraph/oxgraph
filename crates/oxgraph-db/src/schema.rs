//! Declarative catalog schema.
//!
//! Declare a store's roles, labels, relation types, typed property keys,
//! equality indexes, and graph projections by name ONCE as a [`Schema`], then
//! apply it idempotently with [`Writer::apply_schema`](crate::Writer::apply_schema)
//! (register-or-get) or resolve an already-bootstrapped store with
//! [`Db::bind`](crate::Db::bind). Both return a [`Bound`] handle bag whose typed
//! getters hand back [`Key<T>`](crate::Key)/[`EqualityIndex<T>`](crate::EqualityIndex)
//! and plain id newtypes — so a consumer never threads name→id maps by hand and a
//! name typo is a typed [`DbError::UnknownName`].

use std::collections::BTreeMap;

use crate::{
    DbError, IndexId, LabelId, ProjectionId, PropertyFamily, PropertyKeyId, PropertyType,
    RelationTypeId, RoleId, encode_composite_key,
    typed::{EqualityIndex, Key, ValueType},
};

/// A binary graph-projection declaration over a set of relation types.
///
/// # Performance
///
/// Cloning is `O(relation-type count + name lengths)`.
#[derive(Clone, Debug)]
pub struct GraphProjectionSpec {
    /// Projection name.
    pub(crate) name: String,
    /// Relation-type names whose edges the projection traverses.
    pub(crate) relation_types: Vec<String>,
    /// Source incidence role name.
    pub(crate) source_role: String,
    /// Target incidence role name.
    pub(crate) target_role: String,
}

/// A directed hypergraph-projection declaration over a set of relation types.
///
/// Unlike a [`GraphProjectionSpec`], participation is many-sided: a relation's
/// incidences whose role is in `source_roles` form the source-side participants
/// and those in `target_roles` the target-side participants, so one relation can
/// carry an arbitrary number of participants per side.
///
/// # Performance
///
/// Cloning is `O(relation-type count + role count + name lengths)`.
#[derive(Clone, Debug)]
pub struct HypergraphProjectionSpec {
    /// Projection name.
    pub(crate) name: String,
    /// Relation-type names whose relations the projection treats as hyperedges.
    pub(crate) relation_types: Vec<String>,
    /// Incidence role names whose participants are source-side.
    pub(crate) source_roles: Vec<String>,
    /// Incidence role names whose participants are target-side.
    pub(crate) target_roles: Vec<String>,
}

/// A declarative catalog schema, applied once to obtain a [`Bound`] handle bag.
///
/// Built fluently; declaration order is preserved (and irrelevant to the result).
///
/// # Performance
///
/// Each builder method is `O(1)` amortized; cloning is `O(declared item count)`.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    /// Declared role names.
    pub(crate) roles: Vec<String>,
    /// Declared label names.
    pub(crate) labels: Vec<String>,
    /// Declared relation-type names.
    pub(crate) relation_types: Vec<String>,
    /// Declared property keys: `(name, family, value type)`.
    pub(crate) keys: Vec<(String, PropertyFamily, PropertyType)>,
    /// Declared equality indexes: `(index name, indexed key name)`.
    pub(crate) equality_indexes: Vec<(String, String)>,
    /// Declared graph projections.
    pub(crate) graph_projections: Vec<GraphProjectionSpec>,
    /// Declared hypergraph projections.
    pub(crate) hypergraph_projections: Vec<HypergraphProjectionSpec>,
}

impl Schema {
    /// Starts an empty schema.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares an incidence role.
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    #[must_use]
    pub fn role(mut self, name: &str) -> Self {
        self.roles.push(name.to_owned());
        self
    }

    /// Declares an element/relation label.
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    #[must_use]
    pub fn label(mut self, name: &str) -> Self {
        self.labels.push(name.to_owned());
        self
    }

    /// Declares a relation type.
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    #[must_use]
    pub fn relation_type(mut self, name: &str) -> Self {
        self.relation_types.push(name.to_owned());
        self
    }

    /// Declares a typed property key in `family` whose value type is `T`.
    ///
    /// # Performance
    ///
    /// This method is `O(name length)`.
    #[must_use]
    pub fn key<T: ValueType>(mut self, name: &str, family: PropertyFamily) -> Self {
        self.keys.push((name.to_owned(), family, T::TYPE));
        self
    }

    /// Declares an equality index named `name` over the property key `key`.
    ///
    /// # Performance
    ///
    /// This method is `O(name lengths)`.
    #[must_use]
    pub fn equality_index(mut self, name: &str, key: &str) -> Self {
        self.equality_indexes
            .push((name.to_owned(), key.to_owned()));
        self
    }

    /// Declares a binary graph projection over `relation_types`, traversing from
    /// `source_role` to `target_role`.
    ///
    /// # Performance
    ///
    /// This method is `O(relation-type count + name lengths)`.
    #[must_use]
    pub fn graph_projection(
        mut self,
        name: &str,
        relation_types: &[&str],
        source_role: &str,
        target_role: &str,
    ) -> Self {
        self.graph_projections.push(GraphProjectionSpec {
            name: name.to_owned(),
            relation_types: relation_types
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            source_role: source_role.to_owned(),
            target_role: target_role.to_owned(),
        });
        self
    }

    /// Declares a directed hypergraph projection over `relation_types`, treating
    /// incidences in `source_roles` as source-side participants and those in
    /// `target_roles` as target-side participants.
    ///
    /// The role-name slices must be non-empty: a hypergraph projection
    /// materializes only relations carrying at least one source and one target
    /// participant (enforced when
    /// [`Reader::hypergraph_projection`](crate::Reader::hypergraph_projection) builds the
    /// view).
    ///
    /// # Performance
    ///
    /// This method is `O(relation-type count + role count + name lengths)`.
    #[must_use]
    pub fn hypergraph_projection(
        mut self,
        name: &str,
        relation_types: &[&str],
        source_roles: &[&str],
        target_roles: &[&str],
    ) -> Self {
        self.hypergraph_projections.push(HypergraphProjectionSpec {
            name: name.to_owned(),
            relation_types: relation_types
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            source_roles: source_roles.iter().map(|name| (*name).to_owned()).collect(),
            target_roles: target_roles.iter().map(|name| (*name).to_owned()).collect(),
        });
        self
    }

    /// Returns a content fingerprint of the declared schema shape: every role,
    /// label, relation type, typed key, equality index, and projection. Equal
    /// shapes hash equally regardless of declaration order; any change to a
    /// declared item changes the fingerprint.
    ///
    /// A consumer can fold this into its own index-staleness digest so a schema
    /// change forces a rebuild even when no source content changed. This is a
    /// non-cryptographic content hash, not a cross-compiler-version stability
    /// guarantee.
    ///
    /// # Performance
    ///
    /// This method is `O(declared items × name lengths + items × log items)`.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut items = Vec::new();
        for role in &self.roles {
            items.push(encode_composite_key(&["role", role]));
        }
        for label in &self.labels {
            items.push(encode_composite_key(&["label", label]));
        }
        for relation_type in &self.relation_types {
            items.push(encode_composite_key(&["rtype", relation_type]));
        }
        for (name, family, value_type) in &self.keys {
            items.push(encode_composite_key(&[
                "key",
                name,
                &format!("{family:?}"),
                &format!("{value_type:?}"),
            ]));
        }
        for (name, key) in &self.equality_indexes {
            items.push(encode_composite_key(&["eqidx", name, key]));
        }
        for projection in &self.graph_projections {
            items.push(encode_composite_key(&[
                "gproj",
                &projection.name,
                &projection.source_role,
                &projection.target_role,
                &join_sorted(&projection.relation_types),
            ]));
        }
        for projection in &self.hypergraph_projections {
            items.push(encode_composite_key(&[
                "hproj",
                &projection.name,
                &join_sorted(&projection.relation_types),
                &join_sorted(&projection.source_roles),
                &join_sorted(&projection.target_roles),
            ]));
        }
        items.sort();

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        items.len().hash(&mut hasher);
        for item in &items {
            item.hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Joins a set of names into one order-independent, injective token used inside
/// [`Schema::fingerprint`].
fn join_sorted(names: &[String]) -> String {
    let mut sorted: Vec<&str> = names.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    encode_composite_key(&sorted)
}

/// Resolved name→id handles for an applied [`Schema`].
///
/// The single place names resolve to ids; replaces hand-threaded `*Id` maps.
/// Typed getters return [`Key<T>`]/[`EqualityIndex<T>`] (the value type is
/// checked against the declaration); a missing name is a [`DbError::UnknownName`].
///
/// # Performance
///
/// Cloning is `O(handle count)`; every getter is `O(log n + name length)`.
#[derive(Clone, Debug, Default)]
pub struct Bound {
    /// Role ids by name.
    pub(crate) roles: BTreeMap<String, RoleId>,
    /// Label ids by name.
    pub(crate) labels: BTreeMap<String, LabelId>,
    /// Relation-type ids by name.
    pub(crate) relation_types: BTreeMap<String, RelationTypeId>,
    /// Property key ids (with declared value type) by name.
    pub(crate) keys: BTreeMap<String, (PropertyKeyId, PropertyType)>,
    /// Equality index ids (with indexed key value type) by name.
    pub(crate) equality_indexes: BTreeMap<String, (IndexId, PropertyType)>,
    /// Projection ids by name.
    pub(crate) projections: BTreeMap<String, ProjectionId>,
}

impl Bound {
    /// Resolves a role handle.
    ///
    /// # Errors
    ///
    /// [`DbError::UnknownName`] when the role was not declared/bound.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + name length)`.
    pub fn role(&self, name: &str) -> Result<RoleId, DbError> {
        self.roles.get(name).copied().ok_or_else(|| {
            DbError::Catalog(crate::error::CatalogError::UnknownName {
                kind: "role",
                name: name.to_owned(),
            })
        })
    }

    /// Resolves a label handle.
    ///
    /// # Errors
    ///
    /// [`DbError::UnknownName`] when the label was not declared/bound.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + name length)`.
    pub fn label(&self, name: &str) -> Result<LabelId, DbError> {
        self.labels.get(name).copied().ok_or_else(|| {
            DbError::Catalog(crate::error::CatalogError::UnknownName {
                kind: "label",
                name: name.to_owned(),
            })
        })
    }

    /// Resolves a relation-type handle.
    ///
    /// # Errors
    ///
    /// [`DbError::UnknownName`] when the relation type was not declared/bound.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + name length)`.
    pub fn relation_type(&self, name: &str) -> Result<RelationTypeId, DbError> {
        self.relation_types.get(name).copied().ok_or_else(|| {
            DbError::Catalog(crate::error::CatalogError::UnknownName {
                kind: "relation type",
                name: name.to_owned(),
            })
        })
    }

    /// Resolves a typed property-key handle, checking the value type matches `T`.
    ///
    /// # Errors
    ///
    /// [`DbError::UnknownName`] when absent, or [`DbError::SchemaConflict`] when
    /// the declared value type differs from `T`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + name length)`.
    pub fn key<T: ValueType>(&self, name: &str) -> Result<Key<T>, DbError> {
        let (id, value_type) = self.keys.get(name).copied().ok_or_else(|| {
            DbError::Catalog(crate::error::CatalogError::UnknownName {
                kind: "property key",
                name: name.to_owned(),
            })
        })?;
        if value_type == T::TYPE {
            Ok(Key::from_id(id))
        } else {
            Err(DbError::Catalog(
                crate::error::CatalogError::SchemaConflict {
                    name: name.to_owned(),
                    reason: "property key value type differs from the requested typed handle",
                },
            ))
        }
    }

    /// Resolves a typed equality-index handle, checking the indexed key's value
    /// type matches `T`.
    ///
    /// # Errors
    ///
    /// [`DbError::UnknownName`] when absent, or [`DbError::SchemaConflict`] on a
    /// value-type mismatch.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + name length)`.
    pub fn equality_index<T: ValueType>(&self, name: &str) -> Result<EqualityIndex<T>, DbError> {
        let (id, value_type) = self.equality_indexes.get(name).copied().ok_or_else(|| {
            DbError::Catalog(crate::error::CatalogError::UnknownName {
                kind: "index",
                name: name.to_owned(),
            })
        })?;
        if value_type == T::TYPE {
            Ok(EqualityIndex::from_id(id))
        } else {
            Err(DbError::Catalog(
                crate::error::CatalogError::SchemaConflict {
                    name: name.to_owned(),
                    reason: "equality index value type differs from the requested typed handle",
                },
            ))
        }
    }

    /// Resolves a projection handle.
    ///
    /// # Errors
    ///
    /// [`DbError::UnknownName`] when the projection was not declared/bound.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + name length)`.
    pub fn projection(&self, name: &str) -> Result<ProjectionId, DbError> {
        self.projections.get(name).copied().ok_or_else(|| {
            DbError::Catalog(crate::error::CatalogError::UnknownName {
                kind: "projection",
                name: name.to_owned(),
            })
        })
    }
}
