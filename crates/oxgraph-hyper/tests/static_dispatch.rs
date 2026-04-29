//! Tests that `oxgraph-hyper` traits support static-dispatch hypergraph consumers.

use oxgraph_hyper::{
    ContainsHyperedge, ContainsParticipant, ContainsVertex, DirectedHyperedgeParticipants,
    DirectedVertexPredecessors, DirectedVertexSuccessors, HyperedgeIndex,
    HyperedgeParticipantCount, HyperedgeParticipants, Hypergraph, HypergraphCounts,
    IncidentHyperedgeCount, IncidentHyperedges, ParticipantIndex, VertexIndex,
};
use oxgraph_topology::{
    ContainsElement, ContainsIncidence, ContainsRelation, ElementIncidences, ElementIndex,
    IncidenceBase, IncidenceElement, IncidenceIndex, IncidenceRelation, RelationIncidences,
    RelationIndex, TopologyBase, TopologyCounts,
};
use proptest::prelude::*;

/// Vertex identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Vertex(usize);

/// Hyperedge identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Hyperedge(usize);

/// Participant identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Participant(usize);

/// Participant role used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    /// Source-side participant.
    Source,
    /// Target-side participant.
    Target,
}

/// One participant record in the fixture.
#[derive(Clone, Copy, Debug)]
struct ParticipantRecord {
    /// Hyperedge containing the participant.
    hyperedge: Hyperedge,
    /// Vertex participating in the hyperedge.
    vertex: Vertex,
    /// Participant role.
    role: Role,
}

/// Tiny directed hypergraph fixture with precomputed incidence indexes.
#[derive(Debug)]
struct FixtureHypergraph {
    /// Number of vertices in the fixture.
    vertex_count: usize,
    /// Hyperedge-to-participant offsets.
    hyperedge_offsets: &'static [usize],
    /// Flat participant records.
    participants: &'static [ParticipantRecord],
    /// Vertex-to-participant index.
    vertex_index: &'static [&'static [Participant]],
    /// Direct source-to-target vertex expansion preserving multiplicity.
    successors: &'static [&'static [Vertex]],
    /// Direct target-to-source vertex expansion preserving multiplicity.
    predecessors: &'static [&'static [Vertex]],
}

impl TopologyBase for FixtureHypergraph {
    type ElementId = Vertex;
    type RelationId = Hyperedge;
}

impl IncidenceBase for FixtureHypergraph {
    type IncidenceId = Participant;
    type Role = Role;
}

impl TopologyCounts for FixtureHypergraph {
    fn element_count(&self) -> usize {
        self.vertex_count
    }

    fn relation_count(&self) -> usize {
        self.hyperedge_offsets.len() - 1
    }
}

impl HypergraphCounts for FixtureHypergraph {}

impl ElementIndex for FixtureHypergraph {
    fn element_bound(&self) -> usize {
        self.vertex_count
    }

    fn element_index(&self, element: Vertex) -> usize {
        element.0
    }
}

impl ContainsElement for FixtureHypergraph {
    fn contains_element(&self, element: Vertex) -> bool {
        element.0 < self.vertex_count
    }
}

impl RelationIndex for FixtureHypergraph {
    fn relation_bound(&self) -> usize {
        self.hyperedge_offsets.len() - 1
    }

    fn relation_index(&self, relation: Hyperedge) -> usize {
        relation.0
    }
}

impl ContainsRelation for FixtureHypergraph {
    fn contains_relation(&self, relation: Hyperedge) -> bool {
        relation.0 < self.hyperedge_offsets.len() - 1
    }
}

impl IncidenceIndex for FixtureHypergraph {
    fn incidence_bound(&self) -> usize {
        self.participants.len()
    }

    fn incidence_index(&self, incidence: Participant) -> usize {
        incidence.0
    }
}

impl ContainsIncidence for FixtureHypergraph {
    fn contains_incidence(&self, incidence: Participant) -> bool {
        incidence.0 < self.participants.len()
    }
}

impl RelationIncidences for FixtureHypergraph {
    type Incidences<'view>
        = ParticipantIds
    where
        Self: 'view;

    fn relation_incidences(&self, relation: Hyperedge) -> Self::Incidences<'_> {
        let start = self.hyperedge_offsets[relation.0];
        let end = self.hyperedge_offsets[relation.0 + 1];
        ParticipantIds { next: start, end }
    }
}

impl ElementIncidences for FixtureHypergraph {
    type Incidences<'view>
        = core::iter::Copied<core::slice::Iter<'view, Participant>>
    where
        Self: 'view;

    fn element_incidences(&self, element: Vertex) -> Self::Incidences<'_> {
        self.vertex_index[element.0].iter().copied()
    }
}

