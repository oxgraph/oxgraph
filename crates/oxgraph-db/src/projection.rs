//! Materialized graph and hypergraph projection views.

use std::collections::{BTreeMap, BTreeSet};

use oxgraph_csr::build::GraphBuilder;
use oxgraph_graph::{
    DenseElementIndex, DenseRelationIndex, EdgeSourceGraph, EdgeTargetGraph, ElementPredecessors,
    ElementSuccessors, IncomingEdgeCount, IncomingGraph, LocalElementIdentity,
    LocalRelationIdentity, OutgoingEdgeCount, OutgoingGraph, TopologyBase, TopologyCounts,
};
use oxgraph_hyper::{
    DenseIncidenceIndex, DirectedHyperedgeIncidences, DirectedHyperedgeParticipants,
    DirectedVertexHyperedges, ElementIncidenceCount, IncidenceBase, IncidenceCounts,
    IncidenceElement, IncidenceRelation, IncidenceRole, IncidentHyperedges, RelationIncidenceCount,
    RelationIncidences,
};
use oxgraph_hyper_bcsr::build::{HyperVertexId, HypergraphBuilder};
use serde::{Deserialize, Serialize};

use crate::{
    DbError, ElementId, IncidenceId, RelationId, RelationTypeId, RoleId,
    catalog::{GraphProjectionDefinition, HypergraphProjectionDefinition},
    overlay::StateView,
    state::{IncidenceRecord, PropertySubject, RelationRecord},
};

/// Dense projection-local element identifier.
///
/// # Performance
///
/// Copying, comparing, ordering, and hashing are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProjectionElementId(u32);

impl ProjectionElementId {
    /// Creates a local element ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw local index.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Dense projection-local relation identifier.
///
/// # Performance
///
/// Copying, comparing, ordering, and hashing are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProjectionRelationId(u32);

impl ProjectionRelationId {
    /// Creates a local relation ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw local index.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Dense projection-local incidence identifier.
///
/// # Performance
///
/// Copying, comparing, ordering, and hashing are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProjectionIncidenceId(u32);

impl ProjectionIncidenceId {
    /// Creates a local incidence ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw local index.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Materialized binary graph projection.
///
/// This view exposes `OxGraph` canonical topology through `oxgraph-graph`
/// traits. Outgoing arrays are CSR-shaped; incoming arrays are CSC-shaped.
///
/// # Performance
///
/// Cloning is `O(n + m)` for visible nodes and edges. Traversal is `O(k)` for
/// each yielded adjacency list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphProjection {
    /// Projection definition used to build this view.
    definition: GraphProjectionDefinition,
    /// Local-to-canonical elements.
    elements: Vec<ElementId>,
    /// Canonical-to-local elements.
    element_index: BTreeMap<ElementId, ProjectionElementId>,
    /// Local-to-canonical relations.
    relations: Vec<RelationId>,
    /// Canonical-to-local relations.
    relation_index: BTreeMap<RelationId, ProjectionRelationId>,
    /// Source and target local IDs by local relation.
    endpoints: Vec<(ProjectionElementId, ProjectionElementId)>,
    /// CSR outgoing edge IDs by source local element.
    out_edges: Vec<Vec<ProjectionRelationId>>,
    /// CSC incoming edge IDs by target local element.
    in_edges: Vec<Vec<ProjectionRelationId>>,
    /// CSR outgoing neighbor IDs by source local element.
    successors: Vec<Vec<ProjectionElementId>>,
    /// CSC incoming neighbor IDs by target local element.
    predecessors: Vec<Vec<ProjectionElementId>>,
}

impl GraphProjection {
    /// Builds a graph projection from canonical state.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidProjection`] when a selected relation is not a
    /// binary relation for the configured source and target roles.
    ///
    /// # Performance
    ///
    /// This function is `O(r * i + e log n)` for relation count `r`,
    /// incidence count `i`, and selected edge count `e`.
    pub(crate) fn from_state(
        state: &impl StateView,
        definition: GraphProjectionDefinition,
    ) -> Result<Self, DbError> {
        let mut builder = GraphProjectionBuilder::new(definition);
        for relation in state.relations() {
            if relation_selected(relation.as_ref(), &builder.definition.relation_types) {
                builder.push_relation(state, relation.as_ref())?;
            }
        }
        builder.finish()
    }

    /// Returns the definition used for this projection.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn definition(&self) -> &GraphProjectionDefinition {
        &self.definition
    }

