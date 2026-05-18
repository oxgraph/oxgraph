//! Storage-agnostic traits for hypergraph views.
//!
//! `oxgraph-hyper` is the hypergraph specialization above `oxgraph-topology`. Use it
//! to write generic hypergraph consumers over vertex, hyperedge, participant,
//! incident-hyperedge, and directed participant-set vocabulary.
//!
//! Hypergraph views use topology elements as vertices, topology relations as
//! hyperedges, and topology incidences as participant records. Concrete layouts,
//! snapshots, builders, mutation systems, payloads, and algorithms belong in
//! higher-level crates.
//!
//! # Performance contract
//!
//! Most of the traits and aliases in this crate are hypergraph-vocabulary
//! shadows of topology traits. Unless a specific item documents otherwise,
//! every method, iterator, and alias inherits its performance contract from
//! the underlying topology trait it shadows: `O(1)` for accessors, `O(1)` to
//! construct an iterator plus `O(k)` to yield `k` items, and `perf:
//! unspecified` for capability-bundle marker traits and blanket impls.
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

/// Hypergraph-facing alias for a topology element ID (vertex).
pub type VertexId<H> = <H as TopologyBase>::ElementId;

/// Hypergraph-facing alias for a topology relation ID (hyperedge).
pub type HyperedgeId<H> = <H as TopologyBase>::RelationId;

/// Hypergraph-facing alias for a topology incidence ID (participant record).
pub type ParticipantId<H> = <H as IncidenceBase>::IncidenceId;

/// Hypergraph-facing alias for a topology incidence role used to distinguish
/// source, target, input, output, or implementation-defined participant
/// categories on directed or role-aware hypergraph views.
pub type ParticipantRole<H> = <H as IncidenceBase>::Role;

/// Base capability for hypergraph views over topology storage.
///
/// Hypergraph-facing name for [`TopologyBase`].
pub trait HypergraphBase: TopologyBase {}

impl<T> HypergraphBase for T where T: TopologyBase {}

/// Base capability for hypergraph views that expose participant records.
///
/// Hypergraph-facing name for [`IncidenceBase`].
pub trait ParticipantBase: IncidenceBase {}

impl<T> ParticipantBase for T where T: IncidenceBase {}

/// Count capability for a hypergraph view.
///
/// Hypergraph-facing name for [`TopologyCounts`].
pub trait HypergraphCounts: TopologyCounts {
    /// Returns the number of vertices visible in this hypergraph view.
    fn vertex_count(&self) -> usize {
        self.element_count()
    }

    /// Returns the number of hyperedges visible in this hypergraph view.
    fn hyperedge_count(&self) -> usize {
        self.relation_count()
    }
}

/// Participant-record count capability for hypergraph views.
///
/// Hypergraph-facing name for [`IncidenceCounts`].
pub trait ParticipantCounts: IncidenceCounts {
    /// Returns the total number of participant records visible in this view.
    fn participant_count(&self) -> usize {
        self.incidence_count()
    }
}

impl<T> ParticipantCounts for T where T: IncidenceCounts {}

/// Dense vertex-index capability for hypergraph views.
///
/// Hypergraph-facing name for [`ElementIndex`].
pub trait VertexIndex: ElementIndex {
    /// Returns the exclusive upper bound for vertex indexes in this view.
    fn vertex_bound(&self) -> usize {
        self.element_bound()
    }

    /// Returns the dense index for `vertex` in this view.
    fn vertex_index(&self, vertex: Self::ElementId) -> usize {
        self.element_index(vertex)
    }
}

impl<T> VertexIndex for T where T: ElementIndex {}

/// Dense hyperedge-index capability for hypergraph views.
///
/// Hypergraph-facing name for [`RelationIndex`].
pub trait HyperedgeIndex: RelationIndex {
    /// Returns the exclusive upper bound for hyperedge indexes in this view.
    fn hyperedge_bound(&self) -> usize {
        self.relation_bound()
    }

    /// Returns the dense index for `hyperedge` in this view.
    fn hyperedge_index(&self, hyperedge: Self::RelationId) -> usize {
        self.relation_index(hyperedge)
    }
}

impl<T> HyperedgeIndex for T where T: RelationIndex {}

/// Dense participant-index capability for hypergraph views with incidences.
///
/// Hypergraph-facing name for [`IncidenceIndex`].
pub trait ParticipantIndex: IncidenceIndex {
    /// Returns the exclusive upper bound for participant indexes in this view.
    fn participant_bound(&self) -> usize {
        self.incidence_bound()
    }

