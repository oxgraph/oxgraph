//! Demonstrates implementing `oxgraph-hyper` for a tiny directed hypergraph.

use oxgraph_hyper::{
    DirectedHyperedgeParticipants, HyperedgeParticipantCount, HyperedgeParticipants,
    HypergraphCounts, IncidentHyperedgeCount, IncidentHyperedges,
};
use oxgraph_topology::{
    ElementIncidences, IncidenceBase, IncidenceElement, IncidenceRelation, RelationIncidences,
    TopologyBase, TopologyCounts,
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

/// Direction-side role for a participant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParticipantRole {
    /// Source-side participant.
    Source,
    /// Target-side participant.
    Target,
}

/// One participant incidence stored by the example hypergraph.
#[derive(Clone, Copy, Debug)]
struct Participant {
    /// Hyperedge containing the participant.
    hyperedge: HyperedgeId,
    /// Vertex participating in the hyperedge.
    vertex: VertexId,
    /// Direction-side role for this participant.
    role: ParticipantRole,
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
    /// Hyperedge-to-participant offsets.
    hyperedge_offsets: &'static [usize],
    /// Vertex-to-participant index.
    vertex_incidence_index: &'static [&'static [ParticipantId]],
}

impl TopologyBase for TinyHypergraph {
    type ElementId = VertexId;
    type RelationId = HyperedgeId;
}

impl IncidenceBase for TinyHypergraph {
    type IncidenceId = ParticipantId;
    type Role = ParticipantRole;
}

impl TopologyCounts for TinyHypergraph {
    fn element_count(&self) -> usize {
        self.vertex_count
    }

    fn relation_count(&self) -> usize {
        self.hyperedge_count
    }
}

impl HypergraphCounts for TinyHypergraph {}

impl RelationIncidences for TinyHypergraph {
    type Incidences<'view>
        = ParticipantIds
    where
        Self: 'view;

    fn relation_incidences(&self, relation: HyperedgeId) -> Self::Incidences<'_> {
        let start = self.hyperedge_offsets[relation.0];
        let end = self.hyperedge_offsets[relation.0 + 1];
        ParticipantIds { next: start, end }
    }
}

impl ElementIncidences for TinyHypergraph {
    type Incidences<'view>
        = core::iter::Copied<core::slice::Iter<'view, ParticipantId>>
    where
        Self: 'view;

    fn element_incidences(&self, element: VertexId) -> Self::Incidences<'_> {
        self.vertex_incidence_index[element.0].iter().copied()
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

impl HyperedgeParticipants for TinyHypergraph {
    type Participants<'view>
        = HyperedgeVertices<'view>
    where
        Self: 'view;

    fn hyperedge_participants(&self, hyperedge: HyperedgeId) -> Self::Participants<'_> {
        HyperedgeVertices {
            hypergraph: self,
            incidences: self.relation_incidences(hyperedge),
        }
    }
}

impl IncidentHyperedges for TinyHypergraph {
    type IncidentHyperedges<'view>
        = IncidentEdges<'view>
    where
        Self: 'view;

    fn incident_hyperedges(&self, vertex: VertexId) -> Self::IncidentHyperedges<'_> {
        IncidentEdges {
            hypergraph: self,
            incidences: self.element_incidences(vertex),
        }
    }
}

impl HyperedgeParticipantCount for TinyHypergraph {
    fn hyperedge_participant_count(&self, hyperedge: HyperedgeId) -> usize {
        self.hyperedge_offsets[hyperedge.0 + 1] - self.hyperedge_offsets[hyperedge.0]
    }
}

impl IncidentHyperedgeCount for TinyHypergraph {
    fn incident_hyperedge_count(&self, vertex: VertexId) -> usize {
        self.vertex_incidence_index[vertex.0].len()
    }
}

impl DirectedHyperedgeParticipants for TinyHypergraph {
    type SourceParticipants<'view>
        = RoleVertices<'view>
    where
        Self: 'view;

    type TargetParticipants<'view>
        = RoleVertices<'view>
    where
        Self: 'view;

