//! Embedded `OxGraph` database engine API.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{
    Catalog, CommitSeq, DbError, ElementId, ElementRecord, GraphProjection, HypergraphProjection,
    IncidenceId, IncidenceRecord, IndexId, LabelId, PreparedQuery, ProjectionDefinition,
    ProjectionId, PropertyKeyId, PropertySubject, PropertyType, PropertyValue, QueryLanguage,
    QueryResult, RelationId, RelationRecord, RelationTypeId, RoleId, TransactionId,
    catalog::{IndexDefinition, PropertyFamily},
    projection::{self},
    state::DatabaseState,
    storage::{self, StoredDatabase},
    traversal::{self, TraversalOptions, TraversalResult},
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

/// Open OXGDB database handle.
///
/// # Performance
///
/// Moving a handle is `O(n)` for the owned in-memory database state.
pub struct Database {
    /// Root database directory.
    path: PathBuf,
    /// Visible canonical state.
    state: DatabaseState,
    /// Last visible commit sequence.
    visible_commit_seq: CommitSeq,
    /// Last writer transaction ID burned by this handle.
    ///
    /// Rollback burns are session-local. Committed and empty-committed IDs are
    /// durable because commit publication persists the current high-water mark.
    last_transaction_id: TransactionId,
}

impl Database {
    /// Creates a new empty OXGDB database at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::AlreadyExists`] when a greenfield store already
    /// exists, or [`DbError::Io`] when creation fails.
    ///
    /// # Performance
    ///
    /// This function is `O(path length + empty store bytes)`.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref().to_path_buf();
        if storage::store_path(&path).exists() {
            return Err(DbError::AlreadyExists);
        }
        let stored = StoredDatabase::empty();
        storage::write_store(&path, &stored)?;
        Ok(Self::from_stored(path, stored))
    }

    /// Opens an existing OXGDB database.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the store is missing, malformed, or
    /// semantically invalid.
    ///
    /// # Performance
    ///
    /// This function is `O(serialized database bytes)`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref().to_path_buf();
        let stored = storage::read_store(&path)?;
        Ok(Self::from_stored(path, stored))
    }

    /// Validates an OXGDB database at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when store or semantic validation fails.
    ///
    /// # Performance
    ///
    /// This function is `O(serialized database bytes)`.
    pub fn validate_path(path: impl AsRef<Path>) -> Result<(), DbError> {
        storage::validate_store(path.as_ref())
    }

    /// Rewrites the store in the current greenfield format.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when validation, encoding, writing, or replacement
    /// fails.
    ///
    /// # Performance
    ///
    /// This method is `O(serialized database bytes)`.
    pub fn compact(&mut self) -> Result<(), DbError> {
        self.state.validate()?;
        storage::write_store(&self.path, &self.to_stored())
    }

    /// Validates this open handle's store and in-memory state.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when validation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(serialized database bytes)`.
    pub fn validate(&self) -> Result<(), DbError> {
        self.state.validate()?;
        storage::validate_store(&self.path)
    }

    /// Returns operational status for this handle.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn status(&self) -> DatabaseStatus {
        DatabaseStatus {
            visible_commit_seq: self.visible_commit_seq,
            last_transaction_id: self.last_transaction_id,
            element_count: self.state.element_count(),
            relation_count: self.state.relation_count(),
            incidence_count: self.state.incidence_count(),
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
        CatalogSummary::from_catalog(self.state.catalog())
    }

    /// Starts a read transaction pinned to the current visible generation.
    ///
    /// # Performance
    ///
    /// This method is `O(database state size)` because readers own immutable
    /// snapshots.
    #[must_use]
    pub fn begin_read(&self) -> ReadTransaction {
        ReadTransaction {
            pin: ReadPin {
                visible_commit_seq: self.visible_commit_seq,
                last_transaction_id: self.last_transaction_id,
            },
            state: self.state.clone(),
            graph_projections: RefCell::new(BTreeMap::new()),
            hypergraph_projections: RefCell::new(BTreeMap::new()),
        }
    }

    /// Starts the single writer transaction.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::TransactionIdOverflow`] when writer IDs are
    /// exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(database state size)` because writes stage an owned
    /// copy.
    pub fn begin_write(&mut self) -> Result<WriteTransaction<'_>, DbError> {
        let transaction_id = self
            .last_transaction_id
            .checked_next()
            .ok_or(DbError::TransactionIdOverflow)?;
        let state = self.state.clone();
        self.last_transaction_id = transaction_id;
        Ok(WriteTransaction {
            database: self,
            state,
            transaction_id,
            dirty: false,
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
        PreparedQuery::prepare(language, query, &self.state)
    }

    /// Builds a handle from stored state.
    fn from_stored(path: PathBuf, stored: StoredDatabase) -> Self {
        Self {
            path,
            state: stored.state,
            visible_commit_seq: stored.commit_seq,
            last_transaction_id: stored.transaction_id,
        }
    }

    /// Converts this handle into the durable payload.
    fn to_stored(&self) -> StoredDatabase {
        StoredDatabase {
            commit_seq: self.visible_commit_seq,
            transaction_id: self.last_transaction_id,
            state: self.state.clone(),
        }
    }

    /// Allocates the next commit sequence.
    fn next_commit_seq(&self) -> Result<CommitSeq, DbError> {
        self.visible_commit_seq
            .checked_next()
            .ok_or(DbError::CommitSeqOverflow)
    }
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
    /// This value is durable after commit and session-local after rollback.
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
    /// Pinned writer transaction high-water mark visible to this handle.
    pub last_transaction_id: TransactionId,
}

