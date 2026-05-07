//! Storage-agnostic traits for binary graph views.
//!
//! `oxgraph-graph` is the binary graph specialization above `oxgraph-topology`. Use it
//! to write generic graph consumers over node/edge vocabulary: endpoint lookup,
//! outgoing traversal, incoming traversal, and degree queries.
//!
//! Graph views use topology elements as nodes and topology relations as edges.
//! Implement concrete layouts, snapshots, builders, mutation systems, payloads,
//! and algorithms in higher-level crates by implementing these read-view
//! capabilities.
#![no_std]

#[cfg(kani)]
extern crate kani;

pub use oxgraph_topology::{
    CanonicalElementIdentity, CanonicalIncidenceIdentity, CanonicalRelationIdentity,
    ContainsElement, ContainsIncidence, ContainsRelation, ElementIncidenceCount, ElementIncidences,
    ElementIndex, ElementPredecessors, ElementSuccessors, ElementWeight, IncidenceBase,
    IncidenceCounts, IncidenceElement, IncidenceIndex, IncidenceRelation, IncidenceRole,
    IncidenceWeight, LocalElementIdentity, LocalIncidenceIdentity, LocalRelationIdentity,
    RelationIncidenceCount, RelationIncidences, RelationIndex, RelationWeight, TopologyBase,
    TopologyCounts, TopologyId,
};

/// Graph-facing alias for a topology element ID.
///
/// Binary graph views use topology elements as graph nodes. This alias gives
/// graph-facing code node vocabulary for generic function signatures and return
/// types.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`TopologyBase::ElementId`] type.
pub type NodeId<G> = <G as TopologyBase>::ElementId;

/// Graph-facing alias for a topology relation ID.
///
/// Binary graph views use topology relations as graph edges. This alias gives
/// graph-facing code edge vocabulary for generic function signatures and return
/// types.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`TopologyBase::RelationId`] type.
pub type EdgeId<G> = <G as TopologyBase>::RelationId;

/// Graph-facing alias for a topology incidence ID.
///
/// A binary graph endpoint participation is represented as a topology incidence
/// only when a graph view exposes incidence capabilities.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`IncidenceBase::IncidenceId`] type.
pub type EndpointId<G> = <G as IncidenceBase>::IncidenceId;

/// Graph-facing alias for a topology role.
///
/// A graph view can use the topology role to distinguish source and target
/// endpoint participation when it exposes incidence capabilities.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`IncidenceBase::Role`] type.
pub type EndpointRole<G> = <G as IncidenceBase>::Role;

/// Base capability for graph views over topology storage.
///
/// This is the graph-facing name for [`TopologyBase`]. It bundles the associated
/// `ElementId` and `RelationId` types under graph vocabulary so generic code can
/// require a graph base contract without naming topology traits directly.
///
/// # Performance
///
/// `perf: unspecified`; this trait carries only associated types.
pub trait GraphBase: TopologyBase {}

/// Blanket implementation for any view that implements [`TopologyBase`].
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`TopologyBase`].
impl<T> GraphBase for T where T: TopologyBase {}

/// Count capability for a graph view.
///
/// This trait gives graph-facing names to [`TopologyCounts`] values for views
/// that represent binary graphs.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait GraphCounts: TopologyCounts {
    /// Returns the number of nodes visible in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn node_count(&self) -> usize {
        self.element_count()
    }

    /// Returns the number of edges visible in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn edge_count(&self) -> usize {
        self.relation_count()
    }
}

/// Dense node-index capability for graph views.
///
/// This is the graph-facing name for [`ElementIndex`]. It lets graph algorithms
/// allocate per-node scratch storage such as visited sets or distance arrays
/// without requiring graph users to think in topology vocabulary.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait NodeIndex: ElementIndex {
    /// Returns the exclusive upper bound for node indexes in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn node_bound(&self) -> usize {
        self.element_bound()
    }

    /// Returns the dense index for `node` in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn node_index(&self, node: Self::ElementId) -> usize {
        self.element_index(node)
    }
}

