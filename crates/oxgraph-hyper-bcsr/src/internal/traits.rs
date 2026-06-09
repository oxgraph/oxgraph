//! Topology and hypergraph trait implementations for [`BcsrHypergraph`].
//!
//! [`BcsrHypergraph`]: crate::BcsrHypergraph

use oxgraph_hyper::{
    ContainsElement, ContainsIncidence, ContainsRelation, DirectedHyperedgeIncidences,
    DirectedHyperedgeParticipants, DirectedVertexHyperedges, ElementIncidenceCount,
    ElementIncidences, ElementIndex, ElementPredecessors, ElementSuccessors, HyperedgeParticipants,
    HypergraphCounts, IncidenceBase, IncidenceCounts, IncidenceElement, IncidenceIndex,
    IncidenceRelation, IncidenceRole, IncidentHyperedges, RelationIncidenceCount,
    RelationIncidences, RelationIndex, TopologyBase, TopologyCounts,
};

use crate::{
    id::{BcsrHyperedgeId, BcsrParticipantId, BcsrRole, BcsrVertexId},
    internal::{
        iter::{
            BcsrChainedHyperedges, BcsrChainedParticipants, BcsrChainedRelationIncidences,
            BcsrElementIncidences, BcsrHyperedgeSlice, BcsrParticipantSlice,
            BcsrPredecessorVertices, BcsrSuccessorVertices, BcsrVertexSlice,
        },
        validation::{index_to_usize_validated, usize_to_index_validated},
        view::BcsrHypergraph,
    },
    word::{LayoutIndex, LayoutWord},
};

/// Private shorthand for the generic BCSR view.
type View<'a, V, R, I, O, VW, RW> = BcsrHypergraph<'a, V, R, I, O, VW, RW>;

