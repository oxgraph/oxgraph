//! Tests that `oxgraph-graph` traits support static-dispatch graph consumers.

use oxgraph_graph::{
    ContainsEdge, ContainsElement, ContainsEndpoint, ContainsIncidence, ContainsNode,
    ContainsRelation, DirectedGraph, EdgeEndpointGraph, EdgeIndex, EdgeSourceGraph,
    EdgeTargetGraph, ElementIndex, ElementPredecessors, ElementSuccessors, EndpointIndex,
    ForwardGraph, GraphCounts, IncidenceBase, IncidenceIndex, IncomingEdgeCount, IncomingGraph,
    IncomingNeighborsGraph, NodeId as GraphNodeId, NodeIndex, OutgoingEdgeCount, OutgoingGraph,
    OutgoingNeighborsGraph, RelationIndex, ReverseGraph, TopologyBase, TopologyCounts,
};
use proptest::prelude::*;

/// Node identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct NodeId(usize);

/// Edge identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct EdgeId(usize);

/// Endpoint identifier used by the incidence-capable test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct EndpointId(usize);

/// One directed edge in the test fixture.
#[derive(Clone, Copy, Debug)]
struct Edge {
    /// Source node.
    source: NodeId,
    /// Target node.
    target: NodeId,
}

/// Tiny directed graph fixture with precomputed adjacency indexes.
#[derive(Debug)]
struct FixtureGraph {
    /// Number of nodes in the fixture.
    node_count: usize,
    /// Edge table in canonical edge order.
    edges: &'static [Edge],
    /// Outgoing edge IDs per node.
    outgoing: &'static [&'static [EdgeId]],
    /// Incoming edge IDs per node.
    incoming: &'static [&'static [EdgeId]],
    /// Direct outgoing neighbor node IDs per node.
    outgoing_neighbors: &'static [&'static [NodeId]],
    /// Direct incoming neighbor node IDs per node.
    incoming_neighbors: &'static [&'static [NodeId]],
}

impl TopologyBase for FixtureGraph {
    type ElementId = NodeId;
    type RelationId = EdgeId;
}

impl TopologyCounts for FixtureGraph {
    fn element_count(&self) -> usize {
        self.node_count
    }

    fn relation_count(&self) -> usize {
        self.edges.len()
    }
}

impl GraphCounts for FixtureGraph {}

impl ElementIndex for FixtureGraph {
    fn element_bound(&self) -> usize {
        self.node_count
    }

    fn element_index(&self, element: NodeId) -> usize {
        element.0
    }
}

impl ContainsElement for FixtureGraph {
    fn contains_element(&self, element: NodeId) -> bool {
        element.0 < self.node_count
    }
}

impl RelationIndex for FixtureGraph {
    fn relation_bound(&self) -> usize {
        self.edges.len()
    }

    fn relation_index(&self, relation: EdgeId) -> usize {
        relation.0
    }
}

impl ContainsRelation for FixtureGraph {
    fn contains_relation(&self, relation: EdgeId) -> bool {
        relation.0 < self.edges.len()
    }
}

impl EdgeSourceGraph for FixtureGraph {
    fn source(&self, edge: EdgeId) -> NodeId {
        self.edges[edge.0].source
    }
}

impl EdgeTargetGraph for FixtureGraph {
    fn target(&self, edge: EdgeId) -> NodeId {
        self.edges[edge.0].target
    }
}

impl OutgoingGraph for FixtureGraph {
    type OutEdges<'view>
        = core::iter::Copied<core::slice::Iter<'view, EdgeId>>
    where
        Self: 'view;

    fn outgoing_edges(&self, node: NodeId) -> Self::OutEdges<'_> {
        self.outgoing[node.0].iter().copied()
    }
}

impl IncomingGraph for FixtureGraph {
    type InEdges<'view>
        = core::iter::Copied<core::slice::Iter<'view, EdgeId>>
    where
        Self: 'view;

    fn incoming_edges(&self, node: NodeId) -> Self::InEdges<'_> {
        self.incoming[node.0].iter().copied()
    }
}

impl ElementSuccessors for FixtureGraph {
    type Successors<'view>
        = core::iter::Copied<core::slice::Iter<'view, NodeId>>
    where
        Self: 'view;

    fn element_successors(&self, node: NodeId) -> Self::Successors<'_> {
        self.outgoing_neighbors[node.0].iter().copied()
    }
}

impl ElementPredecessors for FixtureGraph {
    type Predecessors<'view>
        = core::iter::Copied<core::slice::Iter<'view, NodeId>>
    where
        Self: 'view;

    fn element_predecessors(&self, node: NodeId) -> Self::Predecessors<'_> {
        self.incoming_neighbors[node.0].iter().copied()
    }
}

impl OutgoingEdgeCount for FixtureGraph {
    fn out_degree(&self, node: NodeId) -> usize {
        self.outgoing[node.0].len()
    }
}