/// Blanket implementation for graph views with dense element indexes.
///
/// Any graph view that implements [`ElementIndex`] automatically exposes the
/// graph-facing [`NodeIndex`] vocabulary.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ElementIndex`].
impl<T> NodeIndex for T where T: ElementIndex {}

/// Dense edge-index capability for graph views.
///
/// This is the graph-facing name for [`RelationIndex`]. It lets graph
/// algorithms allocate per-edge scratch storage without requiring graph users to
/// think in topology vocabulary.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait EdgeIndex: RelationIndex {
    /// Returns the exclusive upper bound for edge indexes in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn edge_bound(&self) -> usize {
        self.relation_bound()
    }

    /// Returns the dense index for `edge` in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn edge_index(&self, edge: Self::RelationId) -> usize {
        self.relation_index(edge)
    }
}

/// Blanket implementation for graph views with dense relation indexes.
///
/// Any graph view that implements [`RelationIndex`] automatically exposes the
/// graph-facing [`EdgeIndex`] vocabulary.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`RelationIndex`].
impl<T> EdgeIndex for T where T: RelationIndex {}

/// Dense endpoint-index capability for graph views with incidences.
///
/// This is the graph-facing name for [`IncidenceIndex`]. It lets graph
/// algorithms allocate per-endpoint scratch storage for views that expose graph
/// endpoints as topology incidences.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait EndpointIndex: IncidenceIndex {
    /// Returns the exclusive upper bound for endpoint indexes in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn endpoint_bound(&self) -> usize {
        self.incidence_bound()
    }

    /// Returns the dense index for `endpoint` in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn endpoint_index(&self, endpoint: Self::IncidenceId) -> usize {
        self.incidence_index(endpoint)
    }
}

/// Blanket implementation for graph views with dense incidence indexes.
///
/// Any graph view that implements [`IncidenceIndex`] automatically exposes the
/// graph-facing [`EndpointIndex`] vocabulary.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`IncidenceIndex`].
impl<T> EndpointIndex for T where T: IncidenceIndex {}

/// Node-ID containment capability for graph views.
///
/// This is the graph-facing name for [`ContainsElement`]. It answers whether a
/// node ID is valid and visible in this graph view.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsNode: ContainsElement {
    /// Returns whether `node` is valid and visible in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_node(&self, node: Self::ElementId) -> bool {
        self.contains_element(node)
    }
}

/// Blanket implementation for graph views with element containment.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ContainsElement`].
impl<T> ContainsNode for T where T: ContainsElement {}

/// Edge-ID containment capability for graph views.
///
/// This is the graph-facing name for [`ContainsRelation`]. It answers whether an
/// edge ID is valid and visible in this graph view.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsEdge: ContainsRelation {
    /// Returns whether `edge` is valid and visible in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_edge(&self, edge: Self::RelationId) -> bool {
        self.contains_relation(edge)
    }
}

/// Blanket implementation for graph views with relation containment.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ContainsRelation`].
impl<T> ContainsEdge for T where T: ContainsRelation {}

/// Endpoint-ID containment capability for graph views with incidences.
///
/// This is the graph-facing name for [`ContainsIncidence`]. It answers whether
/// an endpoint ID is valid and visible in this graph view.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsEndpoint: ContainsIncidence {
    /// Returns whether `endpoint` is valid and visible in this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_endpoint(&self, endpoint: Self::IncidenceId) -> bool {
        self.contains_incidence(endpoint)
    }
}

/// Blanket implementation for graph views with incidence containment.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ContainsIncidence`].
impl<T> ContainsEndpoint for T where T: ContainsIncidence {}

/// Capability for resolving directed edge sources.
///
/// This is separated from [`EdgeTargetGraph`] because some layouts, such as
/// outgoing-only CSR, can resolve targets cheaply but need extra indexing to
/// resolve sources with the same complexity.
///
/// # Performance
///
/// Source lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait EdgeSourceGraph: TopologyBase {
    /// Returns the source node of `edge`.
    ///
    /// `edge` must be a valid edge ID from this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn source(&self, edge: Self::RelationId) -> Self::ElementId;
}