    /// Returns deterministic canonical subjects materialized by this projection.
    pub(crate) fn subjects(&self) -> Vec<PropertySubject> {
        let mut subjects = Vec::with_capacity(self.elements.len() + self.relations.len());
        subjects.extend(self.elements.iter().copied().map(PropertySubject::Element));
        subjects.extend(
            self.relations
                .iter()
                .copied()
                .map(PropertySubject::Relation),
        );
        subjects.sort_unstable();
        subjects
    }
}

/// Builder used to keep graph projection construction small.
struct GraphProjectionBuilder {
    /// Projection definition being built.
    definition: GraphProjectionDefinition,
    /// Local-to-canonical elements.
    elements: Vec<ElementId>,
    /// Canonical-to-local elements.
    element_index: BTreeMap<ElementId, ProjectionElementId>,
    /// Local-to-canonical relations.
    relations: Vec<RelationId>,
    /// Canonical-to-local relations.
    relation_index: BTreeMap<RelationId, ProjectionRelationId>,
    /// Edge endpoints.
    endpoints: Vec<(ProjectionElementId, ProjectionElementId)>,
}

impl GraphProjectionBuilder {
    /// Creates an empty graph projection builder.
    const fn new(definition: GraphProjectionDefinition) -> Self {
        Self {
            definition,
            elements: Vec::new(),
            element_index: BTreeMap::new(),
            relations: Vec::new(),
            relation_index: BTreeMap::new(),
            endpoints: Vec::new(),
        }
    }

    /// Adds one selected canonical relation.
    fn push_relation(
        &mut self,
        state: &impl StateView,
        relation: &RelationRecord,
    ) -> Result<(), DbError> {
        let (source, target) = graph_endpoints(
            state,
            relation.id,
            self.definition.source_role,
            self.definition.target_role,
        )?;
        let source = intern_element(&mut self.element_index, &mut self.elements, source)?;
        let target = intern_element(&mut self.element_index, &mut self.elements, target)?;
        let edge = projection_relation_id(self.relations.len())?;
        self.relations.push(relation.id);
        self.relation_index.insert(relation.id, edge);
        self.endpoints.push((source, target));
        Ok(())
    }

    /// Finishes the graph projection.
    ///
    /// The CSR forward adjacency and its CSC transpose are constructed by the
    /// `oxgraph-csr` substrate builder rather than re-implementing the
    /// counting-sort/offset pass here. Parallel edges are preserved: every
    /// interned endpoint becomes one builder edge, so an element with `k`
    /// outgoing edges to the same target yields `k` successor entries.
    fn finish(self) -> Result<GraphProjection, DbError> {
        let node_count = self.elements.len();
        let mut builder = GraphBuilder::<u32, u32>::new();
        let mut nodes = Vec::with_capacity(node_count);
        for _element in &self.elements {
            nodes.push(
                builder
                    .add_node()
                    .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Element))?,
            );
        }
        for (source, target) in self.endpoints.iter().copied() {
            builder
                .add_edge(nodes[local_index(source)], nodes[local_index(target)])
                .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Relation))?;
        }
        let forward = builder
            .freeze()
            .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Relation))?;
        let reverse = forward
            .transpose()
            .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Relation))?;

        // Forward freeze: `targets`/`edge_ids` are grouped by source node, so
        // outgoing successors and outgoing edge IDs read straight off its CSR
        // arrays. The transpose groups the original edge IDs and original
        // sources by target node, giving the incoming edge IDs and predecessor
        // successors with original edge identity preserved.
        let successors = grouped_elements(forward.offsets(), forward.targets(), node_count)?;
        let out_edges = grouped_relations(forward.offsets(), forward.edge_ids(), node_count)?;
        let predecessors = grouped_elements(reverse.offsets(), reverse.targets(), node_count)?;
        let in_edges = grouped_relations(reverse.offsets(), reverse.edge_ids(), node_count)?;

        Ok(GraphProjection {
            definition: self.definition,
            elements: self.elements,
            element_index: self.element_index,
            relations: self.relations,
            relation_index: self.relation_index,
            endpoints: self.endpoints,
            out_edges,
            in_edges,
            successors,
            predecessors,
        })
    }
}

/// Splits a flat CSR target array into per-node projection element lists.
///
/// `offsets` has `node_count + 1` entries; `flat[offsets[n]..offsets[n + 1]]`
/// holds the grouped target node indices for node `n`.
///
/// # Performance
///
/// This function is `O(node_count + flat.len())`.
fn grouped_elements(
    offsets: &[u32],
    flat: &[u32],
    node_count: usize,
) -> Result<Vec<Vec<ProjectionElementId>>, DbError> {
    grouped(offsets, flat, node_count, |raw| {
        Ok(ProjectionElementId(raw))
    })
}

/// Splits a flat CSR edge-ID array into per-node projection relation lists.
///
/// # Performance
///
/// This function is `O(node_count + flat.len())`.
fn grouped_relations(
    offsets: &[u32],
    flat: &[u32],
    node_count: usize,
) -> Result<Vec<Vec<ProjectionRelationId>>, DbError> {
    grouped(offsets, flat, node_count, |raw| {
        Ok(ProjectionRelationId(raw))
    })
}

/// Splits a flat CSR payload array into `node_count` grouped, mapped lists.
///
/// # Performance
///
/// This function is `O(node_count + flat.len())`.
fn grouped<T>(
    offsets: &[u32],
    flat: &[u32],
    node_count: usize,
    map: impl Fn(u32) -> Result<T, DbError>,
) -> Result<Vec<Vec<T>>, DbError> {
    let mut rows = Vec::with_capacity(node_count);
    for node in 0..node_count {
        let start = usize::try_from(offsets[node])
            .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Relation))?;
        let end = usize::try_from(offsets[node + 1])
            .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Relation))?;
        let slice = flat
            .get(start..end)
            .ok_or_else(|| DbError::invalid_projection("csr offset range out of bounds"))?;
        rows.push(
            slice
                .iter()
                .copied()
                .map(&map)
                .collect::<Result<Vec<T>, DbError>>()?,
        );
    }
    Ok(rows)
}