/// By-value traversal constructors whose returned iterators borrow the
/// underlying sections (`'view`), not the view value itself.
///
/// The view is `Copy`, so callers holding only a temporary copy — such as the
/// frozen build wrappers delegating through `as_view` — can hand the returned
/// iterators out for the full section lifetime. The borrowed-view trait impls
/// below delegate here so each traversal is implemented once.
impl<'view, V, R, I, O, VW, RW> View<'view, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    /// By-value variant of [`ElementIncidences::element_incidences`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(log d_h)` per
    /// item for maximum hyperedge participant-set size `d_h`.
    pub(crate) fn detached_element_incidences(
        self,
        element: BcsrVertexId<V>,
    ) -> BcsrElementIncidences<'view, O, VW, RW> {
        let sections = self.sections();
        let counts = self.counts();
        let v_index = index_to_usize_validated(element.get());
        let outgoing = vertex_bucket(
            sections.vertex_outgoing_offsets,
            sections.vertex_outgoing_hyperedges,
            v_index,
        );
        let incoming = vertex_bucket(
            sections.vertex_incoming_offsets,
            sections.vertex_incoming_hyperedges,
            v_index,
        );
        BcsrElementIncidences::new(v_index, counts.p_outgoing, outgoing, incoming, sections)
    }

    /// By-value variant of [`HyperedgeParticipants::hyperedge_participants`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)`.
    pub(crate) fn detached_hyperedge_participants(
        self,
        hyperedge: BcsrHyperedgeId<R>,
    ) -> BcsrChainedParticipants<'view, VW> {
        let sections = self.sections();
        let h_index = index_to_usize_validated(hyperedge.get());
        let head = vertex_bucket(sections.head_offsets, sections.head_participants, h_index);
        let tail = vertex_bucket(sections.tail_offsets, sections.tail_participants, h_index);
        BcsrVertexSlice::new(head).chain(BcsrVertexSlice::new(tail))
    }

    /// By-value variant of [`IncidentHyperedges::incident_hyperedges`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)`.
    pub(crate) fn detached_incident_hyperedges(
        self,
        vertex: BcsrVertexId<V>,
    ) -> BcsrChainedHyperedges<'view, RW> {
        let sections = self.sections();
        let v_index = index_to_usize_validated(vertex.get());
        let outgoing = vertex_bucket(
            sections.vertex_outgoing_offsets,
            sections.vertex_outgoing_hyperedges,
            v_index,
        );
        let incoming = vertex_bucket(
            sections.vertex_incoming_offsets,
            sections.vertex_incoming_hyperedges,
            v_index,
        );
        BcsrHyperedgeSlice::new(outgoing).chain(BcsrHyperedgeSlice::new(incoming))
    }

    /// By-value variant of [`DirectedHyperedgeParticipants::source_participants`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)`.
    pub(crate) fn detached_source_participants(
        self,
        hyperedge: BcsrHyperedgeId<R>,
    ) -> BcsrVertexSlice<'view, VW> {
        let sections = self.sections();
        let h_index = index_to_usize_validated(hyperedge.get());
        let head = vertex_bucket(sections.head_offsets, sections.head_participants, h_index);
        BcsrVertexSlice::new(head)
    }

    /// By-value variant of [`DirectedHyperedgeParticipants::target_participants`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)`.
    pub(crate) fn detached_target_participants(
        self,
        hyperedge: BcsrHyperedgeId<R>,
    ) -> BcsrVertexSlice<'view, VW> {
        let sections = self.sections();
        let h_index = index_to_usize_validated(hyperedge.get());
        let tail = vertex_bucket(sections.tail_offsets, sections.tail_participants, h_index);
        BcsrVertexSlice::new(tail)
    }

    /// By-value variant of [`DirectedVertexHyperedges::outgoing_hyperedges`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)`.
    pub(crate) fn detached_outgoing_hyperedges(
        self,
        vertex: BcsrVertexId<V>,
    ) -> BcsrHyperedgeSlice<'view, RW> {
        let sections = self.sections();
        let v_index = index_to_usize_validated(vertex.get());
        let outgoing = vertex_bucket(
            sections.vertex_outgoing_offsets,
            sections.vertex_outgoing_hyperedges,
            v_index,
        );
        BcsrHyperedgeSlice::new(outgoing)
    }

    /// By-value variant of [`DirectedVertexHyperedges::incoming_hyperedges`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)`.
    pub(crate) fn detached_incoming_hyperedges(
        self,
        vertex: BcsrVertexId<V>,
    ) -> BcsrHyperedgeSlice<'view, RW> {
        let sections = self.sections();
        let v_index = index_to_usize_validated(vertex.get());
        let incoming = vertex_bucket(
            sections.vertex_incoming_offsets,
            sections.vertex_incoming_hyperedges,
            v_index,
        );
        BcsrHyperedgeSlice::new(incoming)
    }

    /// By-value variant of [`ElementSuccessors::element_successors`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)` amortised.
    pub(crate) fn detached_element_successors(
        self,
        vertex: BcsrVertexId<V>,
    ) -> BcsrSuccessorVertices<'view, RW, O, VW> {
        let sections = self.sections();
        let v_index = index_to_usize_validated(vertex.get());
        let outgoing = vertex_bucket(
            sections.vertex_outgoing_offsets,
            sections.vertex_outgoing_hyperedges,
            v_index,
        );
        BcsrSuccessorVertices::new(outgoing, sections.tail_offsets, sections.tail_participants)
    }

    /// By-value variant of [`ElementPredecessors::element_predecessors`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(1)` amortised.
    pub(crate) fn detached_element_predecessors(
        self,
        vertex: BcsrVertexId<V>,
    ) -> BcsrPredecessorVertices<'view, RW, O, VW> {
        let sections = self.sections();
        let v_index = index_to_usize_validated(vertex.get());
        let incoming = vertex_bucket(
            sections.vertex_incoming_offsets,
            sections.vertex_incoming_hyperedges,
            v_index,
        );
        BcsrPredecessorVertices::new(incoming, sections.head_offsets, sections.head_participants)
    }
}

impl<V, R, I, O, VW, RW> TopologyBase for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type ElementId = BcsrVertexId<V>;
    type RelationId = BcsrHyperedgeId<R>;
}