/// Capability for resolving directed edge targets.
///
/// This is the endpoint capability needed by outgoing traversal algorithms: an
/// outgoing edge ID can be mapped to the node it reaches without requiring the
/// backend to support reverse source lookup.
///
/// # Performance
///
/// Target lookup should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait EdgeTargetGraph: TopologyBase {
    /// Returns the target node of `edge`.
    ///
    /// `edge` must be a valid edge ID from this graph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn target(&self, edge: Self::RelationId) -> Self::ElementId;
}

/// Capability for resolving both directed edge endpoints.
///
/// This is the binary graph form of complete relation endpoint lookup.
/// Implementations may back it with an edge table, compact arrays, generated
/// edges, or validated snapshot sections.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`EdgeSourceGraph`] and
/// [`EdgeTargetGraph`].
pub trait EdgeEndpointGraph: EdgeSourceGraph + EdgeTargetGraph {
    /// Returns `(source, target)` for `edge`.
    ///
    /// `edge` must be a valid edge ID from this graph view.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; performance is inherited from [`Self::source`] and
    /// [`Self::target`].
    fn endpoints(&self, edge: Self::RelationId) -> (Self::ElementId, Self::ElementId) {
        (self.source(edge), self.target(edge))
    }
}

/// Blanket implementation for graph views that can resolve both endpoints.
///
/// Any graph view that implements both source and target lookup automatically
/// implements [`EdgeEndpointGraph`].
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`EdgeSourceGraph`] and
/// [`EdgeTargetGraph`].
impl<T> EdgeEndpointGraph for T where T: EdgeSourceGraph + EdgeTargetGraph {}

/// Capability for traversing outgoing edges from a source node.
///
/// The generic associated iterator type lets each backend return its own
/// concrete iterator without allocation or dynamic dispatch.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` outgoing edges should be `O(k)` because the
/// output itself has length `k`.
pub trait OutgoingGraph: TopologyBase {
    /// Iterator over edge IDs leaving one source node.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type OutEdges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Returns edges whose source is `node`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` edges is expected
    /// `O(k)`.
    fn outgoing_edges(&self, node: Self::ElementId) -> Self::OutEdges<'_>;
}

/// Capability for traversing incoming edges to a target node.
///
/// This capability is separate from [`OutgoingGraph`] because many layouts only
/// provide one direction cheaply.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` incoming edges should be `O(k)` because the
/// output itself has length `k`.
pub trait IncomingGraph: TopologyBase {
    /// Iterator over edge IDs entering one target node.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type InEdges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Returns edges whose target is `node`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` edges is expected
    /// `O(k)`.
    fn incoming_edges(&self, node: Self::ElementId) -> Self::InEdges<'_>;
}

/// Capability for traversing directly reachable outgoing neighbor nodes.
///
/// This is the graph-facing name for [`ElementSuccessors`]. It answers
/// `node -> successor nodes` without requiring callers to materialize
/// outgoing edge IDs and resolve each edge target.
///
/// Implementations define whether parallel edges produce repeated neighbor
/// nodes. Implementations that preserve graph edge order and multiplicity
/// should document that behavior.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` neighbors should be `O(k)` and should not
/// allocate unless the implementation documents otherwise.
pub trait OutgoingNeighborsGraph: ElementSuccessors {
    /// Iterator over nodes directly reachable from one source node.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type OutNeighbors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns neighbor nodes reached by outgoing edges from `node`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` neighbors is
    /// expected `O(k)`.
    fn outgoing_neighbors(&self, node: Self::ElementId) -> Self::OutNeighbors<'_>;
}