impl IncidenceElement for FixtureHypergraph {
    fn incidence_element(&self, incidence: Participant) -> Vertex {
        self.participants[incidence.0].vertex
    }
}

impl IncidenceRelation for FixtureHypergraph {
    fn incidence_relation(&self, incidence: Participant) -> Hyperedge {
        self.participants[incidence.0].hyperedge
    }
}

impl HyperedgeParticipants for FixtureHypergraph {
    type Participants<'view>
        = HyperedgeVertices<'view>
    where
        Self: 'view;

    fn hyperedge_participants(&self, hyperedge: Hyperedge) -> Self::Participants<'_> {
        HyperedgeVertices {
            hypergraph: self,
            incidences: self.relation_incidences(hyperedge),
        }
    }
}

impl IncidentHyperedges for FixtureHypergraph {
    type IncidentHyperedges<'view>
        = IncidentEdges<'view>
    where
        Self: 'view;

    fn incident_hyperedges(&self, vertex: Vertex) -> Self::IncidentHyperedges<'_> {
        IncidentEdges {
            hypergraph: self,
            incidences: self.element_incidences(vertex),
        }
    }
}

impl HyperedgeParticipantCount for FixtureHypergraph {
    fn hyperedge_participant_count(&self, hyperedge: Hyperedge) -> usize {
        self.hyperedge_offsets[hyperedge.0 + 1] - self.hyperedge_offsets[hyperedge.0]
    }
}

impl IncidentHyperedgeCount for FixtureHypergraph {
    fn incident_hyperedge_count(&self, vertex: Vertex) -> usize {
        self.vertex_index[vertex.0].len()
    }
}

impl DirectedHyperedgeParticipants for FixtureHypergraph {
    type SourceParticipants<'view>
        = DirectedVertices<'view>
    where
        Self: 'view;

    type TargetParticipants<'view>
        = DirectedVertices<'view>
    where
        Self: 'view;

    fn source_participants(&self, hyperedge: Hyperedge) -> Self::SourceParticipants<'_> {
        DirectedVertices {
            hypergraph: self,
            incidences: self.relation_incidences(hyperedge),
            role: Role::Source,
        }
    }

    fn target_participants(&self, hyperedge: Hyperedge) -> Self::TargetParticipants<'_> {
        DirectedVertices {
            hypergraph: self,
            incidences: self.relation_incidences(hyperedge),
            role: Role::Target,
        }
    }
}

impl DirectedVertexSuccessors for FixtureHypergraph {
    type Successors<'view>
        = core::iter::Copied<core::slice::Iter<'view, Vertex>>
    where
        Self: 'view;

    fn successor_vertices(&self, vertex: Vertex) -> Self::Successors<'_> {
        self.successors[vertex.0].iter().copied()
    }
}

impl DirectedVertexPredecessors for FixtureHypergraph {
    type Predecessors<'view>
        = core::iter::Copied<core::slice::Iter<'view, Vertex>>
    where
        Self: 'view;

    fn predecessor_vertices(&self, vertex: Vertex) -> Self::Predecessors<'_> {
        self.predecessors[vertex.0].iter().copied()
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
    type Item = Participant;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }

        let participant = Participant(self.next);
        self.next += 1;
        Some(participant)
    }
}

/// Iterator mapping participant IDs to vertices.
#[derive(Debug)]
struct HyperedgeVertices<'view> {
    /// Hypergraph being traversed.
    hypergraph: &'view FixtureHypergraph,
    /// Participant IDs still to inspect.
    incidences: ParticipantIds,
}

impl Iterator for HyperedgeVertices<'_> {
    type Item = Vertex;

    fn next(&mut self) -> Option<Self::Item> {
        self.incidences
            .next()
            .map(|incidence| self.hypergraph.incidence_element(incidence))
    }
}

/// Iterator mapping participant IDs with one role to vertices.
#[derive(Debug)]
struct DirectedVertices<'view> {
    /// Hypergraph being traversed.
    hypergraph: &'view FixtureHypergraph,
    /// Participant IDs still to inspect.
    incidences: ParticipantIds,
    /// Role to retain while traversing.
    role: Role,
}

impl Iterator for DirectedVertices<'_> {
    type Item = Vertex;

    fn next(&mut self) -> Option<Self::Item> {
        for participant in self.incidences.by_ref() {
            let record = self.hypergraph.participants[participant.0];
            if record.role == self.role {
                return Some(record.vertex);
            }
        }

        None
    }
}

/// Iterator mapping participant IDs to hyperedges.
#[derive(Debug)]
struct IncidentEdges<'view> {
    /// Hypergraph being traversed.
    hypergraph: &'view FixtureHypergraph,
    /// Participant IDs still to inspect.
    incidences: core::iter::Copied<core::slice::Iter<'view, Participant>>,
}