impl<V, R, I, O, VW, RW> IncidenceBase for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type IncidenceId = BcsrParticipantId<I>;
    type Role = BcsrRole;
}

impl<V, R, I, O, VW, RW> TopologyCounts for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn element_count(&self) -> usize {
        self.vertex_count()
    }

    fn relation_count(&self) -> usize {
        self.hyperedge_count()
    }
}

impl<V, R, I, O, VW, RW> IncidenceCounts for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn incidence_count(&self) -> usize {
        self.counts().total_incidences
    }
}

impl<V, R, I, O, VW, RW> HypergraphCounts for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
}

impl<V, R, I, O, VW, RW> ElementIndex for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn element_bound(&self) -> usize {
        self.vertex_count()
    }

    fn element_index(&self, element: BcsrVertexId<V>) -> usize {
        index_to_usize_validated(element.get())
    }
}

impl<V, R, I, O, VW, RW> RelationIndex for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn relation_bound(&self) -> usize {
        self.hyperedge_count()
    }

    fn relation_index(&self, relation: BcsrHyperedgeId<R>) -> usize {
        index_to_usize_validated(relation.get())
    }
}

impl<V, R, I, O, VW, RW> IncidenceIndex for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn incidence_bound(&self) -> usize {
        self.counts().total_incidences
    }

    fn incidence_index(&self, incidence: BcsrParticipantId<I>) -> usize {
        index_to_usize_validated(incidence.get())
    }
}

impl<V, R, I, O, VW, RW> ContainsElement for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn contains_element(&self, element: BcsrVertexId<V>) -> bool {
        index_to_usize_validated(element.get()) < self.counts().vertex_count
    }
}

impl<V, R, I, O, VW, RW> ContainsRelation for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn contains_relation(&self, relation: BcsrHyperedgeId<R>) -> bool {
        index_to_usize_validated(relation.get()) < self.counts().hyperedge_count
    }
}

impl<V, R, I, O, VW, RW> ContainsIncidence for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn contains_incidence(&self, incidence: BcsrParticipantId<I>) -> bool {
        index_to_usize_validated(incidence.get()) < self.counts().total_incidences
    }
}

impl<V, R, I, O, VW, RW> IncidenceElement for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn incidence_element(&self, incidence: BcsrParticipantId<I>) -> BcsrVertexId<V> {
        let incidence_index = index_to_usize_validated(incidence.get());
        let counts = self.counts();
        let sections = self.sections();
        if incidence_index < counts.p_outgoing {
            BcsrVertexId::new(sections.head_participants[incidence_index].get())
        } else {
            let position = incidence_index - counts.p_outgoing;
            BcsrVertexId::new(sections.tail_participants[position].get())
        }
    }
}

impl<V, R, I, O, VW, RW> IncidenceRelation for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn incidence_relation(&self, incidence: BcsrParticipantId<I>) -> BcsrHyperedgeId<R> {
        let incidence_index = index_to_usize_validated(incidence.get());
        let counts = self.counts();
        let sections = self.sections();
        if incidence_index < counts.p_outgoing {
            BcsrHyperedgeId::new(locate_owning_bucket(sections.head_offsets, incidence_index))
        } else {
            let position = incidence_index - counts.p_outgoing;
            BcsrHyperedgeId::new(locate_owning_bucket(sections.tail_offsets, position))
        }
    }
}

impl<V, R, I, O, VW, RW> IncidenceRole for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn incidence_role(&self, incidence: BcsrParticipantId<I>) -> BcsrRole {
        if index_to_usize_validated(incidence.get()) < self.counts().p_outgoing {
            BcsrRole::Head
        } else {
            BcsrRole::Tail
        }
    }
}