    /// Returns the dense index for `participant` in this view.
    fn participant_index(&self, participant: Self::IncidenceId) -> usize {
        self.incidence_index(participant)
    }
}

impl<T> ParticipantIndex for T where T: IncidenceIndex {}

/// Vertex-ID containment capability for hypergraph views.
///
/// Hypergraph-facing name for [`ContainsElement`].
pub trait ContainsVertex: ContainsElement {
    /// Returns whether `vertex` is valid and visible in this hypergraph view.
    fn contains_vertex(&self, vertex: Self::ElementId) -> bool {
        self.contains_element(vertex)
    }
}

impl<T> ContainsVertex for T where T: ContainsElement {}

/// Hyperedge-ID containment capability for hypergraph views.
///
/// Hypergraph-facing name for [`ContainsRelation`].
pub trait ContainsHyperedge: ContainsRelation {
    /// Returns whether `hyperedge` is valid and visible in this hypergraph view.
    fn contains_hyperedge(&self, hyperedge: Self::RelationId) -> bool {
        self.contains_relation(hyperedge)
    }
}

impl<T> ContainsHyperedge for T where T: ContainsRelation {}

/// Participant-ID containment capability for hypergraph views with incidences.
///
/// Hypergraph-facing name for [`ContainsIncidence`].
pub trait ContainsParticipant: ContainsIncidence {
    /// Returns whether `participant` is valid and visible in this hypergraph view.
    fn contains_participant(&self, participant: Self::IncidenceId) -> bool {
        self.contains_incidence(participant)
    }
}

impl<T> ContainsParticipant for T where T: ContainsIncidence {}

/// Capability for traversing participant records attached to one hyperedge.
///
/// Hypergraph-facing name for [`RelationIncidences`]; yields raw participant
/// IDs rather than resolved vertices. Pair with [`HyperedgeParticipants`]
/// when callers want vertices directly.
pub trait HyperedgeIncidences: RelationIncidences {
    /// Iterator over participant IDs attached to one hyperedge.
    type ParticipantIds<'view>: Iterator<Item = ParticipantId<Self>>
    where
        Self: 'view;

    /// Returns participant IDs attached to `hyperedge`.
    fn hyperedge_incidences(&self, hyperedge: HyperedgeId<Self>) -> Self::ParticipantIds<'_>;
}

impl<T> HyperedgeIncidences for T
where
    T: RelationIncidences,
{
    type ParticipantIds<'view>
        = <T as RelationIncidences>::Incidences<'view>
    where
        T: 'view;

    fn hyperedge_incidences(&self, hyperedge: HyperedgeId<Self>) -> Self::ParticipantIds<'_> {
        <Self as RelationIncidences>::relation_incidences(self, hyperedge)
    }
}

/// Capability for traversing participant records attached to one vertex.
///
/// Hypergraph-facing name for [`ElementIncidences`]; yields raw participant
/// IDs rather than resolved hyperedges. Pair with [`IncidentHyperedges`]
/// when callers want hyperedges directly.
pub trait VertexIncidences: ElementIncidences {
    /// Iterator over participant IDs attached to one vertex.
    type ParticipantIds<'view>: Iterator<Item = ParticipantId<Self>>
    where
        Self: 'view;

    /// Returns participant IDs attached to `vertex`.
    fn vertex_incidences(&self, vertex: VertexId<Self>) -> Self::ParticipantIds<'_>;
}

impl<T> VertexIncidences for T
where
    T: ElementIncidences,
{
    type ParticipantIds<'view>
        = <T as ElementIncidences>::Incidences<'view>
    where
        T: 'view;

    fn vertex_incidences(&self, vertex: VertexId<Self>) -> Self::ParticipantIds<'_> {
        <Self as ElementIncidences>::element_incidences(self, vertex)
    }
}

/// Capability for resolving the vertex carried by a participant record.
///
/// Hypergraph-facing name for [`IncidenceElement`].
pub trait ParticipantVertex: IncidenceElement {
    /// Returns the vertex referenced by `participant`.
    fn participant_vertex(&self, participant: ParticipantId<Self>) -> VertexId<Self> {
        self.incidence_element(participant)
    }
}

impl<T> ParticipantVertex for T where T: IncidenceElement {}

