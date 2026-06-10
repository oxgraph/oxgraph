//! Read transactions: a pinned snapshot and the full read/query surface.

use std::{borrow::Cow, collections::BTreeSet, sync::Arc};

use oxgraph_algo::{
    PageRankConfig, PageRankError, PageRankWorkspace, Uniform, longest_path_dag,
    pagerank_graph_with_workspace,
};
use oxgraph_graph::{CanonicalElementIdentity, DenseElementIndex, LocalElementIdentity};

use super::IndexProbe;
use crate::{
    Catalog, CheckpointGeneration, CommitSeq, DbError, Element, ElementId, IncidenceId,
    IncidenceRecord, IndexId, PreparedQuery, ProjectionDefinition, ProjectionId, Properties,
    PropertyKeyId, PropertySubject, PropertyValue, QueryResult, Relation, RelationId,
    RelationTypeId,
    catalog::IndexDefinition,
    overlay::{Snapshot, StateView},
    projection::{self, GraphProjection, HypergraphProjection, ProjectionElementId},
    traversal::{self, Direction, Subgraph, Walk},
    typed::{Assignable, EqualityIndex, ValueType},
};

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
/// [`Db`], so it stays valid across a later `begin_write`/`checkpoint` on
/// the same handle (it cloned the snapshot before the write borrowed `&mut`). It
/// is [`Send`] + [`Sync`] (asserted below).
///
/// # Performance
///
/// Creating and cloning a read transaction is `O(1)`: it shares the pinned
/// snapshot through an `Arc`, not by copying.
pub struct Reader {
    /// The pinned snapshot this reader observes.
    pub(super) snapshot: Arc<Snapshot>,
}

/// Returns whether a [`Reader::neighbors`] walk should follow the edge from the
/// incidence `from` (the queried element's incidence) to the incidence `to`
/// (a candidate neighbor's incidence) under `direction`.
///
/// Endpoint roles are encoded by incidence-creation order: the source endpoint
/// has the lower incidence id. `Outgoing` follows source→target (the queried
/// element is the source, so `from < to`), `Incoming` follows target→source, and
/// `Both` follows either side.
///
/// # Performance
///
/// This function is `O(1)`.
const fn follow_direction(direction: Direction, from: IncidenceId, to: IncidenceId) -> bool {
    match direction {
        Direction::Outgoing => from.get() < to.get(),
        Direction::Incoming => from.get() > to.get(),
        Direction::Both => true,
    }
}

/// `Reader` MUST be `Send + Sync`: it pins only an `Arc<Snapshot>`,
/// which holds `Arc`-shared `Send + Sync` data (no `Rc`/`RefCell` reachable).
const fn assert_send_sync<T: Send + Sync>() {}
const _: () = assert_send_sync::<Reader>();
const _: () = assert_send_sync::<Arc<Snapshot>>();

impl Reader {
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

    /// Returns an owned element view — id, labels, and all properties read in one
    /// call.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + label count + property count)`.
    #[must_use]
    pub fn element(&self, id: ElementId) -> Option<Element> {
        let view = self.snapshot.view();
        let record = view.element_ref(id)?;
        let labels = record.labels.iter().collect();
        let properties =
            Properties::from_pairs(view.subject_properties(PropertySubject::Element(id)));
        Some(Element::new(id, labels, properties))
    }

    /// Returns an owned relation view — id, type, labels, and all properties read
    /// in one call.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + label count + property count)`.
    #[must_use]
    pub fn relation(&self, id: RelationId) -> Option<Relation> {
        let view = self.snapshot.view();
        let record = view.relation_ref(id)?;
        let labels = record.labels.iter().collect();
        let properties =
            Properties::from_pairs(view.subject_properties(PropertySubject::Relation(id)));
        Some(Relation::new(id, record.relation_type, labels, properties))
    }