impl TopologyBase for GraphProjection {
    type ElementId = ProjectionElementId;
    type RelationId = ProjectionRelationId;
}

impl TopologyCounts for GraphProjection {
    fn element_count(&self) -> usize {
        self.elements.len()
    }

    fn relation_count(&self) -> usize {
        self.relations.len()
    }
}

impl DenseElementIndex for GraphProjection {
    fn element_bound(&self) -> usize {
        self.elements.len()
    }

    fn element_index(&self, element: Self::ElementId) -> usize {
        local_index(element)
    }
}

impl DenseRelationIndex for GraphProjection {
    fn relation_bound(&self) -> usize {
        self.relations.len()
    }

    fn relation_index(&self, relation: Self::RelationId) -> usize {
        local_relation_index(relation)
    }
}

impl oxgraph_graph::ContainsElement for GraphProjection {
    fn contains_element(&self, element: Self::ElementId) -> bool {
        local_index(element) < self.elements.len()
    }
}

impl oxgraph_graph::ContainsRelation for GraphProjection {
    fn contains_relation(&self, relation: Self::RelationId) -> bool {
        local_relation_index(relation) < self.relations.len()
    }
}

impl oxgraph_graph::CanonicalElementIdentity for GraphProjection {
    type CanonicalElementId = ElementId;

    fn canonical_element_id(&self, element: Self::ElementId) -> Self::CanonicalElementId {
        self.elements[local_index(element)]
    }
}

impl LocalElementIdentity for GraphProjection {
    fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId> {
        self.element_index.get(&canonical).copied()
    }
}

impl oxgraph_graph::CanonicalRelationIdentity for GraphProjection {
    type CanonicalRelationId = RelationId;

    fn canonical_relation_id(&self, relation: Self::RelationId) -> Self::CanonicalRelationId {
        self.relations[local_relation_index(relation)]
    }
}

impl LocalRelationIdentity for GraphProjection {
    fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId> {
        self.relation_index.get(&canonical).copied()
    }
}

impl EdgeSourceGraph for GraphProjection {
    fn source(&self, edge: Self::RelationId) -> Self::ElementId {
        self.endpoints[local_relation_index(edge)].0
    }
}

impl EdgeTargetGraph for GraphProjection {
    fn target(&self, edge: Self::RelationId) -> Self::ElementId {
        self.endpoints[local_relation_index(edge)].1
    }
}

impl OutgoingGraph for GraphProjection {
    type OutEdges<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionRelationId>>
    where
        Self: 'view;

    fn outgoing_edges(&self, node: Self::ElementId) -> Self::OutEdges<'_> {
        self.out_edges[local_index(node)].iter().copied()
    }
}

impl IncomingGraph for GraphProjection {
    type InEdges<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionRelationId>>
    where
        Self: 'view;

    fn incoming_edges(&self, node: Self::ElementId) -> Self::InEdges<'_> {
        self.in_edges[local_index(node)].iter().copied()
    }
}

impl ElementSuccessors for GraphProjection {
    type Successors<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionElementId>>
    where
        Self: 'view;

    fn element_successors(&self, element: Self::ElementId) -> Self::Successors<'_> {
        self.successors[local_index(element)].iter().copied()
    }
}

impl ElementPredecessors for GraphProjection {
    type Predecessors<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionElementId>>
    where
        Self: 'view;

    fn element_predecessors(&self, element: Self::ElementId) -> Self::Predecessors<'_> {
        self.predecessors[local_index(element)].iter().copied()
    }
}

impl OutgoingEdgeCount for GraphProjection {
    fn out_degree(&self, node: Self::ElementId) -> usize {
        self.out_edges[local_index(node)].len()
    }
}

impl IncomingEdgeCount for GraphProjection {
    fn in_degree(&self, node: Self::ElementId) -> usize {
        self.in_edges[local_index(node)].len()
    }
}

