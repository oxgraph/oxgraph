//! Demonstrates implementing `oxgraph-topology` for a tiny directed graph.
//!
//! The example maps graph nodes to topology elements, graph edges to topology
//! relations, and each endpoint participation to one topology incidence.

use oxgraph_topology::{
    ElementIncidences, IncidenceBase, IncidenceCounts, IncidenceElement, IncidenceRelation,
    IncidenceRole, RelationIncidenceCount, RelationIncidences, TopologyBase, TopologyCounts,
};

/// Local node handle for the example graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct NodeId(usize);

/// Local edge handle for the example graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct EdgeId(usize);

/// Local endpoint-incidence handle for the example graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct EndpointId(usize);

/// Endpoint role in a directed edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EdgeRole {
    /// Source endpoint of a directed edge.
    Source,
    /// Target endpoint of a directed edge.
    Target,
}

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
    /// Directed edges in insertion order.
    edges: &'static [Edge],
}

impl TopologyBase for TinyDirectedGraph {
    type ElementId = NodeId;
    type RelationId = EdgeId;
}

impl IncidenceBase for TinyDirectedGraph {
    type IncidenceId = EndpointId;
    type Role = EdgeRole;
}

impl TopologyCounts for TinyDirectedGraph {
    fn element_count(&self) -> usize {
        self.node_count
    }

    fn relation_count(&self) -> usize {
        self.edges.len()
    }
}

impl IncidenceCounts for TinyDirectedGraph {
    fn incidence_count(&self) -> usize {
        self.edges.len() * 2
    }
}

impl RelationIncidences for TinyDirectedGraph {
    type Incidences<'view>
        = core::array::IntoIter<EndpointId, 2>
    where
        Self: 'view;

    fn relation_incidences(&self, relation: EdgeId) -> Self::Incidences<'_> {
        let first = relation.0 * 2;
        [EndpointId(first), EndpointId(first + 1)].into_iter()
    }
}

impl ElementIncidences for TinyDirectedGraph {
    type Incidences<'view>
        = ElementIncidenceIter<'view>
    where
        Self: 'view;

    fn element_incidences(&self, element: NodeId) -> Self::Incidences<'_> {
        ElementIncidenceIter {
            graph: self,
            element,
            next: 0,
        }
    }
}

impl IncidenceElement for TinyDirectedGraph {
    fn incidence_element(&self, incidence: EndpointId) -> NodeId {
        let edge = self.edges[incidence.0 / 2];
        if incidence.0.is_multiple_of(2) {
            edge.source
        } else {
            edge.target
        }
    }
}

impl IncidenceRelation for TinyDirectedGraph {
    fn incidence_relation(&self, incidence: EndpointId) -> EdgeId {
        EdgeId(incidence.0 / 2)
    }
}

impl IncidenceRole for TinyDirectedGraph {
    fn incidence_role(&self, incidence: EndpointId) -> EdgeRole {
        if incidence.0.is_multiple_of(2) {
            EdgeRole::Source
        } else {
            EdgeRole::Target
        }
    }
}

impl RelationIncidenceCount for TinyDirectedGraph {
    fn relation_incidence_count(&self, _relation: EdgeId) -> usize {
        2
    }
}

/// Iterator over incidences attached to one element.
struct ElementIncidenceIter<'view> {
    /// Graph being filtered.
    graph: &'view TinyDirectedGraph,
    /// Element whose incidences should be yielded.
    element: NodeId,
    /// Next flat incidence index to inspect.
    next: usize,
}

impl Iterator for ElementIncidenceIter<'_> {
    type Item = EndpointId;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.graph.incidence_count() {
            let incidence = EndpointId(self.next);
            self.next += 1;

            if self.graph.incidence_element(incidence) == self.element {
                return Some(incidence);
            }
        }

        None
    }
}

/// Prints the two endpoint incidences for edge `0 -> 1`.
fn main() {
    static EDGES: &[Edge] = &[
        Edge {
            source: NodeId(0),
            target: NodeId(1),
        },
        Edge {
            source: NodeId(1),
            target: NodeId(2),
        },
    ];

    let graph = TinyDirectedGraph {
        node_count: 3,
        edges: EDGES,
    };

    for incidence in graph.relation_incidences(EdgeId(0)) {
        println!(
            "edge={:?} incidence={:?} node={:?} role={:?}",
            graph.incidence_relation(incidence),
            incidence,
            graph.incidence_element(incidence),
            graph.incidence_role(incidence)
        );
    }
}
