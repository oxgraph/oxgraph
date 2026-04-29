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
#![no_std]

#[cfg(kani)]
extern crate kani;

use oxgraph_topology::{
    ContainsElement, ContainsIncidence, ContainsRelation, ElementIncidences, ElementIndex,
    IncidenceBase, IncidenceElement, IncidenceIndex, IncidenceRelation, RelationIncidences,
    RelationIndex, TopologyBase, TopologyCounts,
};

/// Hypergraph-facing alias for a topology element ID.
///
/// Hypergraph views use topology elements as vertices. This alias gives
/// hypergraph-facing code vertex vocabulary without introducing a second
/// identity layer.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`TopologyBase::ElementId`] type.
pub type VertexId<H> = <H as TopologyBase>::ElementId;

/// Hypergraph-facing alias for a topology relation ID.
///
/// Hypergraph views use topology relations as hyperedges. This alias gives
/// hypergraph-facing code hyperedge vocabulary without introducing a second
/// identity layer.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`TopologyBase::RelationId`] type.
pub type HyperedgeId<H> = <H as TopologyBase>::RelationId;

/// Hypergraph-facing alias for a topology incidence ID.
///
/// Each participant occurrence in a hyperedge is represented as one topology
/// incidence when a hypergraph view exposes participant records.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`IncidenceBase::IncidenceId`] type.
pub type ParticipantId<H> = <H as IncidenceBase>::IncidenceId;

/// Hypergraph-facing alias for a topology incidence role.
///
/// Directed or role-aware hypergraph views can use roles to distinguish source,
/// target, input, output, or implementation-defined participant categories.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the underlying
/// [`IncidenceBase::Role`] type.
pub type ParticipantRole<H> = <H as IncidenceBase>::Role;

/// Count capability for a hypergraph view.
///
/// This trait gives hypergraph-facing names to [`TopologyCounts`] values for
/// views that represent hypergraphs.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait HypergraphCounts: TopologyCounts {
    /// Returns the number of vertices visible in this hypergraph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn vertex_count(&self) -> usize {
        self.element_count()
    }

    /// Returns the number of hyperedges visible in this hypergraph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn hyperedge_count(&self) -> usize {
        self.relation_count()
    }
}

/// Dense vertex-index capability for hypergraph views.
///
/// This is the hypergraph-facing name for [`ElementIndex`]. It lets hypergraph
/// algorithms allocate per-vertex scratch storage without requiring users to
/// think in topology vocabulary.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait VertexIndex: ElementIndex {
    /// Returns the exclusive upper bound for vertex indexes in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn vertex_bound(&self) -> usize {
        self.element_bound()
    }

    /// Returns the dense index for `vertex` in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn vertex_index(&self, vertex: Self::ElementId) -> usize {
        self.element_index(vertex)
    }
}

/// Blanket implementation for hypergraph views with dense element indexes.
///
/// Any hypergraph view that implements [`ElementIndex`] automatically exposes
/// the hypergraph-facing [`VertexIndex`] vocabulary.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ElementIndex`].
impl<T> VertexIndex for T where T: ElementIndex {}

/// Dense hyperedge-index capability for hypergraph views.
///
/// This is the hypergraph-facing name for [`RelationIndex`]. It lets hypergraph
/// algorithms allocate per-hyperedge scratch storage without requiring users to
/// think in topology vocabulary.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait HyperedgeIndex: RelationIndex {
    /// Returns the exclusive upper bound for hyperedge indexes in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn hyperedge_bound(&self) -> usize {
        self.relation_bound()
    }

    /// Returns the dense index for `hyperedge` in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn hyperedge_index(&self, hyperedge: Self::RelationId) -> usize {
        self.relation_index(hyperedge)
    }
}

/// Blanket implementation for hypergraph views with dense relation indexes.
///
/// Any hypergraph view that implements [`RelationIndex`] automatically exposes
/// the hypergraph-facing [`HyperedgeIndex`] vocabulary.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`RelationIndex`].
impl<T> HyperedgeIndex for T where T: RelationIndex {}

/// Dense participant-index capability for hypergraph views with incidences.
///
/// This is the hypergraph-facing name for [`IncidenceIndex`]. It lets hypergraph
/// algorithms allocate per-participant scratch storage for views that expose
/// participants as topology incidences.
///
/// # Performance
///
/// Methods should be `O(1)` unless an implementation documents a weaker
/// contract.
pub trait ParticipantIndex: IncidenceIndex {
    /// Returns the exclusive upper bound for participant indexes in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn participant_bound(&self) -> usize {
        self.incidence_bound()
    }

    /// Returns the dense index for `participant` in this view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn participant_index(&self, participant: Self::IncidenceId) -> usize {
        self.incidence_index(participant)
    }
}