/// Materialized directed hypergraph projection.
///
/// This view exposes `OxGraph` canonical topology through `oxgraph-hyper`
/// traits. Its physical arrays are BCSR-shaped: relation-major participant
/// arrays plus vertex-major incoming/outgoing hyperedge arrays.
///
/// # Performance
///
/// Cloning is `O(v + h + p)` for vertices, hyperedges, and participants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HypergraphProjection {
    /// Projection definition used to build this view.
    definition: HypergraphProjectionDefinition,
    /// Local-to-canonical elements.
    elements: Vec<ElementId>,
    /// Canonical-to-local elements.
    element_index: BTreeMap<ElementId, ProjectionElementId>,
    /// Local-to-canonical relations.
    relations: Vec<RelationId>,
    /// Canonical-to-local relations.
    relation_index: BTreeMap<RelationId, ProjectionRelationId>,
    /// Local-to-canonical incidences.
    incidences: Vec<IncidenceRecord>,
    /// Canonical-to-local incidences.
    incidence_index: BTreeMap<IncidenceId, ProjectionIncidenceId>,
    /// Relation-major incidence IDs.
    relation_incidences: Vec<Vec<ProjectionIncidenceId>>,
    /// Element-major incidence IDs.
    element_incidences: Vec<Vec<ProjectionIncidenceId>>,
    /// Source-side incidence IDs by relation.
    source_incidences: Vec<Vec<ProjectionIncidenceId>>,
    /// Target-side incidence IDs by relation.
    target_incidences: Vec<Vec<ProjectionIncidenceId>>,
    /// Source-side element IDs by relation.
    source_vertices: Vec<Vec<ProjectionElementId>>,
    /// Target-side element IDs by relation.
    target_vertices: Vec<Vec<ProjectionElementId>>,
    /// Outgoing hyperedges by source-side element.
    outgoing_hyperedges: Vec<Vec<ProjectionRelationId>>,
    /// Incoming hyperedges by target-side element.
    incoming_hyperedges: Vec<Vec<ProjectionRelationId>>,
    /// Directed successor elements by source-side element.
    successors: Vec<Vec<ProjectionElementId>>,
    /// Directed predecessor elements by target-side element.
    predecessors: Vec<Vec<ProjectionElementId>>,
}

impl HypergraphProjection {
    /// Builds a hypergraph projection from canonical state.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidProjection`] when source/target role sets do
    /// not produce directed participant sets for selected relations.
    ///
    /// # Performance
    ///
    /// This function is `O(r * i + p log v)` for relation count `r`,
    /// incidence count `i`, and selected participant count `p`.
    pub(crate) fn from_state(
        state: &impl StateView,
        definition: HypergraphProjectionDefinition,
    ) -> Result<Self, DbError> {
        if definition.source_roles.is_empty() || definition.target_roles.is_empty() {
            return Err(DbError::invalid_projection(
                "hypergraph projection requires non-empty source and target roles",
            ));
        }
        let mut builder = HypergraphProjectionBuilder::new(definition);
        for relation in state.relations() {
            if relation_selected(relation.as_ref(), &builder.definition.relation_types) {
                builder.push_relation(state, relation.as_ref())?;
            }
        }
        builder.finish()
    }

    /// Returns the definition used for this projection.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn definition(&self) -> &HypergraphProjectionDefinition {
        &self.definition
    }

    /// Returns deterministic canonical subjects materialized by this projection.
    pub(crate) fn subjects(&self) -> Vec<PropertySubject> {
        let mut subjects =
            Vec::with_capacity(self.elements.len() + self.relations.len() + self.incidences.len());
        subjects.extend(self.elements.iter().copied().map(PropertySubject::Element));
        subjects.extend(
            self.relations
                .iter()
                .copied()
                .map(PropertySubject::Relation),
        );
        subjects.extend(
            self.incidences
                .iter()
                .map(|record| PropertySubject::Incidence(record.id)),
        );
        subjects.sort_unstable();
        subjects
    }
}

/// Builder used to keep hypergraph projection construction small.
struct HypergraphProjectionBuilder {
    /// Projection definition being built.
    definition: HypergraphProjectionDefinition,
    /// Local-to-canonical elements.
    elements: Vec<ElementId>,
    /// Canonical-to-local elements.
    element_index: BTreeMap<ElementId, ProjectionElementId>,
    /// Local-to-canonical relations.
    relations: Vec<RelationId>,
    /// Canonical-to-local relations.
    relation_index: BTreeMap<RelationId, ProjectionRelationId>,
    /// Local incidence records.
    incidences: Vec<IncidenceRecord>,
    /// Canonical-to-local incidences.
    incidence_index: BTreeMap<IncidenceId, ProjectionIncidenceId>,
    /// Relation incidence arrays.
    relation_incidences: Vec<Vec<ProjectionIncidenceId>>,
    /// Source incidence arrays.
    source_incidences: Vec<Vec<ProjectionIncidenceId>>,
    /// Target incidence arrays.
    target_incidences: Vec<Vec<ProjectionIncidenceId>>,
    /// Source vertex arrays.
    source_vertices: Vec<Vec<ProjectionElementId>>,
    /// Target vertex arrays.
    target_vertices: Vec<Vec<ProjectionElementId>>,
}

impl HypergraphProjectionBuilder {
    /// Creates an empty hypergraph projection builder.
    const fn new(definition: HypergraphProjectionDefinition) -> Self {
        Self {
            definition,
            elements: Vec::new(),
            element_index: BTreeMap::new(),
            relations: Vec::new(),
            relation_index: BTreeMap::new(),
            incidences: Vec::new(),
            incidence_index: BTreeMap::new(),
            relation_incidences: Vec::new(),
            source_incidences: Vec::new(),
            target_incidences: Vec::new(),
            source_vertices: Vec::new(),
            target_vertices: Vec::new(),
        }
    }

