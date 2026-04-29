//! Demonstrates implementing `oxgraph-topology` for a tiny oriented hypergraph.
//!
//! The example maps hypergraph vertices to topology elements, hyperedges to
//! topology relations, and each participant to one topology incidence.

use oxgraph_topology::{
    IncidenceBase, IncidenceCounts, IncidenceElement, IncidenceRelation, IncidenceRole,
    RelationIncidences, TopologyBase, TopologyCounts,
};

/// Local vertex handle for the example hypergraph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct VertexId(usize);

/// Local hyperedge handle for the example hypergraph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct HyperedgeId(usize);

/// Local participant-incidence handle for the example hypergraph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ParticipantId(usize);

/// Orientation of a vertex incidence in an oriented hypergraph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IncidenceOrientation {
    /// Negative incidence orientation.
    Negative,
    /// Positive incidence orientation.
    Positive,
}

/// One participant incidence stored by the example hypergraph.
#[derive(Clone, Copy, Debug)]
struct Participant {
    /// Hyperedge containing the participant.
    hyperedge: HyperedgeId,
    /// Vertex participating in the hyperedge.
    vertex: VertexId,
    /// Orientation assigned to this incidence.
    orientation: IncidenceOrientation,
}

/// Small hypergraph used only by this example.
#[derive(Debug)]
struct TinyHypergraph {
    /// Number of vertices in the hypergraph.
    vertex_count: usize,
    /// Number of hyperedges in the hypergraph.
    hyperedge_count: usize,
    /// Contiguous participant incidences.
    participants: &'static [Participant],
}

impl TopologyBase for TinyHypergraph {
    type ElementId = VertexId;
    type RelationId = HyperedgeId;
}

impl IncidenceBase for TinyHypergraph {
    type IncidenceId = ParticipantId;
    type Role = IncidenceOrientation;
}

impl TopologyCounts for TinyHypergraph {
    fn element_count(&self) -> usize {
        self.vertex_count
    }

    fn relation_count(&self) -> usize {
        self.hyperedge_count
    }
}

impl IncidenceCounts for TinyHypergraph {
    fn incidence_count(&self) -> usize {
        self.participants.len()
    }
}

impl RelationIncidences for TinyHypergraph {
    type Incidences<'view>
        = HyperedgeParticipantIter<'view>
    where
        Self: 'view;

    fn relation_incidences(&self, relation: HyperedgeId) -> Self::Incidences<'_> {
        HyperedgeParticipantIter {
            hypergraph: self,
            relation,
            next: 0,
        }
    }
}

impl IncidenceElement for TinyHypergraph {
    fn incidence_element(&self, incidence: ParticipantId) -> VertexId {
        self.participants[incidence.0].vertex
    }
}

impl IncidenceRelation for TinyHypergraph {
    fn incidence_relation(&self, incidence: ParticipantId) -> HyperedgeId {
        self.participants[incidence.0].hyperedge
    }
}

impl IncidenceRole for TinyHypergraph {
    fn incidence_role(&self, incidence: ParticipantId) -> IncidenceOrientation {
        self.participants[incidence.0].orientation
    }
}

/// Iterator over participants attached to one hyperedge.
struct HyperedgeParticipantIter<'view> {
    /// Hypergraph being traversed.
    hypergraph: &'view TinyHypergraph,
    /// Hyperedge whose participants should be yielded.
    relation: HyperedgeId,
    /// Next flat participant index to inspect.
    next: usize,
}

impl Iterator for HyperedgeParticipantIter<'_> {
    type Item = ParticipantId;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.hypergraph.participants.len() {
            let index = self.next;
            self.next += 1;

            if self.hypergraph.participants[index].hyperedge == self.relation {
                return Some(ParticipantId(index));
            }
        }

        None
    }
}

/// Prints participants for one oriented hyperedge `{0-, 1-, 2+}`.
fn main() {
    static PARTICIPANTS: &[Participant] = &[
        Participant {
            hyperedge: HyperedgeId(0),
            vertex: VertexId(0),
            orientation: IncidenceOrientation::Negative,
        },
        Participant {
            hyperedge: HyperedgeId(0),
            vertex: VertexId(1),
            orientation: IncidenceOrientation::Negative,
        },
        Participant {
            hyperedge: HyperedgeId(0),
            vertex: VertexId(2),
            orientation: IncidenceOrientation::Positive,
        },
    ];

    let hypergraph = TinyHypergraph {
        vertex_count: 3,
        hyperedge_count: 1,
        participants: PARTICIPANTS,
    };

    for incidence in hypergraph.relation_incidences(HyperedgeId(0)) {
        println!(
            "hyperedge={:?} incidence={:?} vertex={:?} role={:?}",
            hypergraph.incidence_relation(incidence),
            incidence,
            hypergraph.incidence_element(incidence),
            hypergraph.incidence_role(incidence)
        );
    }
}