/// Read transaction over a pinned state snapshot.
///
/// # Performance
///
/// Moving a read transaction is `O(database state size)`.
pub struct ReadTransaction {
    /// Pinned generation coordinates.
    pin: ReadPin,
    /// Cloned visible state.
    state: DatabaseState,
    /// Materialized graph projections cached for the pinned commit, keyed by
    /// projection ID. The stored [`CommitSeq`] guards against reuse across a
    /// commit-sequence advance; within one read it is always the pin's seq.
    graph_projections: ProjectionCache<GraphProjection>,
    /// Materialized hypergraph projections cached for the pinned commit.
    hypergraph_projections: ProjectionCache<HypergraphProjection>,
}

/// Per-read materialized-projection cache keyed by projection ID.
///
/// Each entry carries the [`CommitSeq`] it was built against so a stale entry
/// is discarded when the visible commit sequence advances.
///
/// # Performance
///
/// Lookups and inserts are `O(log projection count)`.
type ProjectionCache<P> = RefCell<BTreeMap<ProjectionId, (CommitSeq, Rc<P>)>>;

impl ReadTransaction {
    /// Returns this transaction's reader pin.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn pin(&self) -> ReadPin {
        self.pin
    }

    /// Returns catalog metadata.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        self.state.catalog()
    }

    /// Returns visible element count.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn element_count(&self) -> usize {
        self.state.element_count()
    }

    /// Returns visible relation count.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn relation_count(&self) -> usize {
        self.state.relation_count()
    }

    /// Returns visible incidence count.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn incidence_count(&self) -> usize {
        self.state.incidence_count()
    }

    /// Returns whether an element exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub fn contains_element(&self, id: ElementId) -> bool {
        self.state.contains_element(id)
    }

    /// Returns whether a relation exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub fn contains_relation(&self, id: RelationId) -> bool {
        self.state.contains_relation(id)
    }

    /// Returns whether an incidence exists.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub fn contains_incidence(&self, id: IncidenceId) -> bool {
        self.state.contains_incidence(id)
    }

    /// Returns an element record.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub fn element(&self, id: ElementId) -> Option<&ElementRecord> {
        self.state.element(id)
    }

    /// Returns a relation record.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub fn relation(&self, id: RelationId) -> Option<&RelationRecord> {
        self.state.relation(id)
    }

    /// Returns an incidence record.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    #[must_use]
    pub fn incidence(&self, id: IncidenceId) -> Option<&IncidenceRecord> {
        self.state.incidence(id)
    }

    /// Iterates incidences attached to an element.
    ///
    /// # Performance
    ///
    /// This method is `O(i)` for visible incidence count.
    pub fn element_incidences(&self, id: ElementId) -> impl Iterator<Item = &IncidenceRecord> {
        self.state.element_incidences(id)
    }

    /// Returns one property value.
    ///
    /// # Performance
    ///
    /// This method is `O(log subjects + log keys)`.
    #[must_use]
    pub fn property(&self, subject: PropertySubject, key: PropertyKeyId) -> Option<&PropertyValue> {
        self.state.property(subject, key)
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
        self.state.typed_property_equal(key, value)
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
        self.state.typed_property_range(key, min, max)
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
    /// This method is `O(indexed family size)` for the greenfield embedded
    /// implementation.
    pub fn lookup_index(
        &self,
        index: IndexId,
        lookup: IndexLookup<'_>,
    ) -> Result<Vec<PropertySubject>, DbError> {
        let entry = self
            .state
            .catalog()
            .index(index)
            .ok_or(DbError::UnknownIndex { id: index })?;
        match (&entry.definition, lookup) {
            (IndexDefinition::Label { label }, IndexLookup::All) => Ok(self
                .state
                .elements_with_label(*label)
                .into_iter()
                .map(PropertySubject::Element)
                .collect()),
            (IndexDefinition::Label { .. }, _lookup) => {
                Err(DbError::unsupported("label index expects all lookup"))
            }
            (IndexDefinition::RelationType { relation_type }, IndexLookup::All) => Ok(self
                .state
                .relations_with_type(*relation_type)
                .into_iter()
                .map(PropertySubject::Relation)
                .collect()),
            (IndexDefinition::RelationType { .. }, _lookup) => Err(DbError::unsupported(
                "relation type index expects all lookup",
            )),
            (IndexDefinition::PropertyEquality { key }, IndexLookup::Equal(value)) => {
                self.state.typed_property_equal(*key, value)
            }
            (IndexDefinition::PropertyEquality { .. }, _lookup) => Err(DbError::unsupported(
                "property equality index expects equality lookup",
            )),
            (IndexDefinition::PropertyRange { key }, IndexLookup::Range { min, max }) => {
                self.state.typed_property_range(*key, min, max)
            }
            (IndexDefinition::PropertyRange { .. }, _lookup) => Err(DbError::unsupported(
                "property range index expects range lookup",
            )),
            (IndexDefinition::CompositeEquality { keys }, IndexLookup::CompositeEqual(values)) => {
                self.state.typed_property_composite_equal(keys, values)
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
        self.cached_graph_projection(id)
            .map(|graph| (*graph).clone())
    }

    /// Returns a cached graph projection, materializing it on first use.
    ///
    /// The projection is keyed by ID and tagged with the pinned commit
    /// sequence, so repeated traverse/query/index calls within one read reuse
    /// one materialization instead of rebuilding it `O(relation * incidence)`
    /// each time. A stale-seq entry is discarded and rebuilt.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a graph, or
    /// fails validation against current topology.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` on a cache hit and
    /// `O(relation count * incidence count)` on a miss.
    fn cached_graph_projection(&self, id: ProjectionId) -> Result<Rc<GraphProjection>, DbError> {
        let seq = self.pin.visible_commit_seq;
        if let Some((cached_seq, graph)) = self.graph_projections.borrow().get(&id)
            && *cached_seq == seq
        {
            return Ok(Rc::clone(graph));
        }
        let entry = self
            .state
            .catalog()
            .projection(id)
            .ok_or(DbError::UnknownProjection { id })?;
        let graph = match &entry.definition {
            ProjectionDefinition::Graph(definition) => {
                projection::GraphProjection::from_state(&self.state, definition.clone())?
            }
            ProjectionDefinition::Hypergraph(_definition) => {
                return Err(DbError::invalid_projection("projection is not a graph"));
            }
        };
        let graph = Rc::new(graph);
        self.graph_projections
            .borrow_mut()
            .insert(id, (seq, Rc::clone(&graph)));
        Ok(graph)
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
            .state
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
        let graph = self.cached_graph_projection(projection)?;
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
        self.cached_hypergraph_projection(id)
            .map(|hyper| (*hyper).clone())
    }

    /// Returns a cached hypergraph projection, materializing it on first use.
    ///
    /// Mirrors [`Self::cached_graph_projection`]: keyed by ID, tagged with the
    /// pinned commit sequence, and rebuilt only on a miss or stale seq.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a hypergraph,
    /// or fails validation against current topology.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` on a cache hit and
    /// `O(relation count * incidence count)` on a miss.
    fn cached_hypergraph_projection(
        &self,
        id: ProjectionId,
    ) -> Result<Rc<HypergraphProjection>, DbError> {
        let seq = self.pin.visible_commit_seq;
        if let Some((cached_seq, hyper)) = self.hypergraph_projections.borrow().get(&id)
            && *cached_seq == seq
        {
            return Ok(Rc::clone(hyper));
        }
        let entry = self
            .state
            .catalog()
            .projection(id)
            .ok_or(DbError::UnknownProjection { id })?;
        let hyper = match &entry.definition {
            ProjectionDefinition::Hypergraph(definition) => {
                projection::HypergraphProjection::from_state(&self.state, definition.clone())?
            }
            ProjectionDefinition::Graph(_definition) => {
                return Err(DbError::invalid_projection(
                    "projection is not a hypergraph",
                ));
            }
        };
        let hyper = Rc::new(hyper);
        self.hypergraph_projections
            .borrow_mut()
            .insert(id, (seq, Rc::clone(&hyper)));
        Ok(hyper)
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
        query.execute(&self.state)
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
    fn projection_index_subjects(
        &self,
        projection: ProjectionId,
    ) -> Result<Vec<PropertySubject>, DbError> {
        let entry = self
            .state
            .catalog()
            .projection(projection)
            .ok_or(DbError::UnknownProjection { id: projection })?;
        match &entry.definition {
            ProjectionDefinition::Graph(_definition) => {
                Ok(self.cached_graph_projection(projection)?.subjects())
            }
            ProjectionDefinition::Hypergraph(_definition) => {
                Ok(self.cached_hypergraph_projection(projection)?.subjects())
            }
        }
    }
}

/// Single writer transaction.
///
/// # Performance
///
/// Moving a writer is `O(database state size)`.
pub struct WriteTransaction<'db> {
    /// Database receiving the commit.
    database: &'db mut Database,
    /// Staged state after mutations.
    state: DatabaseState,
    /// Writer transaction ID.
    transaction_id: TransactionId,
    /// Whether this transaction changed visible state.
    dirty: bool,
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
        let id = self.state.register_role(name.into())?;
        self.dirty = true;
        Ok(id)
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
        let id = self.state.register_label(name.into())?;
        self.dirty = true;
        Ok(id)
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
        let id = self.state.register_relation_type(name.into())?;
        self.dirty = true;
        Ok(id)
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
        let id = self
            .state
            .register_property_key(name.into(), family, value_type)?;
        self.dirty = true;
        Ok(id)
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
        let id = self.state.define_projection(definition)?;
        self.dirty = true;
        Ok(id)
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
        let id = self.state.define_index(name.into(), definition)?;
        self.dirty = true;
        Ok(id)
    }

    /// Creates a canonical element.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when element IDs are exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log element count)`.
    pub fn create_element(&mut self) -> Result<ElementId, DbError> {
        let id = self.state.create_element()?;
        self.dirty = true;
        Ok(id)
    }

    /// Creates a canonical relation.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::IdOverflow`] when relation IDs are exhausted.
    ///
    /// # Performance
    ///
    /// This method is `O(log relation count)`.
    pub fn create_relation(&mut self) -> Result<RelationId, DbError> {
        let id = self.state.create_relation()?;
        self.dirty = true;
        Ok(id)
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
    /// This method is `O(log incidence count + reference lookup cost)`.
    pub fn create_incidence(
        &mut self,
        relation: RelationId,
        element: ElementId,
        role: RoleId,
    ) -> Result<IncidenceId, DbError> {
        let id = self.state.create_incidence(relation, element, role)?;
        self.dirty = true;
        Ok(id)
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
        self.state.tombstone_element(id)?;
        self.dirty = true;
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
        self.state.tombstone_relation(id)?;
        self.dirty = true;
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
    /// This method is `O(log incidence count)`.
    pub fn tombstone_incidence(&mut self, id: IncidenceId) -> Result<(), DbError> {
        self.state.tombstone_incidence(id)?;
        self.dirty = true;
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
    /// This method is `O(log element count + log label count)`.
    pub fn add_element_label(&mut self, element: ElementId, label: LabelId) -> Result<(), DbError> {
        self.state.add_element_label(element, label)?;
        self.dirty = true;
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
    /// This method is `O(log relation count + log label count)`.
    pub fn add_relation_label(
        &mut self,
        relation: RelationId,
        label: LabelId,
    ) -> Result<(), DbError> {
        self.state.add_relation_label(relation, label)?;
        self.dirty = true;
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
    /// This method is `O(log relation count + log relation type count)`.
    pub fn set_relation_type(
        &mut self,
        relation: RelationId,
        relation_type: RelationTypeId,
    ) -> Result<(), DbError> {
        self.state.set_relation_type(relation, relation_type)?;
        self.dirty = true;
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
    /// This method is `O(log subject count + log key count)`.
    pub fn set_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: PropertyValue,
    ) -> Result<(), DbError> {
        self.state.set_property(subject, key, value)?;
        self.dirty = true;
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
    /// This method is `O(log subject count + log key count)`.
    pub fn remove_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Result<(), DbError> {
        self.state.remove_property(subject, key)?;
        self.dirty = true;
        Ok(())
    }

    /// Commits this write transaction durably.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when commit sequence allocation, validation,
    /// encoding, writing, or store replacement fails.
    ///
    /// # Performance
    ///
    /// This method is `O(serialized database bytes)`.
    pub fn commit(self) -> Result<CommitSeq, DbError> {
        let commit_seq = if self.dirty {
            self.database.next_commit_seq()?
        } else {
            self.database.visible_commit_seq
        };
        let stored = StoredDatabase {
            commit_seq,
            transaction_id: self.transaction_id,
            state: self.state.clone(),
        };
        storage::write_store(&self.database.path, &stored)?;
        self.database.state = self.state;
        self.database.visible_commit_seq = commit_seq;
        self.database.last_transaction_id = self.transaction_id;
        Ok(commit_seq)
    }

    /// Drops this write transaction without committing.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` excluding staged-state drop cost.
    pub fn rollback(self) {}
}