    /// Adds one selected canonical hyperedge relation.
    fn push_relation(
        &mut self,
        state: &impl StateView,
        relation: &RelationRecord,
    ) -> Result<(), DbError> {
        let hyperedge = projection_relation_id(self.relations.len())?;
        self.relations.push(relation.id);
        self.relation_index.insert(relation.id, hyperedge);
        let mut relation_ids = Vec::new();
        let mut source_ids = Vec::new();
        let mut target_ids = Vec::new();
        let mut sources = Vec::new();
        let mut targets = Vec::new();
        for incidence in state.relation_incidences(relation.id) {
            let local = self.push_incidence(&incidence)?;
            let vertex = intern_element(
                &mut self.element_index,
                &mut self.elements,
                incidence.element,
            )?;
            relation_ids.push(local);
            if self.definition.source_roles.contains(&incidence.role) {
                source_ids.push(local);
                sources.push(vertex);
            }
            if self.definition.target_roles.contains(&incidence.role) {
                target_ids.push(local);
                targets.push(vertex);
            }
        }
        if sources.is_empty() || targets.is_empty() {
            return Err(DbError::invalid_projection(
                "selected hyperedge lacks source or target participants",
            ));
        }
        self.relation_incidences.push(relation_ids);
        self.source_incidences.push(source_ids);
        self.target_incidences.push(target_ids);
        self.source_vertices.push(sources);
        self.target_vertices.push(targets);
        Ok(())
    }

    /// Adds one canonical incidence to local storage.
    fn push_incidence(
        &mut self,
        incidence: &IncidenceRecord,
    ) -> Result<ProjectionIncidenceId, DbError> {
        if let Some(id) = self.incidence_index.get(&incidence.id) {
            return Ok(*id);
        }
        let local = projection_incidence_id(self.incidences.len())?;
        self.incidences.push(*incidence);
        self.incidence_index.insert(incidence.id, local);
        Ok(local)
    }

    /// Finishes the hypergraph projection.
    fn finish(self) -> Result<HypergraphProjection, DbError> {
        let element_incidences =
            build_element_incidences(self.elements.len(), &self.incidences, &self.element_index)?;
        let (outgoing_hyperedges, incoming_hyperedges, successors, predecessors) =
            build_directed_vertex_arrays(
                self.elements.len(),
                &self.source_vertices,
                &self.target_vertices,
            )?;
        Ok(HypergraphProjection {
            definition: self.definition,
            elements: self.elements,
            element_index: self.element_index,
            relations: self.relations,
            relation_index: self.relation_index,
            incidences: self.incidences,
            incidence_index: self.incidence_index,
            relation_incidences: self.relation_incidences,
            element_incidences,
            source_incidences: self.source_incidences,
            target_incidences: self.target_incidences,
            source_vertices: self.source_vertices,
            target_vertices: self.target_vertices,
            outgoing_hyperedges,
            incoming_hyperedges,
            successors,
            predecessors,
        })
    }
}

impl TopologyBase for HypergraphProjection {
    type ElementId = ProjectionElementId;
    type RelationId = ProjectionRelationId;
}

impl IncidenceBase for HypergraphProjection {
    type IncidenceId = ProjectionIncidenceId;
    type Role = RoleId;
}

impl TopologyCounts for HypergraphProjection {
    fn element_count(&self) -> usize {
        self.elements.len()
    }

    fn relation_count(&self) -> usize {
        self.relations.len()
    }
}

impl IncidenceCounts for HypergraphProjection {
    fn incidence_count(&self) -> usize {
        self.incidences.len()
    }
}

impl DenseElementIndex for HypergraphProjection {
    fn element_bound(&self) -> usize {
        self.elements.len()
    }

    fn element_index(&self, element: Self::ElementId) -> usize {
        local_index(element)
    }
}

impl DenseRelationIndex for HypergraphProjection {
    fn relation_bound(&self) -> usize {
        self.relations.len()
    }

    fn relation_index(&self, relation: Self::RelationId) -> usize {
        local_relation_index(relation)
    }
}

impl DenseIncidenceIndex for HypergraphProjection {
    fn incidence_bound(&self) -> usize {
        self.incidences.len()
    }

    fn incidence_index(&self, incidence: Self::IncidenceId) -> usize {
        local_incidence_index(incidence)
    }
}

impl oxgraph_hyper::ContainsElement for HypergraphProjection {
    fn contains_element(&self, element: Self::ElementId) -> bool {
        local_index(element) < self.elements.len()
    }
}

impl oxgraph_hyper::ContainsRelation for HypergraphProjection {
    fn contains_relation(&self, relation: Self::RelationId) -> bool {
        local_relation_index(relation) < self.relations.len()
    }
}

impl oxgraph_hyper::ContainsIncidence for HypergraphProjection {
    fn contains_incidence(&self, incidence: Self::IncidenceId) -> bool {
        local_incidence_index(incidence) < self.incidences.len()
    }
}

impl oxgraph_hyper::CanonicalElementIdentity for HypergraphProjection {
    type CanonicalElementId = ElementId;

    fn canonical_element_id(&self, element: Self::ElementId) -> Self::CanonicalElementId {
        self.elements[local_index(element)]
    }
}

impl oxgraph_hyper::LocalElementIdentity for HypergraphProjection {
    fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId> {
        self.element_index.get(&canonical).copied()
    }
}

