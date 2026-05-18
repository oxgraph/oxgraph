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
//!
//! # Performance contract
//!
//! Most of the traits and aliases in this crate are graph-vocabulary shadows of
//! topology traits. Unless a specific item documents otherwise, every method,
//! iterator, and alias inherits its performance contract from the underlying
//! topology trait it shadows: `O(1)` for accessors, `O(1)` to construct an
//! iterator plus `O(k)` to yield `k` items, and `perf: unspecified` for
//! capability-bundle marker traits and blanket impls.
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

/// Graph-facing alias for a topology element ID (binary graph node).
pub type NodeId<G> = <G as TopologyBase>::ElementId;

/// Graph-facing alias for a topology relation ID (binary graph edge).
pub type EdgeId<G> = <G as TopologyBase>::RelationId;

/// Graph-facing alias for a topology incidence ID (binary graph endpoint).
///
/// A binary graph endpoint participation is represented as a topology incidence
/// only when a graph view exposes incidence capabilities.
pub type EndpointId<G> = <G as IncidenceBase>::IncidenceId;

/// Graph-facing alias for a topology role used to distinguish source and
/// target endpoint participation on graph views that expose incidences.
pub type EndpointRole<G> = <G as IncidenceBase>::Role;

/// Base capability for graph views over topology storage.
///
/// Graph-facing name for [`TopologyBase`]. Bundles the associated `ElementId`
/// and `RelationId` types under graph vocabulary so generic code can require a
/// graph base contract without naming topology traits directly.
pub trait GraphBase: TopologyBase {}

impl<T> GraphBase for T where T: TopologyBase {}

/// Count capability for a graph view.
///
/// Graph-facing name for [`TopologyCounts`].
pub trait GraphCounts: TopologyCounts {
    /// Returns the number of nodes visible in this graph view.
    fn node_count(&self) -> usize {
        self.element_count()
    }

    /// Returns the number of edges visible in this graph view.
    fn edge_count(&self) -> usize {
        self.relation_count()
    }
}

/// Dense node-index capability for graph views.
///
/// Graph-facing name for [`ElementIndex`]. Lets graph algorithms allocate
/// per-node scratch storage such as visited sets or distance arrays.
pub trait NodeIndex: ElementIndex {
    /// Returns the exclusive upper bound for node indexes in this graph view.
    fn node_bound(&self) -> usize {
        self.element_bound()
    }

    /// Returns the dense index for `node` in this graph view.
    fn node_index(&self, node: Self::ElementId) -> usize {
        self.element_index(node)
    }
}

impl<T> NodeIndex for T where T: ElementIndex {}

/// Dense edge-index capability for graph views.
///
/// Graph-facing name for [`RelationIndex`].
pub trait EdgeIndex: RelationIndex {
    /// Returns the exclusive upper bound for edge indexes in this graph view.
    fn edge_bound(&self) -> usize {
        self.relation_bound()
    }

    /// Returns the dense index for `edge` in this graph view.
    fn edge_index(&self, edge: Self::RelationId) -> usize {
        self.relation_index(edge)
    }
}

impl<T> EdgeIndex for T where T: RelationIndex {}

/// Dense endpoint-index capability for graph views with incidences.
///
/// Graph-facing name for [`IncidenceIndex`].
pub trait EndpointIndex: IncidenceIndex {
    /// Returns the exclusive upper bound for endpoint indexes in this graph view.
    fn endpoint_bound(&self) -> usize {
        self.incidence_bound()
    }

    /// Returns the dense index for `endpoint` in this graph view.
    fn endpoint_index(&self, endpoint: Self::IncidenceId) -> usize {
        self.incidence_index(endpoint)
    }
}

impl<T> EndpointIndex for T where T: IncidenceIndex {}

/// Node-ID containment capability for graph views.
///
/// Graph-facing name for [`ContainsElement`].
pub trait ContainsNode: ContainsElement {
    /// Returns whether `node` is valid and visible in this graph view.
    fn contains_node(&self, node: Self::ElementId) -> bool {
        self.contains_element(node)
    }
}

impl<T> ContainsNode for T where T: ContainsElement {}

/// Edge-ID containment capability for graph views.
///
/// Graph-facing name for [`ContainsRelation`].
pub trait ContainsEdge: ContainsRelation {
    /// Returns whether `edge` is valid and visible in this graph view.
    fn contains_edge(&self, edge: Self::RelationId) -> bool {
        self.contains_relation(edge)
    }
}

impl<T> ContainsEdge for T where T: ContainsRelation {}

/// Endpoint-ID containment capability for graph views with incidences.
///
/// Graph-facing name for [`ContainsIncidence`].
pub trait ContainsEndpoint: ContainsIncidence {
    /// Returns whether `endpoint` is valid and visible in this graph view.
    fn contains_endpoint(&self, endpoint: Self::IncidenceId) -> bool {
        self.contains_incidence(endpoint)
    }
}

impl<T> ContainsEndpoint for T where T: ContainsIncidence {}

