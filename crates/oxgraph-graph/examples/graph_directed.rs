//! Demonstrates implementing `oxgraph-graph` for a tiny directed graph.
//!
//! The example uses graph-specific node, edge, outgoing, and incoming
//! APIs. It does not require callers to reason about topology incidences for the
//! ordinary binary graph path.

use oxgraph_graph::{
    EdgeId as GraphEdgeId, EdgeSourceGraph, EdgeTargetGraph, GraphCounts, IncomingEdgeCount,
    IncomingGraph, NodeId as GraphNodeId, OutgoingEdgeCount, OutgoingGraph,
};
use oxgraph_topology::{TopologyBase, TopologyCounts};

/// Local node handle for the example graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct NodeId(usize);

/// Local edge handle for the example graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct EdgeId(usize);

/// One directed edge stored as source and target nodes.
#[derive(Clone, Copy, Debug)]
struct Edge {
    /// Source node.
    source: NodeId,
    /// Target node.
    target: NodeId,
}

/// Small graph used only by this example.
#[derive(Debug)]
struct TinyDirectedGraph {
    /// Number of nodes in the graph.
    node_count: usize,
    /// Directed edges in canonical edge order.
    edges: &'static [Edge],
    /// Outgoing edge IDs per node.
    outgoing: &'static [&'static [EdgeId]],
    /// Incoming edge IDs per node.
    incoming: &'static [&'static [EdgeId]],
}

impl TopologyBase for TinyDirectedGraph {
    type ElementId = NodeId;
    type RelationId = EdgeId;
}

impl TopologyCounts for TinyDirectedGraph {
    fn element_count(&self) -> usize {
        self.node_count
    }

    fn relation_count(&self) -> usize {
        self.edges.len()
    }
}

/// Graph-facing node ID alias for the example graph.
type GraphNode = GraphNodeId<TinyDirectedGraph>;

/// Graph-facing edge ID alias for the example graph.
type GraphEdge = GraphEdgeId<TinyDirectedGraph>;

impl EdgeSourceGraph for TinyDirectedGraph {
    fn source(&self, edge: GraphEdge) -> GraphNode {
        self.edges[edge.0].source
    }
}

impl EdgeTargetGraph for TinyDirectedGraph {
    fn target(&self, edge: GraphEdge) -> GraphNode {
        self.edges[edge.0].target
    }
}

impl OutgoingGraph for TinyDirectedGraph {
    type OutEdges<'view>
        = core::iter::Copied<core::slice::Iter<'view, EdgeId>>
    where
        Self: 'view;

    fn outgoing_edges(&self, node: GraphNode) -> Self::OutEdges<'_> {
        self.outgoing[node.0].iter().copied()
    }
}

impl IncomingGraph for TinyDirectedGraph {
    type InEdges<'view>
        = core::iter::Copied<core::slice::Iter<'view, EdgeId>>
    where
        Self: 'view;

    fn incoming_edges(&self, node: GraphNode) -> Self::InEdges<'_> {
        self.incoming[node.0].iter().copied()
    }
}

impl OutgoingEdgeCount for TinyDirectedGraph {
    fn out_degree(&self, node: GraphNode) -> usize {
        self.outgoing[node.0].len()
    }
}

impl IncomingEdgeCount for TinyDirectedGraph {
    fn in_degree(&self, node: GraphNode) -> usize {
        self.incoming[node.0].len()
    }
}

/// Returns the tiny graph used by the example.
fn graph() -> TinyDirectedGraph {
    static EDGES: &[Edge] = &[
        Edge {
            source: NodeId(0),
            target: NodeId(1),
        },
        Edge {
            source: NodeId(0),
            target: NodeId(2),
        },
        Edge {
            source: NodeId(2),
            target: NodeId(1),
        },
    ];
    static OUT_0: &[EdgeId] = &[EdgeId(0), EdgeId(1)];
    static OUT_1: &[EdgeId] = &[];
    static OUT_2: &[EdgeId] = &[EdgeId(2)];
    static OUTGOING: &[&[EdgeId]] = &[OUT_0, OUT_1, OUT_2];
    static IN_0: &[EdgeId] = &[];
    static IN_1: &[EdgeId] = &[EdgeId(0), EdgeId(2)];
    static IN_2: &[EdgeId] = &[EdgeId(1)];
    static INCOMING: &[&[EdgeId]] = &[IN_0, IN_1, IN_2];

    TinyDirectedGraph {
        node_count: 3,
        edges: EDGES,
        outgoing: OUTGOING,
        incoming: INCOMING,
    }
}

/// Prints outgoing and incoming traversal from the graph-specific API.
fn main() {
    let graph = graph();
    let node = NodeId(0);

    println!(
        "graph nodes={} edges={}",
        graph.node_count(),
        graph.edge_count()
    );
    println!("out_degree({node:?})={}", graph.out_degree(node));

    for edge in graph.outgoing_edges(node) {
        let target = graph.target(edge);
        println!("out edge={edge:?} target={target:?}");
    }

    let node = NodeId(1);
    println!("in_degree({node:?})={}", graph.in_degree(node));

    for edge in graph.incoming_edges(node) {
        let source = graph.source(edge);
        println!("in edge={edge:?} source={source:?}");
    }
}