/// Capability for resolving the hyperedge that carries a participant record.
///
/// Hypergraph-facing name for [`IncidenceRelation`].
pub trait ParticipantHyperedge: IncidenceRelation {
    /// Returns the hyperedge carrying `participant`.
    fn participant_hyperedge(&self, participant: ParticipantId<Self>) -> HyperedgeId<Self> {
        self.incidence_relation(participant)
    }
}

impl<T> ParticipantHyperedge for T where T: IncidenceRelation {}

/// Capability for resolving the role recorded for a participant.
///
/// Hypergraph-facing name for [`IncidenceRole`]. Trait name
/// `ParticipantRoleOf` avoids colliding with the existing
/// [`ParticipantRole`] type alias.
pub trait ParticipantRoleOf: IncidenceRole {
    /// Returns the role recorded for `participant`.
    fn participant_role_of(&self, participant: ParticipantId<Self>) -> ParticipantRole<Self> {
        self.incidence_role(participant)
    }
}

impl<T> ParticipantRoleOf for T where T: IncidenceRole {}

/// Capability for traversing vertices participating in one hyperedge.
///
/// Hypergraph-facing form of relation-to-element traversal.
pub trait HyperedgeParticipants: HyperedgeIncidences + ParticipantVertex {
    /// Iterator over vertices participating in one hyperedge.
    type Participants<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns vertices participating in `hyperedge`.
    fn hyperedge_participants(&self, hyperedge: Self::RelationId) -> Self::Participants<'_>;
}

/// Capability for traversing hyperedges incident to one vertex.
///
/// Hypergraph-facing form of element-to-relation traversal.
pub trait IncidentHyperedges: VertexIncidences + ParticipantHyperedge {
    /// Iterator over hyperedges incident to one vertex.
    type IncidentHyperedges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Returns hyperedges incident to `vertex`.
    fn incident_hyperedges(&self, vertex: Self::ElementId) -> Self::IncidentHyperedges<'_>;
}

/// Exact hyperedge-participant count capability.
///
/// Pairs with [`HyperedgeParticipants`] for backends that can report a
/// hyperedge's participant count without traversal.
pub trait HyperedgeParticipantCount: IncidenceBase {
    /// Returns the number of participants attached to `hyperedge`.
    fn hyperedge_participant_count(&self, hyperedge: Self::RelationId) -> usize;
}

/// Exact incident-hyperedge count capability.
///
/// Pairs with [`IncidentHyperedges`] for backends that can report a vertex's
/// incident hyperedge count without traversal.
pub trait IncidentHyperedgeCount: IncidenceBase {
    /// Returns the number of hyperedges incident to `vertex`.
    fn incident_hyperedge_count(&self, vertex: Self::ElementId) -> usize;
}

/// Capability for traversing directed hyperedge participant sets.
///
/// Directed hypergraphs distinguish source-side and target-side participants.
/// The core does not prescribe how a view stores those sets or whether they
/// are derived from roles, separate indexes, or generated views.
pub trait DirectedHyperedgeParticipants: TopologyBase {
    /// Iterator over source-side participants in one directed hyperedge.
    type SourceParticipants<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Iterator over target-side participants in one directed hyperedge.
    type TargetParticipants<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns source-side participants for `hyperedge`.
    fn source_participants(&self, hyperedge: Self::RelationId) -> Self::SourceParticipants<'_>;

    /// Returns target-side participants for `hyperedge`.
    fn target_participants(&self, hyperedge: Self::RelationId) -> Self::TargetParticipants<'_>;
}

/// Capability for traversing directed participant incidence IDs.
///
/// Incidence-ID counterpart to [`DirectedHyperedgeParticipants`]. Algorithms
/// that need incidence weights bind on this trait so they can choose target
/// participants by incidence ID without adding weighted expansion traits to
/// topology.
pub trait DirectedHyperedgeIncidences: IncidenceBase {
    /// Iterator over source-side participant incidence IDs.
    type SourceIncidences<'view>: Iterator<Item = Self::IncidenceId>
    where
        Self: 'view;

    /// Iterator over target-side participant incidence IDs.
    type TargetIncidences<'view>: Iterator<Item = Self::IncidenceId>
    where
        Self: 'view;

    /// Returns source-side participant incidence IDs for `hyperedge`.
    fn source_incidences(&self, hyperedge: Self::RelationId) -> Self::SourceIncidences<'_>;

    /// Returns target-side participant incidence IDs for `hyperedge`.
    fn target_incidences(&self, hyperedge: Self::RelationId) -> Self::TargetIncidences<'_>;
}