    fn source_participants(&self, hyperedge: HyperedgeId) -> Self::SourceParticipants<'_> {
        RoleVertices {
            hypergraph: self,
            incidences: self.relation_incidences(hyperedge),
            role: ParticipantRole::Source,
        }
    }

    fn target_participants(&self, hyperedge: HyperedgeId) -> Self::TargetParticipants<'_> {
        RoleVertices {
            hypergraph: self,
            incidences: self.relation_incidences(hyperedge),
            role: ParticipantRole::Target,
        }
    }
}

/// Iterator over contiguous participant IDs.
#[derive(Debug)]
struct ParticipantIds {
    /// Next flat participant index to yield.
    next: usize,
    /// Exclusive end of the participant range.
    end: usize,
}

impl Iterator for ParticipantIds {
    type Item = ParticipantId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }

        let participant = ParticipantId(self.next);
        self.next += 1;
        Some(participant)
    }
}

/// Iterator mapping participant IDs to vertices.
#[derive(Debug)]
struct HyperedgeVertices<'view> {
    /// Hypergraph being traversed.
    hypergraph: &'view TinyHypergraph,
    /// Participant IDs still to inspect.
    incidences: ParticipantIds,
}

impl Iterator for HyperedgeVertices<'_> {
    type Item = VertexId;

    fn next(&mut self) -> Option<Self::Item> {
        self.incidences
            .next()
            .map(|incidence| self.hypergraph.incidence_element(incidence))
    }
}

/// Iterator mapping participant IDs to hyperedges.
#[derive(Debug)]
struct IncidentEdges<'view> {
    /// Hypergraph being traversed.
    hypergraph: &'view TinyHypergraph,
    /// Participant IDs still to inspect.
    incidences: core::iter::Copied<core::slice::Iter<'view, ParticipantId>>,
}

impl Iterator for IncidentEdges<'_> {
    type Item = HyperedgeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.incidences
            .next()
            .map(|incidence| self.hypergraph.incidence_relation(incidence))
    }
}

/// Iterator over vertices with one selected participant role.
#[derive(Debug)]
struct RoleVertices<'view> {
    /// Hypergraph being traversed.
    hypergraph: &'view TinyHypergraph,
    /// Participant IDs still to inspect.
    incidences: ParticipantIds,
    /// Role to include.
    role: ParticipantRole,
}

impl Iterator for RoleVertices<'_> {
    type Item = VertexId;

    fn next(&mut self) -> Option<Self::Item> {
        for incidence in self.incidences.by_ref() {
            let participant = self.hypergraph.participants[incidence.0];
            if participant.role == self.role {
                return Some(participant.vertex);
            }
        }
        None
    }
}

/// Prints participant sets for one directed hyperedge `{0, 1} -> {2}`.
fn main() {
    static PARTICIPANTS: &[Participant] = &[
        Participant {
            hyperedge: HyperedgeId(0),
            vertex: VertexId(0),
            role: ParticipantRole::Source,
        },
        Participant {
            hyperedge: HyperedgeId(0),
            vertex: VertexId(1),
            role: ParticipantRole::Source,
        },
        Participant {
            hyperedge: HyperedgeId(0),
            vertex: VertexId(2),
            role: ParticipantRole::Target,
        },
    ];
    static OFFSETS: &[usize] = &[0, 3];
    static V0: &[ParticipantId] = &[ParticipantId(0)];
    static V1: &[ParticipantId] = &[ParticipantId(1)];
    static V2: &[ParticipantId] = &[ParticipantId(2)];
    static VERTEX_INDEX: &[&[ParticipantId]] = &[V0, V1, V2];

    let hypergraph = TinyHypergraph {
        vertex_count: 3,
        hyperedge_count: 1,
        participants: PARTICIPANTS,
        hyperedge_offsets: OFFSETS,
        vertex_incidence_index: VERTEX_INDEX,
    };

    println!("vertices={}", hypergraph.vertex_count());
    println!("hyperedges={}", hypergraph.hyperedge_count());
    println!(
        "participants={:?}",
        hypergraph
            .hyperedge_participants(HyperedgeId(0))
            .collect::<Vec<_>>()
    );
    println!(
        "sources={:?}",
        hypergraph
            .source_participants(HyperedgeId(0))
            .collect::<Vec<_>>()
    );
    println!(
        "targets={:?}",
        hypergraph
            .target_participants(HyperedgeId(0))
            .collect::<Vec<_>>()
    );
}