impl oxgraph_hyper::CanonicalRelationIdentity for HypergraphProjection {
    type CanonicalRelationId = RelationId;

    fn canonical_relation_id(&self, relation: Self::RelationId) -> Self::CanonicalRelationId {
        self.relations[local_relation_index(relation)]
    }
}

impl oxgraph_hyper::LocalRelationIdentity for HypergraphProjection {
    fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId> {
        self.relation_index.get(&canonical).copied()
    }
}

impl oxgraph_hyper::CanonicalIncidenceIdentity for HypergraphProjection {
    type CanonicalIncidenceId = IncidenceId;

    fn canonical_incidence_id(&self, incidence: Self::IncidenceId) -> Self::CanonicalIncidenceId {
        self.incidences[local_incidence_index(incidence)].id
    }
}

impl oxgraph_hyper::LocalIncidenceIdentity for HypergraphProjection {
    fn local_incidence_id(
        &self,
        canonical: Self::CanonicalIncidenceId,
    ) -> Option<Self::IncidenceId> {
        self.incidence_index.get(&canonical).copied()
    }
}

impl RelationIncidences for HypergraphProjection {
    type Incidences<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionIncidenceId>>
    where
        Self: 'view;

    fn relation_incidences(&self, relation: Self::RelationId) -> Self::Incidences<'_> {
        self.relation_incidences[local_relation_index(relation)]
            .iter()
            .copied()
    }
}

impl oxgraph_hyper::ElementIncidences for HypergraphProjection {
    type Incidences<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionIncidenceId>>
    where
        Self: 'view;

    fn element_incidences(&self, element: Self::ElementId) -> Self::Incidences<'_> {
        self.element_incidences[local_index(element)]
            .iter()
            .copied()
    }
}

impl IncidenceElement for HypergraphProjection {
    fn incidence_element(&self, incidence: Self::IncidenceId) -> Self::ElementId {
        let canonical = self.incidences[local_incidence_index(incidence)].element;
        self.element_index[&canonical]
    }
}

impl IncidenceRelation for HypergraphProjection {
    fn incidence_relation(&self, incidence: Self::IncidenceId) -> Self::RelationId {
        let canonical = self.incidences[local_incidence_index(incidence)].relation;
        self.relation_index[&canonical]
    }
}

impl IncidenceRole for HypergraphProjection {
    fn incidence_role(&self, incidence: Self::IncidenceId) -> Self::Role {
        self.incidences[local_incidence_index(incidence)].role
    }
}

impl RelationIncidenceCount for HypergraphProjection {
    fn relation_incidence_count(&self, relation: Self::RelationId) -> usize {
        self.relation_incidences[local_relation_index(relation)].len()
    }
}

impl ElementIncidenceCount for HypergraphProjection {
    fn element_incidence_count(&self, element: Self::ElementId) -> usize {
        self.element_incidences[local_index(element)].len()
    }
}

impl oxgraph_hyper::HyperedgeParticipants for HypergraphProjection {
    type Participants<'view>
        = HyperedgeParticipants<'view>
    where
        Self: 'view;

    fn hyperedge_participants(&self, hyperedge: Self::RelationId) -> Self::Participants<'_> {
        HyperedgeParticipants {
            view: self,
            inner: self.relation_incidences[local_relation_index(hyperedge)].iter(),
        }
    }
}

impl IncidentHyperedges for HypergraphProjection {
    type IncidentHyperedges<'view>
        = IncidentHyperedgesIter<'view>
    where
        Self: 'view;

    fn incident_hyperedges(&self, vertex: Self::ElementId) -> Self::IncidentHyperedges<'_> {
        IncidentHyperedgesIter {
            view: self,
            inner: self.element_incidences[local_index(vertex)].iter(),
        }
    }
}

// `HyperedgeParticipantCount` / `IncidentHyperedgeCount` are provided by the
// blanket impls in `oxgraph-hyper` over the `RelationIncidenceCount` /
// `ElementIncidenceCount` impls above.

impl DirectedHyperedgeParticipants for HypergraphProjection {
    type SourceParticipants<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionElementId>>
    where
        Self: 'view;

    type TargetParticipants<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionElementId>>
    where
        Self: 'view;

    fn source_participants(&self, hyperedge: Self::RelationId) -> Self::SourceParticipants<'_> {
        self.source_vertices[local_relation_index(hyperedge)]
            .iter()
            .copied()
    }

    fn target_participants(&self, hyperedge: Self::RelationId) -> Self::TargetParticipants<'_> {
        self.target_vertices[local_relation_index(hyperedge)]
            .iter()
            .copied()
    }
}

impl DirectedHyperedgeIncidences for HypergraphProjection {
    type SourceIncidences<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionIncidenceId>>
    where
        Self: 'view;

    type TargetIncidences<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionIncidenceId>>
    where
        Self: 'view;

    fn source_incidences(&self, hyperedge: Self::RelationId) -> Self::SourceIncidences<'_> {
        self.source_incidences[local_relation_index(hyperedge)]
            .iter()
            .copied()
    }

    fn target_incidences(&self, hyperedge: Self::RelationId) -> Self::TargetIncidences<'_> {
        self.target_incidences[local_relation_index(hyperedge)]
            .iter()
            .copied()
    }
}