impl Iterator for IncidentEdges<'_> {
    type Item = Hyperedge;

    fn next(&mut self) -> Option<Self::Item> {
        self.incidences
            .next()
            .map(|incidence| self.hypergraph.incidence_relation(incidence))
    }
}

/// Returns the vertices participating in one hyperedge through static dispatch.
fn participants<H>(hypergraph: &H, hyperedge: H::RelationId) -> Vec<H::ElementId>
where
    H: HyperedgeParticipants,
{
    hypergraph.hyperedge_participants(hyperedge).collect()
}

/// Returns the hyperedges incident to one vertex through static dispatch.
fn incident_edges<H>(hypergraph: &H, vertex: H::ElementId) -> Vec<H::RelationId>
where
    H: IncidentHyperedges,
{
    hypergraph.incident_hyperedges(vertex).collect()
}

/// Returns directed successor vertices through static dispatch.
fn successor_vertices<H>(hypergraph: &H, vertex: H::ElementId) -> Vec<H::ElementId>
where
    H: DirectedVertexSuccessors,
{
    hypergraph.successor_vertices(vertex).collect()
}

/// Returns directed predecessor vertices through static dispatch.
fn predecessor_vertices<H>(hypergraph: &H, vertex: H::ElementId) -> Vec<H::ElementId>
where
    H: DirectedVertexPredecessors,
{
    hypergraph.predecessor_vertices(vertex).collect()
}

/// Returns both traversal directions through the hypergraph bundle.
fn hypergraph_degrees<H>(hypergraph: &H, vertex: H::ElementId, hyperedge: H::RelationId) -> usize
where
    H: Hypergraph,
{
    hypergraph.incident_hyperedges(vertex).count()
        + hypergraph.hyperedge_participants(hyperedge).count()
}

/// Returns a directed hypergraph with multi-source and multi-target hyperedges.
fn fixture() -> FixtureHypergraph {
    static OFFSETS: &[usize] = &[0, 3, 5, 8];
    static PARTICIPANTS: &[ParticipantRecord] = &[
        ParticipantRecord {
            hyperedge: Hyperedge(0),
            vertex: Vertex(0),
            role: Role::Source,
        },
        ParticipantRecord {
            hyperedge: Hyperedge(0),
            vertex: Vertex(1),
            role: Role::Source,
        },
        ParticipantRecord {
            hyperedge: Hyperedge(0),
            vertex: Vertex(2),
            role: Role::Target,
        },
        ParticipantRecord {
            hyperedge: Hyperedge(1),
            vertex: Vertex(2),
            role: Role::Source,
        },
        ParticipantRecord {
            hyperedge: Hyperedge(1),
            vertex: Vertex(3),
            role: Role::Target,
        },
        ParticipantRecord {
            hyperedge: Hyperedge(2),
            vertex: Vertex(0),
            role: Role::Source,
        },
        ParticipantRecord {
            hyperedge: Hyperedge(2),
            vertex: Vertex(2),
            role: Role::Target,
        },
        ParticipantRecord {
            hyperedge: Hyperedge(2),
            vertex: Vertex(3),
            role: Role::Target,
        },
    ];
    static V0: &[Participant] = &[Participant(0), Participant(5)];
    static V1: &[Participant] = &[Participant(1)];
    static V2: &[Participant] = &[Participant(2), Participant(3), Participant(6)];
    static V3: &[Participant] = &[Participant(4), Participant(7)];
    static VERTEX_INDEX: &[&[Participant]] = &[V0, V1, V2, V3];
    static SUCC_0: &[Vertex] = &[Vertex(2), Vertex(2), Vertex(3)];
    static SUCC_1: &[Vertex] = &[Vertex(2)];
    static SUCC_2: &[Vertex] = &[Vertex(3)];
    static SUCC_3: &[Vertex] = &[];
    static SUCCESSORS: &[&[Vertex]] = &[SUCC_0, SUCC_1, SUCC_2, SUCC_3];
    static PRED_0: &[Vertex] = &[];
    static PRED_1: &[Vertex] = &[];
    static PRED_2: &[Vertex] = &[Vertex(0), Vertex(1), Vertex(0)];
    static PRED_3: &[Vertex] = &[Vertex(2), Vertex(0)];
    static PREDECESSORS: &[&[Vertex]] = &[PRED_0, PRED_1, PRED_2, PRED_3];

    FixtureHypergraph {
        vertex_count: 4,
        hyperedge_offsets: OFFSETS,
        participants: PARTICIPANTS,
        vertex_index: VERTEX_INDEX,
        successors: SUCCESSORS,
        predecessors: PREDECESSORS,
    }
}