/// Capability for traversing source/target hyperedges incident to a vertex.
///
/// Relation-level directed expansion, distinct from successor-vertex
/// expansion. Directed traversal consumers can use source-to-relation
/// transitions before relation-to-target expansion.
pub trait DirectedVertexHyperedges: TopologyBase {
    /// Iterator over hyperedges where the vertex is source-side.
    type OutgoingHyperedges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Iterator over hyperedges where the vertex is target-side.
    type IncomingHyperedges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Returns hyperedges where `vertex` participates on the source side.
    fn outgoing_hyperedges(&self, vertex: Self::ElementId) -> Self::OutgoingHyperedges<'_>;

    /// Returns hyperedges where `vertex` participates on the target side.
    fn incoming_hyperedges(&self, vertex: Self::ElementId) -> Self::IncomingHyperedges<'_>;
}

/// Capability for expanding a directed hypergraph vertex to successor vertices.
///
/// Hypergraph-facing name for [`ElementSuccessors`]. A successor vertex is a
/// target-side participant reachable through a directed hyperedge where the
/// input vertex participates on the source side. The associated iterator GAT
/// is named `VertexSuccessors` to avoid colliding with the inherited
/// `ElementSuccessors::Successors` GAT.
pub trait DirectedVertexSuccessors: ElementSuccessors {
    /// Iterator over successor vertices reachable from one source-side vertex.
    type VertexSuccessors<'view>: Iterator<Item = VertexId<Self>>
    where
        Self: 'view;

    /// Returns target-side vertices reachable from `vertex`.
    fn successor_vertices(&self, vertex: VertexId<Self>) -> Self::VertexSuccessors<'_>;
}

impl<T> DirectedVertexSuccessors for T
where
    T: ElementSuccessors,
{
    type VertexSuccessors<'view>
        = <T as ElementSuccessors>::Successors<'view>
    where
        T: 'view;

    fn successor_vertices(&self, vertex: VertexId<Self>) -> Self::VertexSuccessors<'_> {
        <Self as ElementSuccessors>::element_successors(self, vertex)
    }
}

/// Capability for expanding a directed hypergraph vertex to predecessor vertices.
///
/// Hypergraph-facing name for [`ElementPredecessors`]. A predecessor vertex
/// is a source-side participant reachable through a directed hyperedge where
/// the input vertex participates on the target side. The associated iterator
/// GAT is named `VertexPredecessors` to avoid colliding with the inherited
/// `ElementPredecessors::Predecessors` GAT.
pub trait DirectedVertexPredecessors: ElementPredecessors {
    /// Iterator over predecessor vertices reaching one target-side vertex.
    type VertexPredecessors<'view>: Iterator<Item = VertexId<Self>>
    where
        Self: 'view;

    /// Returns source-side vertices that can reach `vertex`.
    fn predecessor_vertices(&self, vertex: VertexId<Self>) -> Self::VertexPredecessors<'_>;
}

impl<T> DirectedVertexPredecessors for T
where
    T: ElementPredecessors,
{
    type VertexPredecessors<'view>
        = <T as ElementPredecessors>::Predecessors<'view>
    where
        T: 'view;

    fn predecessor_vertices(&self, vertex: VertexId<Self>) -> Self::VertexPredecessors<'_> {
        <Self as ElementPredecessors>::element_predecessors(self, vertex)
    }
}

/// Convenience marker bundling hyperedge participants and incident hyperedges.
pub trait Hypergraph: HyperedgeParticipants + IncidentHyperedges {}

impl<T> Hypergraph for T where T: HyperedgeParticipants + IncidentHyperedges {}

/// Convenience marker for complete directed hypergraph traversal.
///
/// Bundles hyperedge-side traversal ([`Hypergraph`]), source/target
/// participant separation ([`DirectedHyperedgeParticipants`]), and
/// bidirectional vertex-to-vertex traversal ([`DirectedVertexSuccessors`] +
/// [`DirectedVertexPredecessors`]).
pub trait DirectedHypergraph:
    Hypergraph
    + DirectedHyperedgeParticipants
    + DirectedHyperedgeIncidences
    + DirectedVertexHyperedges
    + DirectedVertexSuccessors
    + DirectedVertexPredecessors
{
}

impl<T> DirectedHypergraph for T where
    T: Hypergraph
        + DirectedHyperedgeParticipants
        + DirectedHyperedgeIncidences
        + DirectedVertexHyperedges
        + DirectedVertexSuccessors
        + DirectedVertexPredecessors
{
}