impl DirectedVertexHyperedges for HypergraphProjection {
    type OutgoingHyperedges<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionRelationId>>
    where
        Self: 'view;

    type IncomingHyperedges<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionRelationId>>
    where
        Self: 'view;

    fn outgoing_hyperedges(&self, vertex: Self::ElementId) -> Self::OutgoingHyperedges<'_> {
        self.outgoing_hyperedges[local_index(vertex)]
            .iter()
            .copied()
    }

    fn incoming_hyperedges(&self, vertex: Self::ElementId) -> Self::IncomingHyperedges<'_> {
        self.incoming_hyperedges[local_index(vertex)]
            .iter()
            .copied()
    }
}

impl ElementSuccessors for HypergraphProjection {
    type Successors<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionElementId>>
    where
        Self: 'view;

    fn element_successors(&self, element: Self::ElementId) -> Self::Successors<'_> {
        self.successors[local_index(element)].iter().copied()
    }
}

impl ElementPredecessors for HypergraphProjection {
    type Predecessors<'view>
        = std::iter::Copied<std::slice::Iter<'view, ProjectionElementId>>
    where
        Self: 'view;

    fn element_predecessors(&self, element: Self::ElementId) -> Self::Predecessors<'_> {
        self.predecessors[local_index(element)].iter().copied()
    }
}

/// Iterator mapping hyperedge incidences to participant vertices.
pub struct HyperedgeParticipants<'view> {
    /// Hypergraph projection view.
    view: &'view HypergraphProjection,
    /// Underlying incidence iterator.
    inner: std::slice::Iter<'view, ProjectionIncidenceId>,
}

impl Iterator for HyperedgeParticipants<'_> {
    type Item = ProjectionElementId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|incidence| self.view.incidence_element(*incidence))
    }
}

/// Iterator mapping vertex incidences to incident hyperedges.
pub struct IncidentHyperedgesIter<'view> {
    /// Hypergraph projection view.
    view: &'view HypergraphProjection,
    /// Underlying incidence iterator.
    inner: std::slice::Iter<'view, ProjectionIncidenceId>,
}

impl Iterator for IncidentHyperedgesIter<'_> {
    type Item = ProjectionRelationId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|incidence| self.view.incidence_relation(*incidence))
    }
}

/// Returns whether a relation belongs to a relation-type filter.
fn relation_selected(record: &RelationRecord, selected: &BTreeSet<RelationTypeId>) -> bool {
    record
        .relation_type
        .map_or(selected.is_empty(), |relation_type| {
            selected.is_empty() || selected.contains(&relation_type)
        })
}

/// Finds graph source and target endpoints for one canonical relation.
fn graph_endpoints(
    state: &impl StateView,
    relation: RelationId,
    source_role: RoleId,
    target_role: RoleId,
) -> Result<(ElementId, ElementId), DbError> {
    let mut source = None;
    let mut target = None;
    for incidence in state.relation_incidences(relation) {
        update_endpoint(&mut source, &incidence, source_role, "source")?;
        update_endpoint(&mut target, &incidence, target_role, "target")?;
    }
    match (source, target) {
        (Some(source), Some(target)) => Ok((source, target)),
        _missing => Err(DbError::invalid_projection(
            "selected graph relation lacks source or target endpoint",
        )),
    }
}

/// Updates one graph endpoint slot.
fn update_endpoint(
    slot: &mut Option<ElementId>,
    incidence: &IncidenceRecord,
    role: RoleId,
    name: &'static str,
) -> Result<(), DbError> {
    if incidence.role != role {
        return Ok(());
    }
    if slot.replace(incidence.element).is_some() {
        return Err(DbError::invalid_projection(format!(
            "selected graph relation has duplicate {name} endpoint"
        )));
    }
    Ok(())
}

/// Interns a canonical element into projection-local storage.
fn intern_element(
    index: &mut BTreeMap<ElementId, ProjectionElementId>,
    elements: &mut Vec<ElementId>,
    canonical: ElementId,
) -> Result<ProjectionElementId, DbError> {
    if let Some(local) = index.get(&canonical) {
        return Ok(*local);
    }
    let local = projection_element_id(elements.len())?;
    elements.push(canonical);
    index.insert(canonical, local);
    Ok(local)
}

/// Builds element-major incidence arrays.
fn build_element_incidences(
    element_count: usize,
    incidences: &[IncidenceRecord],
    element_index: &BTreeMap<ElementId, ProjectionElementId>,
) -> Result<Vec<Vec<ProjectionIncidenceId>>, DbError> {
    let mut rows = vec![Vec::new(); element_count];
    for (index, incidence) in incidences.iter().enumerate() {
        let local = projection_incidence_id(index)?;
        let element = element_index
            .get(&incidence.element)
            .ok_or_else(|| DbError::invalid_projection("incidence element not projected"))?;
        rows[local_index(*element)].push(local);
    }
    Ok(rows)
}