/// Capability for resolving directed edge sources.
///
/// Separated from [`EdgeTargetGraph`] because some layouts (e.g. outgoing-only
/// CSR) can resolve targets cheaply but need extra indexing for sources.
pub trait EdgeSourceGraph: TopologyBase {
    /// Returns the source node of `edge`. `edge` must be a valid edge ID.
    fn source(&self, edge: Self::RelationId) -> Self::ElementId;
}

/// Capability for resolving directed edge targets.
///
/// The endpoint capability needed by outgoing traversal: an outgoing edge ID
/// can be mapped to the node it reaches without requiring the backend to
/// support reverse source lookup.
pub trait EdgeTargetGraph: TopologyBase {
    /// Returns the target node of `edge`. `edge` must be a valid edge ID.
    fn target(&self, edge: Self::RelationId) -> Self::ElementId;
}

/// Capability for resolving both directed edge endpoints.
///
/// Binary graph form of complete relation endpoint lookup. Implementations may
/// back it with an edge table, compact arrays, generated edges, or validated
/// snapshot sections.
pub trait EdgeEndpointGraph: EdgeSourceGraph + EdgeTargetGraph {
    /// Returns `(source, target)` for `edge`. `edge` must be a valid edge ID.
    fn endpoints(&self, edge: Self::RelationId) -> (Self::ElementId, Self::ElementId) {
        (self.source(edge), self.target(edge))
    }
}

impl<T> EdgeEndpointGraph for T where T: EdgeSourceGraph + EdgeTargetGraph {}

/// Capability for traversing outgoing edges from a source node.
///
/// The generic associated iterator type lets each backend return its own
/// concrete iterator without allocation or dynamic dispatch.
pub trait OutgoingGraph: TopologyBase {
    /// Iterator over edge IDs leaving one source node.
    type OutEdges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Returns edges whose source is `node`.
    fn outgoing_edges(&self, node: Self::ElementId) -> Self::OutEdges<'_>;
}

/// Capability for traversing incoming edges to a target node.
///
/// Separate from [`OutgoingGraph`] because many layouts only provide one
/// direction cheaply.
pub trait IncomingGraph: TopologyBase {
    /// Iterator over edge IDs entering one target node.
    type InEdges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Returns edges whose target is `node`.
    fn incoming_edges(&self, node: Self::ElementId) -> Self::InEdges<'_>;
}

/// Capability for traversing directly reachable outgoing neighbor nodes.
///
/// Graph-facing name for [`ElementSuccessors`]. Answers `node -> successor
/// nodes` without requiring callers to materialize outgoing edge IDs and
/// resolve each edge target. Implementations define whether parallel edges
/// produce repeated neighbor nodes; those that preserve graph edge order and
/// multiplicity should document that behavior.
pub trait OutgoingNeighborsGraph: ElementSuccessors {
    /// Iterator over nodes directly reachable from one source node.
    type OutNeighbors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns neighbor nodes reached by outgoing edges from `node`.
    fn outgoing_neighbors(&self, node: Self::ElementId) -> Self::OutNeighbors<'_>;
}

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
/// Graph-facing name for [`ElementPredecessors`]. Answers `node ->
/// predecessor nodes` without requiring callers to materialize incoming edge
/// IDs and resolve each edge source.
pub trait IncomingNeighborsGraph: ElementPredecessors {
    /// Iterator over nodes that have incoming edges to one target node.
    type InNeighbors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns predecessor nodes with incoming edges to `node`.
    fn incoming_neighbors(&self, node: Self::ElementId) -> Self::InNeighbors<'_>;
}

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
/// Pairs with [`OutgoingGraph`] for backends that can report out-degree
/// without traversal; does not require outgoing traversal support by itself.
pub trait OutgoingEdgeCount: TopologyBase {
    /// Returns the number of edges leaving `node`.
    fn out_degree(&self, node: Self::ElementId) -> usize;
}

/// Exact incoming-edge count capability.
///
/// Pairs with [`IncomingGraph`] for backends that can report in-degree
/// without traversal; does not require incoming traversal support by itself.
pub trait IncomingEdgeCount: TopologyBase {
    /// Returns the number of edges entering `node`.
    fn in_degree(&self, node: Self::ElementId) -> usize;
}

/// Convenience marker bundling endpoint resolution and both traversal
/// directions. One-direction layouts should implement the smaller capability
/// traits they can provide instead.
pub trait DirectedGraph: EdgeEndpointGraph + OutgoingGraph + IncomingGraph {}

impl<T> DirectedGraph for T where T: EdgeEndpointGraph + OutgoingGraph + IncomingGraph {}

/// Convenience marker for forward directed graph traversal (target lookup +
/// outgoing edge iteration).
pub trait ForwardGraph: EdgeTargetGraph + OutgoingGraph {}

impl<T> ForwardGraph for T where T: EdgeTargetGraph + OutgoingGraph {}

/// Convenience marker for reverse directed graph traversal (source lookup +
/// incoming edge iteration). Expected to be provided by CSC-backed views.
pub trait ReverseGraph: EdgeSourceGraph + IncomingGraph {}

impl<T> ReverseGraph for T where T: EdgeSourceGraph + IncomingGraph {}