/// Blanket implementation for hypergraph views with dense incidence indexes.
///
/// Any hypergraph view that implements [`IncidenceIndex`] automatically exposes
/// the hypergraph-facing [`ParticipantIndex`] vocabulary.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`IncidenceIndex`].
impl<T> ParticipantIndex for T where T: IncidenceIndex {}

/// Vertex-ID containment capability for hypergraph views.
///
/// This is the hypergraph-facing name for [`ContainsElement`]. It answers
/// whether a vertex ID is valid and visible in this hypergraph view.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsVertex: ContainsElement {
    /// Returns whether `vertex` is valid and visible in this hypergraph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_vertex(&self, vertex: Self::ElementId) -> bool {
        self.contains_element(vertex)
    }
}

/// Blanket implementation for hypergraph views with element containment.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ContainsElement`].
impl<T> ContainsVertex for T where T: ContainsElement {}

/// Hyperedge-ID containment capability for hypergraph views.
///
/// This is the hypergraph-facing name for [`ContainsRelation`]. It answers
/// whether a hyperedge ID is valid and visible in this hypergraph view.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsHyperedge: ContainsRelation {
    /// Returns whether `hyperedge` is valid and visible in this hypergraph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_hyperedge(&self, hyperedge: Self::RelationId) -> bool {
        self.contains_relation(hyperedge)
    }
}

/// Blanket implementation for hypergraph views with relation containment.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ContainsRelation`].
impl<T> ContainsHyperedge for T where T: ContainsRelation {}

/// Participant-ID containment capability for hypergraph views with incidences.
///
/// This is the hypergraph-facing name for [`ContainsIncidence`]. It answers
/// whether a participant ID is valid and visible in this hypergraph view.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait ContainsParticipant: ContainsIncidence {
    /// Returns whether `participant` is valid and visible in this hypergraph view.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn contains_participant(&self, participant: Self::IncidenceId) -> bool {
        self.contains_incidence(participant)
    }
}

/// Blanket implementation for hypergraph views with incidence containment.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from [`ContainsIncidence`].
impl<T> ContainsParticipant for T where T: ContainsIncidence {}

/// Capability for traversing vertices participating in one hyperedge.
///
/// This is the hypergraph-facing form of relation-to-element traversal.
/// Implementations may derive it from incidence records, direct participant
/// arrays, generated views, or validated snapshot sections.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` participants should be `O(k)` because the
/// output itself has length `k`.
pub trait HyperedgeParticipants: RelationIncidences + IncidenceElement {
    /// Iterator over vertices participating in one hyperedge.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type Participants<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns vertices participating in `hyperedge`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` participants is
    /// expected `O(k)`.
    fn hyperedge_participants(&self, hyperedge: Self::RelationId) -> Self::Participants<'_>;
}

/// Capability for traversing hyperedges incident to one vertex.
///
/// This is the hypergraph-facing form of element-to-relation traversal.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` unless an implementation documents a
/// weaker contract. Yielding `k` incident hyperedges should be `O(k)` because
/// the output itself has length `k`.
pub trait IncidentHyperedges: ElementIncidences + IncidenceRelation {
    /// Iterator over hyperedges incident to one vertex.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type IncidentHyperedges<'view>: Iterator<Item = Self::RelationId>
    where
        Self: 'view;

    /// Returns hyperedges incident to `vertex`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` hyperedges is
    /// expected `O(k)`.
    fn incident_hyperedges(&self, vertex: Self::ElementId) -> Self::IncidentHyperedges<'_>;
}

/// Exact hyperedge-participant count capability.
///
/// This pairs with [`HyperedgeParticipants`] for backends that can report a
/// hyperedge's participant count without traversal.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait HyperedgeParticipantCount: IncidenceBase {
    /// Returns the number of participants attached to `hyperedge`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn hyperedge_participant_count(&self, hyperedge: Self::RelationId) -> usize;
}