/// Builds directed vertex-major hypergraph arrays on the substrate BCSR builder.
///
/// The vertex→hyperedge adjacency (`outgoing`/`incoming`) and the directed
/// vertex successor/predecessor sets are produced by freezing an
/// [`HypergraphBuilder`] rather than re-implementing the BCSR offset pass. The
/// builder requires unique participants per side, so each hyperedge's source
/// and target vertex lists are deduplicated (first-occurrence order) before
/// being added; this matches the directed-adjacency semantic, where a vertex
/// incident to one hyperedge in a given role contributes that hyperedge once.
/// Vertex successors/predecessors are deduplicated and ordered to preserve the
/// projection's prior set-valued adjacency contract.
fn build_directed_vertex_arrays(
    element_count: usize,
    source_vertices: &[Vec<ProjectionElementId>],
    target_vertices: &[Vec<ProjectionElementId>],
) -> Result<DirectedVertexArrays, DbError> {
    let mut builder = HypergraphBuilder::<u32, u32, u32>::new();
    let mut vertices = Vec::with_capacity(element_count);
    for _element in 0..element_count {
        vertices.push(
            builder
                .add_vertex()
                .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Element))?,
        );
    }
    for (sources, targets) in source_vertices.iter().zip(target_vertices) {
        let source_vertices = unique_vertices(&vertices, sources);
        let target_vertices = unique_vertices(&vertices, targets);
        builder
            .add_hyperedge(&source_vertices, &target_vertices)
            .map_err(|_error| {
                DbError::invalid_projection("hyperedge participants are not directed-unique")
            })?;
    }
    let frozen = builder
        .freeze()
        .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Incidence))?;

    let mut outgoing = Vec::with_capacity(element_count);
    let mut incoming = Vec::with_capacity(element_count);
    let mut successors = Vec::with_capacity(element_count);
    let mut predecessors = Vec::with_capacity(element_count);
    for vertex in vertices.iter().copied() {
        outgoing.push(
            DirectedVertexHyperedges::outgoing_hyperedges(&frozen, vertex)
                .map(|hyperedge| ProjectionRelationId(hyperedge.get()))
                .collect(),
        );
        incoming.push(
            DirectedVertexHyperedges::incoming_hyperedges(&frozen, vertex)
                .map(|hyperedge| ProjectionRelationId(hyperedge.get()))
                .collect(),
        );
        successors.push(unique_sorted(
            frozen
                .element_successors(vertex)
                .map(|target| ProjectionElementId(target.get())),
        ));
        predecessors.push(unique_sorted(
            frozen
                .element_predecessors(vertex)
                .map(|source| ProjectionElementId(source.get())),
        ));
    }
    Ok((outgoing, incoming, successors, predecessors))
}

/// Maps interned local element IDs to builder vertex handles, dropping repeats.
///
/// The substrate hypergraph builder rejects a participant side that repeats a
/// vertex, so first-occurrence order is preserved while later duplicates are
/// elided.
///
/// # Performance
///
/// This function is `O(n log n)` for `n` participants.
fn unique_vertices(
    vertices: &[HyperVertexId<u32>],
    locals: &[ProjectionElementId],
) -> Vec<HyperVertexId<u32>> {
    let mut seen = BTreeSet::new();
    locals
        .iter()
        .filter(|local| seen.insert(**local))
        .map(|local| vertices[local_index(*local)])
        .collect()
}

/// Collects an element iterator into a deduplicated, sorted vector.
///
/// # Performance
///
/// This function is `O(n log n)` for `n` yielded elements.
fn unique_sorted(elements: impl Iterator<Item = ProjectionElementId>) -> Vec<ProjectionElementId> {
    elements.collect::<BTreeSet<_>>().into_iter().collect()
}

/// Directed vertex-array build result.
type DirectedVertexArrays = (
    Vec<Vec<ProjectionRelationId>>,
    Vec<Vec<ProjectionRelationId>>,
    Vec<Vec<ProjectionElementId>>,
    Vec<Vec<ProjectionElementId>>,
);

/// Converts a local element count into an ID.
fn projection_element_id(index: usize) -> Result<ProjectionElementId, DbError> {
    u32::try_from(index)
        .map(ProjectionElementId)
        .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Element))
}

/// Converts a local relation count into an ID.
fn projection_relation_id(index: usize) -> Result<ProjectionRelationId, DbError> {
    u32::try_from(index)
        .map(ProjectionRelationId)
        .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Element))
}

/// Converts a local incidence count into an ID.
fn projection_incidence_id(index: usize) -> Result<ProjectionIncidenceId, DbError> {
    u32::try_from(index)
        .map(ProjectionIncidenceId)
        .map_err(|_error| DbError::id_overflow(crate::error::IdFamily::Element))
}

/// Returns a local element index.
fn local_index(id: ProjectionElementId) -> usize {
    usize::try_from(id.0).unwrap_or(usize::MAX)
}

/// Returns a local relation index.
fn local_relation_index(id: ProjectionRelationId) -> usize {
    usize::try_from(id.0).unwrap_or(usize::MAX)
}

/// Returns a local incidence index.
fn local_incidence_index(id: ProjectionIncidenceId) -> usize {
    usize::try_from(id.0).unwrap_or(usize::MAX)
}