impl IncomingEdgeCount for FixtureGraph {
    fn in_degree(&self, node: NodeId) -> usize {
        self.incoming[node.0].len()
    }
}

/// Tiny graph fixture used only to verify endpoint-index delegation.
#[derive(Debug)]
struct EndpointFixture;

impl TopologyBase for EndpointFixture {
    type ElementId = NodeId;
    type RelationId = EdgeId;
}

impl IncidenceBase for EndpointFixture {
    type IncidenceId = EndpointId;
    type Role = ();
}

impl IncidenceIndex for EndpointFixture {
    fn incidence_bound(&self) -> usize {
        8
    }

    fn incidence_index(&self, incidence: EndpointId) -> usize {
        incidence.0
    }
}

impl ContainsIncidence for EndpointFixture {
    fn contains_incidence(&self, incidence: EndpointId) -> bool {
        incidence.0 < 8
    }
}

/// Returns outgoing target nodes through static dispatch.
fn outgoing_targets<T>(graph: &T, node: GraphNodeId<T>) -> Vec<GraphNodeId<T>>
where
    T: EdgeTargetGraph + OutgoingGraph,
{
    graph
        .outgoing_edges(node)
        .map(|edge| graph.target(edge))
        .collect()
}

/// Returns incoming source nodes through static dispatch.
fn incoming_sources<T>(graph: &T, node: GraphNodeId<T>) -> Vec<GraphNodeId<T>>
where
    T: EdgeSourceGraph + IncomingGraph,
{
    graph
        .incoming_edges(node)
        .map(|edge| graph.source(edge))
        .collect()
}

/// Returns outgoing neighbor nodes through static dispatch.
fn outgoing_neighbors<T>(graph: &T, node: GraphNodeId<T>) -> Vec<GraphNodeId<T>>
where
    T: OutgoingNeighborsGraph,
{
    graph.outgoing_neighbors(node).collect()
}

/// Returns incoming neighbor nodes through static dispatch.
fn incoming_neighbors<T>(graph: &T, node: GraphNodeId<T>) -> Vec<GraphNodeId<T>>
where
    T: IncomingNeighborsGraph,
{
    graph.incoming_neighbors(node).collect()
}

/// Counts outgoing targets through the forward traversal bundle.
fn forward_target_count<T>(graph: &T, node: GraphNodeId<T>) -> usize
where
    T: ForwardGraph,
{
    graph.outgoing_edges(node).count()
}

/// Counts incoming sources through the reverse traversal bundle.
fn reverse_source_count<T>(graph: &T, node: GraphNodeId<T>) -> usize
where
    T: ReverseGraph,
{
    graph.incoming_edges(node).count()
}

/// Returns endpoint pairs through the full bidirectional directed graph bundle.
fn directed_endpoint_pair<T>(graph: &T, edge: T::RelationId) -> (T::ElementId, T::ElementId)
where
    T: DirectedGraph,
{
    graph.endpoints(edge)
}

/// Returns a graph shaped like `0 -> 1`, `1 -> 2`, `0 -> 2`, `2 -> 3`.
fn fixture() -> FixtureGraph {
    static EDGES: &[Edge] = &[
        Edge {
            source: NodeId(0),
            target: NodeId(1),
        },
        Edge {
            source: NodeId(1),
            target: NodeId(2),
        },
        Edge {
            source: NodeId(0),
            target: NodeId(2),
        },
        Edge {
            source: NodeId(2),
            target: NodeId(3),
        },
    ];
    static OUT_0: &[EdgeId] = &[EdgeId(0), EdgeId(2)];
    static OUT_1: &[EdgeId] = &[EdgeId(1)];
    static OUT_2: &[EdgeId] = &[EdgeId(3)];
    static OUT_3: &[EdgeId] = &[];
    static OUTGOING: &[&[EdgeId]] = &[OUT_0, OUT_1, OUT_2, OUT_3];
    static IN_0: &[EdgeId] = &[];
    static IN_1: &[EdgeId] = &[EdgeId(0)];
    static IN_2: &[EdgeId] = &[EdgeId(1), EdgeId(2)];
    static IN_3: &[EdgeId] = &[EdgeId(3)];
    static INCOMING: &[&[EdgeId]] = &[IN_0, IN_1, IN_2, IN_3];
    static OUT_N_0: &[NodeId] = &[NodeId(1), NodeId(2)];
    static OUT_N_1: &[NodeId] = &[NodeId(2)];
    static OUT_N_2: &[NodeId] = &[NodeId(3)];
    static OUT_N_3: &[NodeId] = &[];
    static OUTGOING_NEIGHBORS: &[&[NodeId]] = &[OUT_N_0, OUT_N_1, OUT_N_2, OUT_N_3];
    static IN_N_0: &[NodeId] = &[];
    static IN_N_1: &[NodeId] = &[NodeId(0)];
    static IN_N_2: &[NodeId] = &[NodeId(1), NodeId(0)];
    static IN_N_3: &[NodeId] = &[NodeId(2)];
    static INCOMING_NEIGHBORS: &[&[NodeId]] = &[IN_N_0, IN_N_1, IN_N_2, IN_N_3];

    FixtureGraph {
        node_count: 4,
        edges: EDGES,
        outgoing: OUTGOING,
        incoming: INCOMING,
        outgoing_neighbors: OUTGOING_NEIGHBORS,
        incoming_neighbors: INCOMING_NEIGHBORS,
    }
}