#[test]
fn generic_consumer_reads_hyperedge_participants() {
    let hypergraph = fixture();

    assert_eq!(
        participants(&hypergraph, Hyperedge(0)),
        [Vertex(0), Vertex(1), Vertex(2)]
    );
    assert_eq!(
        participants(&hypergraph, Hyperedge(1)),
        [Vertex(2), Vertex(3)]
    );
}

#[test]
fn generic_consumer_reads_incident_hyperedges() {
    let hypergraph = fixture();

    assert_eq!(
        incident_edges(&hypergraph, Vertex(2)),
        [Hyperedge(0), Hyperedge(1), Hyperedge(2)]
    );
    assert_eq!(
        incident_edges(&hypergraph, Vertex(3)),
        [Hyperedge(1), Hyperedge(2)]
    );
}

#[test]
fn counts_describe_visible_hypergraph() {
    let hypergraph = fixture();

    assert_eq!(hypergraph.vertex_count(), 4);
    assert_eq!(hypergraph.hyperedge_count(), 3);
}

#[test]
fn hypergraph_index_aliases_delegate_to_topology_indexes() {
    let hypergraph = fixture();

    assert_eq!(hypergraph.vertex_bound(), hypergraph.element_bound());
    assert_eq!(
        hypergraph.vertex_index(Vertex(2)),
        hypergraph.element_index(Vertex(2))
    );
    assert_eq!(hypergraph.hyperedge_bound(), hypergraph.relation_bound());
    assert_eq!(
        hypergraph.hyperedge_index(Hyperedge(1)),
        hypergraph.relation_index(Hyperedge(1))
    );
    assert_eq!(hypergraph.participant_bound(), hypergraph.incidence_bound());
    assert_eq!(
        hypergraph.participant_index(Participant(4)),
        hypergraph.incidence_index(Participant(4))
    );
}

#[test]
fn participant_roles_are_preserved_by_topology_view() {
    let hypergraph = fixture();

    assert_eq!(hypergraph.participants[Participant(4).0].role, Role::Target);
}

#[test]
fn hypergraph_containment_aliases_delegate_to_topology_containment() {
    let hypergraph = fixture();

    assert_eq!(
        hypergraph.contains_vertex(Vertex(2)),
        hypergraph.contains_element(Vertex(2))
    );
    assert!(!hypergraph.contains_vertex(Vertex(4)));
    assert_eq!(
        hypergraph.contains_hyperedge(Hyperedge(1)),
        hypergraph.contains_relation(Hyperedge(1))
    );
    assert!(!hypergraph.contains_hyperedge(Hyperedge(3)));
    assert_eq!(
        hypergraph.contains_participant(Participant(4)),
        hypergraph.contains_incidence(Participant(4))
    );
    assert!(!hypergraph.contains_participant(Participant(8)));
}

#[test]
fn hypergraph_bundle_blanket_impl_is_available() {
    let hypergraph = fixture();

    assert_eq!(hypergraph_degrees(&hypergraph, Vertex(2), Hyperedge(0)), 6);
}

#[test]
fn directed_hyperedge_participants_split_roles() {
    let hypergraph = fixture();

    assert_eq!(
        hypergraph
            .source_participants(Hyperedge(0))
            .collect::<Vec<_>>(),
        [Vertex(0), Vertex(1)]
    );
    assert_eq!(
        hypergraph
            .target_participants(Hyperedge(0))
            .collect::<Vec<_>>(),
        [Vertex(2)]
    );
}

#[test]
fn directed_vertex_successors_preserve_multiplicity() {
    let hypergraph = fixture();

    assert_eq!(
        successor_vertices(&hypergraph, Vertex(0)),
        [Vertex(2), Vertex(2), Vertex(3)]
    );
    assert_eq!(successor_vertices(&hypergraph, Vertex(3)), []);
}

#[test]
fn directed_vertex_predecessors_preserve_multiplicity() {
    let hypergraph = fixture();

    assert_eq!(
        predecessor_vertices(&hypergraph, Vertex(2)),
        [Vertex(0), Vertex(1), Vertex(0)]
    );
    assert_eq!(predecessor_vertices(&hypergraph, Vertex(0)), []);
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Hyperedge participant count matches traversal count for every fixture hyperedge.
    #[test]
    fn participant_count_matches_iterator_count(index in 0usize..3) {
        let hypergraph = fixture();
        let hyperedge = Hyperedge(index);

        prop_assert_eq!(
            hypergraph.hyperedge_participant_count(hyperedge),
            hypergraph.hyperedge_participants(hyperedge).count()
        );
    }

    /// Incident hyperedge count matches traversal count for every fixture vertex.
    #[test]
    fn incident_count_matches_iterator_count(index in 0usize..4) {
        let hypergraph = fixture();
        let vertex = Vertex(index);

        prop_assert_eq!(
            hypergraph.incident_hyperedge_count(vertex),
            hypergraph.incident_hyperedges(vertex).count()
        );
    }
}