    /// Returns an owned incidence record.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    #[must_use]
    pub fn incidence(&self, id: IncidenceId) -> Option<IncidenceRecord> {
        self.snapshot.view().incidence_ref(id).map(Cow::into_owned)
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

    /// Returns a binary relation's two endpoint elements, ordered by ascending
    /// incidence id.
    ///
    /// Reads the relation's incidences from the reverse-adjacency index and
    /// returns the elements carried by its first two incidences in id order. A
    /// relation with fewer than two visible incidences returns `None`. This
    /// reports endpoints structurally, without consulting any projection's
    /// source/target roles — use [`Self::neighbors`] when role direction matters.
    ///
    /// # Performance
    ///
    /// This method is `O(degree)` over the relation's incidences.
    #[must_use]
    pub fn endpoints(&self, relation: RelationId) -> Option<(ElementId, ElementId)> {
        let incidences = self.snapshot.view().relation_incidences(relation);
        match incidences.as_slice() {
            [first, second, ..] => Some((first.element, second.element)),
            _too_few => None,
        }
    }

    /// Returns the elements reachable from `element` along relations of
    /// `relation_type`, in ascending element-id order.
    ///
    /// Direction selects the role `element` must play on each relation. Endpoint
    /// roles are encoded by incidence-creation order: a binary relation's source
    /// is its lower incidence id and its target the higher (see
    /// [`Self::endpoints`]). `Outgoing` requires `element` to be the source (and
    /// yields the target), `Incoming` requires it to be the target (and yields
    /// the source), and `Both` yields the opposite endpoint either way. Resolved
    /// over the reverse-adjacency index — each incidence of `element` whose
    /// relation has the requested type contributes that relation's other
    /// endpoint — so this works for any binary relation without a materialized
    /// projection.
    ///
    /// # Performance
    ///
    /// This method is `O(degree of element + sum of touched relation degrees)`.
    #[must_use]
    pub fn neighbors(
        &self,
        element: ElementId,
        relation_type: RelationTypeId,
        direction: Direction,
    ) -> Vec<ElementId> {
        let view = self.snapshot.view();
        let mut neighbors = BTreeSet::new();
        for incidence in view.element_incidences(element) {
            let matches_type = view
                .relation_ref(incidence.relation)
                .is_some_and(|record| record.relation_type == Some(relation_type));
            if !matches_type {
                continue;
            }
            // The incidence id encodes the endpoint role: the source endpoint is
            // created first (lower incidence id), the target second. Compare
            // `element`'s incidence id against each other endpoint's to decide
            // which side `element` is on, then follow per the requested direction.
            neighbors.extend(
                view.relation_incidences(incidence.relation)
                    .into_iter()
                    .filter(|other| other.element != element)
                    .filter(|other| follow_direction(direction, incidence.id, other.id))
                    .map(|other| other.element),
            );
        }
        neighbors.into_iter().collect()
    }

    /// Returns one owned property value.
    ///
    /// Prefer [`Self::value`] (borrowed `Cow`) or [`Self::text`] (borrowed
    /// `&str`) when the value does not need to outlive this reader; owning is
    /// correct only across commit boundaries.
    ///
    /// # Performance
    ///
    /// This method is `O(log subjects + log keys)` plus one value clone
    /// (`O(1)` for every variant — text payloads are `Arc`-shared).
    #[must_use]
    pub fn property(&self, subject: PropertySubject, key: PropertyKeyId) -> Option<PropertyValue> {
        self.snapshot
            .view()
            .property_ref(subject, key)
            .map(Cow::into_owned)
    }

    /// Returns one property value borrowed from this reader's pinned snapshot:
    /// `Cow::Borrowed` for any committed value, `Cow::Owned` only when the
    /// value lives in the published overlay's delta.
    ///
    /// # Performance
    ///
    /// This method is `O(log subjects + log keys)`; the borrowed arm clones
    /// nothing.
    #[must_use]
    pub fn value(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'_, PropertyValue>> {
        self.snapshot.view().property_ref(subject, key)
    }

    /// Returns one text property's `Arc`-shared payload, or `None` when the
    /// property is absent or not text.
    ///
    /// # Performance
    ///
    /// This method is `O(log subjects + log keys)`; the text bytes are never
    /// copied (the shared payload is reference-counted for both base- and
    /// overlay-resident values).
    #[must_use]
    pub fn text(&self, subject: PropertySubject, key: PropertyKeyId) -> Option<Arc<str>> {
        match self.snapshot.view().property_ref(subject, key)?.as_ref() {
            PropertyValue::Text(text) => Some(Arc::clone(text)),
            _other => None,
        }
    }

    /// Returns the owned element whose value in `index` equals `value`, or `None`
    /// when no element matches.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the index is unknown or is not an equality index.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + label count + property count)`.
    pub fn element_by_key<T: ValueType>(
        &self,
        index: EqualityIndex<T>,
        value: impl Assignable<T>,
    ) -> Result<Option<Element>, DbError> {
        let value = value.into_value()?;
        let matched = self
            .lookup(index.id(), IndexProbe::Equal(&value))?
            .into_iter()
            .find_map(|subject| match subject {
                PropertySubject::Element(id) => Some(id),
                PropertySubject::Relation(_) | PropertySubject::Incidence(_) => None,
            });
        Ok(matched.and_then(|id| self.element(id)))
    }

    /// Returns the number of subjects carried by a membership index (a label or
    /// relation-type index).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the index is unknown or does not support
    /// membership enumeration.
    ///
    /// # Performance
    ///
    /// This method is `O(indexed family size)`.
    pub fn count(&self, index: IndexId) -> Result<usize, DbError> {
        self.lookup(index, IndexProbe::All)
            .map(|subjects| subjects.len())
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
    pub fn lookup(
        &self,
        index: IndexId,
        lookup: IndexProbe<'_>,
    ) -> Result<Vec<PropertySubject>, DbError> {
        let view = self.snapshot.view();
        let entry = view
            .catalog()
            .index(index)
            .ok_or_else(|| DbError::unknown(index))?;
        match (&entry.definition, lookup) {
            (IndexDefinition::Label { label }, IndexProbe::All) => Ok(view
                .elements_with_label(*label)
                .into_iter()
                .map(PropertySubject::Element)
                .collect()),
            (IndexDefinition::Label { .. }, _lookup) => {
                Err(DbError::unsupported("label index expects all lookup"))
            }
            (IndexDefinition::RelationType { relation_type }, IndexProbe::All) => Ok(view
                .relations_with_type(*relation_type)
                .into_iter()
                .map(PropertySubject::Relation)
                .collect()),
            (IndexDefinition::RelationType { .. }, _lookup) => Err(DbError::unsupported(
                "relation type index expects all lookup",
            )),
            (IndexDefinition::PropertyEquality { key }, IndexProbe::Equal(value)) => {
                view.typed_property_equal(*key, value)
            }
            (IndexDefinition::PropertyEquality { .. }, _lookup) => Err(DbError::unsupported(
                "property equality index expects equality lookup",
            )),
            (IndexDefinition::PropertyRange { key }, IndexProbe::Range { min, max }) => {
                view.typed_property_range(*key, min, max)
            }
            (IndexDefinition::PropertyRange { .. }, _lookup) => Err(DbError::unsupported(
                "property range index expects range lookup",
            )),
            (IndexDefinition::CompositeEquality { keys }, IndexProbe::Composite(values)) => {
                view.typed_property_composite_equal(keys, values)
            }
            (IndexDefinition::CompositeEquality { .. }, _lookup) => Err(DbError::unsupported(
                "composite equality index expects composite equality lookup",
            )),
            (IndexDefinition::Projection { projection }, IndexProbe::All) => {
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
            .ok_or_else(|| DbError::unknown(id))?;
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

    /// Walks a cataloged graph projection from canonical seed elements,
    /// returning the discovered nodes AND the projection edges among them.
    ///
    /// Nodes are unique canonical elements in BFS first-discovery order; depth is
    /// the shortest discovered hop count from any seed. Edges connect two
    /// discovered nodes, ordered deterministically and unique by relation, so the
    /// [`Subgraph`] never references a node it omitted.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a graph,
    /// cannot be materialized, or a seed element is not part of the projection.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count * incidence count + visited edges)`.
    pub fn walk(
        &self,
        projection: ProjectionId,
        seeds: &[ElementId],
        walk: Walk,
    ) -> Result<Subgraph, DbError> {
        if seeds.is_empty() || walk.limit == 0 {
            return Ok(Subgraph::default());
        }
        let graph = self.graph_projection(projection)?;
        traversal::walk_graph_projection(&graph, seeds, walk)
    }

    /// Ranks a cataloged graph projection by personalized `PageRank`, returning
    /// every projection element paired with its rank, ordered highest first.
    ///
    /// `seeds` are the restart (teleport) set: rank mass returns to them on each
    /// damping step, biasing the stationary distribution toward elements
    /// reachable from the seeds (random walk with restart). The seed weights are
    /// normalized internally, so passing the seed elements is sufficient. With no
    /// seeds this is the uniform-teleport `PageRank` over the projection.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a graph, cannot
    /// be materialized, or `PageRank` rejects the configuration (the
    /// [`PageRankConfig`] was invalid or the power iteration did not converge).
    /// Seeds absent from the projection are ignored rather than erroring; with no
    /// resolvable seed this is the uniform-teleport rank.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count * incidence count + iterations *
    /// (visible elements + visible edges) + visible elements * log(visible
    /// elements))` — the trailing term is the final rank sort.
    pub fn personalized_pagerank(
        &self,
        projection: ProjectionId,
        seeds: &[ElementId],
        config: PageRankConfig<f64>,
    ) -> Result<Vec<(ElementId, f64)>, DbError> {
        let graph = self.graph_projection(projection)?;
        let bound = graph.element_bound();
        let element_count = u32::try_from(bound).map_err(|_| {
            DbError::traversal("projection exceeds the personalized pagerank index bound")
        })?;

        let mut personalization = vec![0.0_f64; bound];
        let mut seeded = false;
        for &seed in seeds {
            if let Some(local) = graph.local_element_id(seed) {
                personalization[graph.element_index(local)] = 1.0;
                seeded = true;
            }
        }

        let mut ranks = vec![0.0_f64; bound];
        let mut workspace = PageRankWorkspace::for_graph(&graph);
        pagerank_graph_with_workspace(
            &graph,
            &Uniform,
            (0..element_count).map(ProjectionElementId::new),
            config,
            seeded.then_some(personalization.as_slice()),
            &mut ranks,
            &mut workspace,
        )
        .map_err(|error| {
            DbError::traversal(match error {
                PageRankError::InvalidDamping { .. }
                | PageRankError::InvalidTolerance { .. }
                | PageRankError::InvalidMaxIterations => "invalid pagerank configuration",
                PageRankError::NonConverged { .. } => "personalized pagerank did not converge",
                _ => "personalized pagerank failed",
            })
        })?;

        let mut ranked: Vec<(ElementId, f64)> = (0..element_count)
            .map(|index| {
                let local = ProjectionElementId::new(index);
                (
                    graph.canonical_element_id(local),
                    ranks[graph.element_index(local)],
                )
            })
            .collect();
        ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
        Ok(ranked)
    }

    /// Returns the longest chain of canonical elements along the projection's
    /// outgoing edges within the subgraph induced by `elements`.
    ///
    /// Only edges whose endpoints are both in `elements` participate. The path
    /// lists each element once from start to end; its length in edges is
    /// `path.len() - 1`. An empty `elements` slice yields an empty path.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the projection is unknown, is not a graph, cannot
    /// be materialized, or the induced subgraph contains a cycle. Elements absent
    /// from the projection are ignored, so the chain is computed over the present
    /// subset.
    ///
    /// # Performance
    ///
    /// This method is `O(relation count * incidence count + visible elements +
    /// visible edges)`.
    pub fn longest_path(
        &self,
        projection: ProjectionId,
        elements: &[ElementId],
    ) -> Result<Vec<ElementId>, DbError> {
        if elements.is_empty() {
            return Ok(Vec::new());
        }
        let graph = self.graph_projection(projection)?;
        let locals = elements
            .iter()
            .filter_map(|&element| graph.local_element_id(element))
            .collect::<Vec<ProjectionElementId>>();
        let path = longest_path_dag(&graph, &locals)
            .map_err(|_| DbError::traversal("longest path found a cycle"))?;
        Ok(path
            .into_iter()
            .map(|local| graph.canonical_element_id(local))
            .collect())
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
            .ok_or_else(|| DbError::unknown(id))?;
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
    pub fn run(&self, query: &PreparedQuery) -> Result<QueryResult, DbError> {
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
            .ok_or_else(|| DbError::unknown(projection))?;
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