impl<V, R, I, O, VW, RW> RelationIncidences for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type Incidences<'view>
        = BcsrChainedRelationIncidences<I>
    where
        Self: 'view;

    fn relation_incidences(&self, relation: BcsrHyperedgeId<R>) -> Self::Incidences<'_> {
        let sections = self.sections();
        let p_outgoing = self.counts().p_outgoing;
        let h_index = index_to_usize_validated(relation.get());
        let head_start = index_to_usize_validated(sections.head_offsets[h_index].get());
        let head_end = index_to_usize_validated(sections.head_offsets[h_index + 1].get());
        let tail_start = index_to_usize_validated(sections.tail_offsets[h_index].get());
        let tail_end = index_to_usize_validated(sections.tail_offsets[h_index + 1].get());
        BcsrParticipantSlice::new(head_start, head_end, 0)
            .chain(BcsrParticipantSlice::new(tail_start, tail_end, p_outgoing))
    }
}

impl<V, R, I, O, VW, RW> ElementIncidences for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type Incidences<'view>
        = BcsrElementIncidences<'view, O, VW, RW>
    where
        Self: 'view;

    fn element_incidences(&self, element: BcsrVertexId<V>) -> Self::Incidences<'_> {
        self.detached_element_incidences(element)
    }
}

impl<V, R, I, O, VW, RW> RelationIncidenceCount for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn relation_incidence_count(&self, relation: BcsrHyperedgeId<R>) -> usize {
        let sections = self.sections();
        let h_index = index_to_usize_validated(relation.get());
        let head_size = index_to_usize_validated(sections.head_offsets[h_index + 1].get())
            - index_to_usize_validated(sections.head_offsets[h_index].get());
        let tail_size = index_to_usize_validated(sections.tail_offsets[h_index + 1].get())
            - index_to_usize_validated(sections.tail_offsets[h_index].get());
        head_size + tail_size
    }
}

impl<V, R, I, O, VW, RW> ElementIncidenceCount for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    fn element_incidence_count(&self, element: BcsrVertexId<V>) -> usize {
        let sections = self.sections();
        let v_index = index_to_usize_validated(element.get());
        let out_size =
            index_to_usize_validated(sections.vertex_outgoing_offsets[v_index + 1].get())
                - index_to_usize_validated(sections.vertex_outgoing_offsets[v_index].get());
        let in_size = index_to_usize_validated(sections.vertex_incoming_offsets[v_index + 1].get())
            - index_to_usize_validated(sections.vertex_incoming_offsets[v_index].get());
        out_size + in_size
    }
}

// `HyperedgeParticipantCount` and `IncidentHyperedgeCount` are provided for free
// by the blanket impls in `oxgraph-hyper` over `RelationIncidenceCount` /
// `ElementIncidenceCount`, which this view implements above.

impl<V, R, I, O, VW, RW> HyperedgeParticipants for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type Participants<'view>
        = BcsrChainedParticipants<'view, VW>
    where
        Self: 'view;

    fn hyperedge_participants(&self, hyperedge: BcsrHyperedgeId<R>) -> Self::Participants<'_> {
        self.detached_hyperedge_participants(hyperedge)
    }
}

impl<V, R, I, O, VW, RW> IncidentHyperedges for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type IncidentHyperedges<'view>
        = BcsrChainedHyperedges<'view, RW>
    where
        Self: 'view;

    fn incident_hyperedges(&self, vertex: BcsrVertexId<V>) -> Self::IncidentHyperedges<'_> {
        self.detached_incident_hyperedges(vertex)
    }
}

impl<V, R, I, O, VW, RW> DirectedHyperedgeParticipants for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type SourceParticipants<'view>
        = BcsrVertexSlice<'view, VW>
    where
        Self: 'view;

    type TargetParticipants<'view>
        = BcsrVertexSlice<'view, VW>
    where
        Self: 'view;

    fn source_participants(&self, hyperedge: BcsrHyperedgeId<R>) -> Self::SourceParticipants<'_> {
        self.detached_source_participants(hyperedge)
    }

    fn target_participants(&self, hyperedge: BcsrHyperedgeId<R>) -> Self::TargetParticipants<'_> {
        self.detached_target_participants(hyperedge)
    }
}