/// Exact incident-hyperedge count capability.
///
/// This pairs with [`IncidentHyperedges`] for backends that can report a
/// vertex's incident hyperedge count without traversal.
///
/// # Performance
///
/// Expected `O(1)` unless the implementation documents otherwise.
pub trait IncidentHyperedgeCount: IncidenceBase {
    /// Returns the number of hyperedges incident to `vertex`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` unless the implementation documents otherwise.
    fn incident_hyperedge_count(&self, vertex: Self::ElementId) -> usize;
}

/// Capability for traversing directed hyperedge participant sets.
///
/// Directed hypergraphs distinguish source-side and target-side participants.
/// The core does not prescribe how a view stores those sets or whether they are
/// derived from roles, separate indexes, or generated views.
///
/// # Performance
///
/// Creating either iterator should be `O(1)` unless an implementation documents
/// a weaker contract. Yielding `k` participants should be `O(k)`.
pub trait DirectedHyperedgeParticipants: TopologyBase {
    /// Iterator over source-side participants in one directed hyperedge.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type SourceParticipants<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Iterator over target-side participants in one directed hyperedge.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type TargetParticipants<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns source-side participants for `hyperedge`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` participants is
    /// expected `O(k)`.
    fn source_participants(&self, hyperedge: Self::RelationId) -> Self::SourceParticipants<'_>;

    /// Returns target-side participants for `hyperedge`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` participants is
    /// expected `O(k)`.
    fn target_participants(&self, hyperedge: Self::RelationId) -> Self::TargetParticipants<'_>;
}

/// Capability for expanding a directed hypergraph vertex to successor vertices.
///
/// A successor vertex is a target-side participant reachable through a directed
/// hyperedge where the input vertex participates on the source side. This is a
/// vertex-to-vertex expansion capability, distinct from relation-level
/// [`DirectedHyperedgeParticipants`] traversal.
///
/// Implementations define whether repeated connections through multiple
/// hyperedges, or multiple participant records in one hyperedge, produce
/// duplicate vertices. Implementations should document whether they preserve
/// multiplicity or deduplicate results.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` when backed by an index unless an
/// implementation documents a weaker contract. Yielding `k` vertices should be
/// `O(k)` and should not allocate unless the implementation documents otherwise.
pub trait DirectedVertexSuccessors: TopologyBase {
    /// Iterator over successor vertices reachable from one source-side vertex.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type Successors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns target-side vertices reachable from `vertex`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator when backed by an index; yielding
    /// `k` vertices is expected `O(k)`.
    fn successor_vertices(&self, vertex: Self::ElementId) -> Self::Successors<'_>;
}

/// Capability for expanding a directed hypergraph vertex to predecessor vertices.
///
/// A predecessor vertex is a source-side participant reachable through a directed
/// hyperedge where the input vertex participates on the target side. This is a
/// vertex-to-vertex expansion capability, distinct from relation-level
/// [`DirectedHyperedgeParticipants`] traversal.
///
/// Implementations define whether repeated connections through multiple
/// hyperedges, or multiple participant records in one hyperedge, produce
/// duplicate vertices. Implementations should document whether they preserve
/// multiplicity or deduplicate results.
///
/// # Performance
///
/// Creating the iterator should be `O(1)` when backed by an index unless an
/// implementation documents a weaker contract. Yielding `k` vertices should be
/// `O(k)` and should not allocate unless the implementation documents otherwise.
pub trait DirectedVertexPredecessors: TopologyBase {
    /// Iterator over predecessor vertices reaching one target-side vertex.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)` unless an
    /// implementation documents otherwise.
    type Predecessors<'view>: Iterator<Item = Self::ElementId>
    where
        Self: 'view;

    /// Returns source-side vertices that can reach `vertex`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator when backed by an index; yielding
    /// `k` vertices is expected `O(k)`.
    fn predecessor_vertices(&self, vertex: Self::ElementId) -> Self::Predecessors<'_>;
}

/// Convenience trait for hypergraph views with both traversal directions.
///
/// This trait has no methods of its own. It names the common capability bundle
/// for generic hypergraph code that needs hyperedge participants and incident
/// hyperedges.
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
pub trait Hypergraph: HyperedgeParticipants + IncidentHyperedges {}

/// Blanket implementation for complete hypergraph views.
///
/// Any hypergraph view that can traverse participants and incident hyperedges
/// automatically implements [`Hypergraph`].
///
/// # Performance
///
/// `perf: unspecified`; performance is inherited from the component traits.
impl<T> Hypergraph for T where T: HyperedgeParticipants + IncidentHyperedges {}