/// Blanket implementation for graph views with element-level successor traversal.
///
/// Any view that implements [`ElementSuccessors`] automatically exposes
/// graph-facing outgoing-neighbor traversal under node vocabulary. The
/// associated iterator type forwards to [`ElementSuccessors::Successors`].
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ElementSuccessors`].
impl<T> OutgoingNeighborsGraph for T
where
    T: ElementSuccessors,
{
    type OutNeighbors<'view>
        = <T as ElementSuccessors>::Successors<'view>
    where
        T: 'view;

    fn outgoing_neighbors(&self, node: Self::ElementId) -> Self::OutNeighbors<'_> {
        <Self as ElementSuccessors>::element_successors(self, node)
    }
}

/// Capability for traversing directly preceding incoming neighbor nodes.
///
/// This is the graph-facing name for [`ElementPredecessors`]. It answers
/// `node -> predecessor nodes` without requiring callers to materialize
/// incoming edge IDs and resolve each edge source.
///
/// Implementations define whether parallel edges produce repeated predecessor
/// nodes. Implementations that preserve graph edge order and multiplicity
/// should document that behavior.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` neighbors should be `O(k)` and should not
/// allocate unless the implementation documents otherwise.
pub trait IncomingNeighborsGraph: ElementPredecessors {
    /// Iterator over nodes that have incoming edges to one target node.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type InNeighbors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns predecessor nodes with incoming edges to `node`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` neighbors is
    /// expected `O(k)`.
    fn incoming_neighbors(&self, node: Self::ElementId) -> Self::InNeighbors<'_>;
}

/// Blanket implementation for graph views with element-level predecessor traversal.
///
/// Any view that implements [`ElementPredecessors`] automatically exposes
/// graph-facing incoming-neighbor traversal under node vocabulary. The
/// associated iterator type forwards to [`ElementPredecessors::Predecessors`].
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ElementPredecessors`].
impl<T> IncomingNeighborsGraph for T
where
    T: ElementPredecessors,
{
    type InNeighbors<'view>
        = <T as ElementPredecessors>::Predecessors<'view>
    where
        T: 'view;

    fn incoming_neighbors(&self, node: Self::ElementId) -> Self::InNeighbors<'_> {
        <Self as ElementPredecessors>::element_predecessors(self, node)
    }
}

/// Exact outgoing-edge count capability.
///
/// This pairs with [`OutgoingGraph`] for backends that can report out-degree
/// without traversal. The trait does not require outgoing traversal support by
/// itself.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait OutgoingEdgeCount: TopologyBase {
    /// Returns the number of edges leaving `node`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn out_degree(&self, node: Self::ElementId) -> usize;
}

/// Exact incoming-edge count capability.
///
/// This pairs with [`IncomingGraph`] for backends that can report in-degree
/// without traversal. The trait does not require incoming traversal support by
/// itself.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait IncomingEdgeCount: TopologyBase {
    /// Returns the number of edges entering `node`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn in_degree(&self, node: Self::ElementId) -> usize;
}

/// Convenience trait for bidirectional directed graph views.
///
/// This trait has no methods of its own. It names the common capability bundle
/// for generic code that needs edge endpoints, outgoing traversal, and incoming
/// traversal. Directed graph views that only support one direction, such as an
/// outgoing-only CSR layout, should implement the smaller capability traits they
/// can provide instead of this full bundle.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
pub trait DirectedGraph: EdgeEndpointGraph + OutgoingGraph + IncomingGraph {}

/// Blanket implementation for complete directed graph views.
///
/// Any graph view that can resolve endpoints and traverse both directions
/// automatically implements [`DirectedGraph`].
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
impl<T> DirectedGraph for T where T: EdgeEndpointGraph + OutgoingGraph + IncomingGraph {}

/// Convenience trait for forward directed graph traversal.
///
/// This trait has no methods of its own. It names the durable capability bundle
/// needed by algorithms that traverse outgoing edges and resolve each edge's
/// target node.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
pub trait ForwardGraph: EdgeTargetGraph + OutgoingGraph {}

/// Blanket implementation for graph views with outgoing traversal and target lookup.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`EdgeTargetGraph`] and
/// [`OutgoingGraph`].
impl<T> ForwardGraph for T where T: EdgeTargetGraph + OutgoingGraph {}

/// Convenience trait for reverse directed graph traversal.
///
/// This trait has no methods of its own. It names the durable capability bundle
/// needed by algorithms that traverse incoming edges and resolve each edge's
/// source node. CSC-backed graph views are expected to provide this capability
/// when they expose efficient incoming traversal.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
pub trait ReverseGraph: EdgeSourceGraph + IncomingGraph {}

/// Blanket implementation for graph views with incoming traversal and source lookup.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`EdgeSourceGraph`] and
/// [`IncomingGraph`].
impl<T> ReverseGraph for T where T: EdgeSourceGraph + IncomingGraph {}