#[test]
fn generic_consumer_reads_outgoing_targets() {
    let graph = fixture();

    assert_eq!(outgoing_targets(&graph, NodeId(0)), [NodeId(1), NodeId(2)]);
    assert_eq!(outgoing_targets(&graph, NodeId(3)), []);
}

#[test]
fn generic_consumer_reads_incoming_sources() {
    let graph = fixture();

    assert_eq!(incoming_sources(&graph, NodeId(2)), [NodeId(1), NodeId(0)]);
    assert_eq!(incoming_sources(&graph, NodeId(0)), []);
}

#[test]
fn generic_consumer_reads_outgoing_neighbors_directly() {
    let graph = fixture();

    assert_eq!(
        outgoing_neighbors(&graph, NodeId(0)),
        [NodeId(1), NodeId(2)]
    );
    assert_eq!(outgoing_neighbors(&graph, NodeId(3)), []);
}

#[test]
fn generic_consumer_reads_incoming_neighbors_directly() {
    let graph = fixture();

    assert_eq!(
        incoming_neighbors(&graph, NodeId(2)),
        [NodeId(1), NodeId(0)]
    );
    assert_eq!(incoming_neighbors(&graph, NodeId(0)), []);
}

#[test]
fn counts_describe_visible_graph() {
    let graph = fixture();

    assert_eq!(graph.node_count(), 4);
    assert_eq!(graph.edge_count(), 4);
}

#[test]
fn graph_index_aliases_delegate_to_topology_indexes() {
    let graph = fixture();

    assert_eq!(graph.node_bound(), graph.element_bound());
    assert_eq!(graph.node_index(NodeId(2)), graph.element_index(NodeId(2)));
    assert_eq!(graph.edge_bound(), graph.relation_bound());
    assert_eq!(graph.edge_index(EdgeId(3)), graph.relation_index(EdgeId(3)));
}

#[test]
fn endpoint_index_alias_delegates_to_incidence_index() {
    let graph = EndpointFixture;

    assert_eq!(graph.endpoint_bound(), graph.incidence_bound());
    assert_eq!(
        graph.endpoint_index(EndpointId(3)),
        graph.incidence_index(EndpointId(3))
    );
}

#[test]
fn graph_containment_aliases_delegate_to_topology_containment() {
    let graph = fixture();
    let endpoint_graph = EndpointFixture;

    assert_eq!(
        graph.contains_node(NodeId(2)),
        graph.contains_element(NodeId(2))
    );
    assert!(!graph.contains_node(NodeId(4)));
    assert_eq!(
        graph.contains_edge(EdgeId(3)),
        graph.contains_relation(EdgeId(3))
    );
    assert!(!graph.contains_edge(EdgeId(4)));
    assert_eq!(
        endpoint_graph.contains_endpoint(EndpointId(3)),
        endpoint_graph.contains_incidence(EndpointId(3))
    );
    assert!(!endpoint_graph.contains_endpoint(EndpointId(8)));
}

#[test]
fn graph_bundle_blanket_impls_are_available() {
    let graph = fixture();

    assert_eq!(forward_target_count(&graph, NodeId(0)), 2);
    assert_eq!(reverse_source_count(&graph, NodeId(2)), 2);
    assert_eq!(
        directed_endpoint_pair(&graph, EdgeId(2)),
        (NodeId(0), NodeId(2))
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Out-degree matches outgoing traversal count for every fixture node.
    #[test]
    fn out_degree_matches_iterator_count(index in 0usize..4) {
        let graph = fixture();
        let node = NodeId(index);

        prop_assert_eq!(graph.out_degree(node), graph.outgoing_edges(node).count());
    }

    /// In-degree matches incoming traversal count for every fixture node.
    #[test]
    fn in_degree_matches_iterator_count(index in 0usize..4) {
        let graph = fixture();
        let node = NodeId(index);

        prop_assert_eq!(graph.in_degree(node), graph.incoming_edges(node).count());
    }

    /// Combined endpoint lookup agrees with individual source and target lookup.
    #[test]
    fn endpoints_match_source_and_target(index in 0usize..4) {
        let graph = fixture();
        let edge = EdgeId(index);

        prop_assert_eq!(graph.endpoints(edge), (graph.source(edge), graph.target(edge)));
    }
}