impl<V, R, I, O, VW, RW> DirectedHyperedgeIncidences for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type SourceIncidences<'view>
        = BcsrParticipantSlice<I>
    where
        Self: 'view;

    type TargetIncidences<'view>
        = BcsrParticipantSlice<I>
    where
        Self: 'view;

    fn source_incidences(&self, hyperedge: BcsrHyperedgeId<R>) -> Self::SourceIncidences<'_> {
        let sections = self.sections();
        let h_index = index_to_usize_validated(hyperedge.get());
        let head_start = index_to_usize_validated(sections.head_offsets[h_index].get());
        let head_end = index_to_usize_validated(sections.head_offsets[h_index + 1].get());
        BcsrParticipantSlice::new(head_start, head_end, 0)
    }

    fn target_incidences(&self, hyperedge: BcsrHyperedgeId<R>) -> Self::TargetIncidences<'_> {
        let sections = self.sections();
        let h_index = index_to_usize_validated(hyperedge.get());
        let tail_start = index_to_usize_validated(sections.tail_offsets[h_index].get());
        let tail_end = index_to_usize_validated(sections.tail_offsets[h_index + 1].get());
        BcsrParticipantSlice::new(tail_start, tail_end, self.counts().p_outgoing)
    }
}

impl<V, R, I, O, VW, RW> DirectedVertexHyperedges for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type OutgoingHyperedges<'view>
        = BcsrHyperedgeSlice<'view, RW>
    where
        Self: 'view;

    type IncomingHyperedges<'view>
        = BcsrHyperedgeSlice<'view, RW>
    where
        Self: 'view;

    fn outgoing_hyperedges(&self, vertex: BcsrVertexId<V>) -> Self::OutgoingHyperedges<'_> {
        self.detached_outgoing_hyperedges(vertex)
    }

    fn incoming_hyperedges(&self, vertex: BcsrVertexId<V>) -> Self::IncomingHyperedges<'_> {
        self.detached_incoming_hyperedges(vertex)
    }
}

impl<V, R, I, O, VW, RW> ElementSuccessors for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type Successors<'view>
        = BcsrSuccessorVertices<'view, RW, O, VW>
    where
        Self: 'view;

    fn element_successors(&self, vertex: BcsrVertexId<V>) -> Self::Successors<'_> {
        self.detached_element_successors(vertex)
    }
}

impl<V, R, I, O, VW, RW> ElementPredecessors for View<'_, V, R, I, O, VW, RW>
where
    V: LayoutIndex,
    R: LayoutIndex,
    I: LayoutIndex,
    O: LayoutWord<Index = I>,
    VW: LayoutWord<Index = V>,
    RW: LayoutWord<Index = R>,
{
    type Predecessors<'view>
        = BcsrPredecessorVertices<'view, RW, O, VW>
    where
        Self: 'view;

    fn element_predecessors(&self, vertex: BcsrVertexId<V>) -> Self::Predecessors<'_> {
        self.detached_element_predecessors(vertex)
    }
}

/// Returns the slice `values[offsets[index].get()..offsets[index + 1].get()]`.
fn vertex_bucket<'view, OffsetWord, ValueWord>(
    offsets: &'view [OffsetWord],
    values: &'view [ValueWord],
    index: usize,
) -> &'view [ValueWord]
where
    OffsetWord: LayoutWord,
{
    let start = index_to_usize_validated(offsets[index].get());
    let end = index_to_usize_validated(offsets[index + 1].get());
    &values[start..end]
}

/// Binary search over an offset array, returning the bucket index that owns
/// absolute position `target`.
fn locate_owning_bucket<OffsetWord, RelationIndex>(
    offsets: &[OffsetWord],
    target: usize,
) -> RelationIndex
where
    OffsetWord: LayoutWord,
    RelationIndex: LayoutIndex,
{
    let mut low = 0_usize;
    let mut high = offsets.len() - 1;
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if index_to_usize_validated(offsets[mid].get()) <= target {
            low = mid;
        } else {
            high = mid;
        }
    }
    usize_to_index_validated(low)
}
