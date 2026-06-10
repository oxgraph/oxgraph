//! Topology and hypergraph trait implementations for [`BcsrHypergraph`].
//!
//! [`BcsrHypergraph`]: crate::BcsrHypergraph

use oxgraph_hyper::{
    ContainsElement, ContainsIncidence, ContainsRelation, DenseElementIndex, DenseIncidenceIndex,
    DenseRelationIndex, DirectedHyperedgeIncidences, DirectedHyperedgeParticipants,
    DirectedVertexHyperedges, ElementIncidenceCount, ElementIncidences, ElementPredecessors,
    ElementSuccessors, HyperedgeParticipants, IncidenceBase, IncidenceCounts, IncidenceElement,
    IncidenceRelation, IncidenceRole, IncidentHyperedges, RelationIncidenceCount,
    RelationIncidences, TopologyBase, TopologyCounts,
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
    word::{BcsrWords, LayoutIndex, LayoutWord},
};

/// By-value traversal constructors whose returned iterators borrow the
/// underlying sections (`'view`), not the view value itself.
///
/// The view is `Copy`, so callers holding only a temporary copy — such as the
/// frozen build wrappers delegating through `as_view` — can hand the returned
/// iterators out for the full section lifetime. The borrowed-view trait impls
/// below delegate here so each traversal is implemented once.
impl<'view, W: BcsrWords> BcsrHypergraph<'view, W> {
    /// By-value variant of [`ElementIncidences::element_incidences`].
    ///
    /// # Performance
    ///
    /// Construction is `O(1)`; advancing the iterator is `O(log d_h)` per
    /// item for maximum hyperedge participant-set size `d_h`.
    pub(crate) fn detached_element_incidences(
        self,
        element: BcsrVertexId<W::VertexIndex>,
    ) -> BcsrElementIncidences<'view, W::OffsetWord, W::VertexWord, W::RelationWord> {
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
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> BcsrChainedParticipants<'view, W::VertexWord> {
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
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> BcsrChainedHyperedges<'view, W::RelationWord> {
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
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> BcsrVertexSlice<'view, W::VertexWord> {
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
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> BcsrVertexSlice<'view, W::VertexWord> {
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
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> BcsrHyperedgeSlice<'view, W::RelationWord> {
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
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> BcsrHyperedgeSlice<'view, W::RelationWord> {
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
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> BcsrSuccessorVertices<'view, W::RelationWord, W::OffsetWord, W::VertexWord> {
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
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> BcsrPredecessorVertices<'view, W::RelationWord, W::OffsetWord, W::VertexWord> {
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

impl<W: BcsrWords> TopologyBase for BcsrHypergraph<'_, W> {
    type ElementId = BcsrVertexId<W::VertexIndex>;
    type RelationId = BcsrHyperedgeId<W::RelationIndex>;
}

impl<W: BcsrWords> IncidenceBase for BcsrHypergraph<'_, W> {
    type IncidenceId = BcsrParticipantId<W::IncidenceIndex>;
    type Role = BcsrRole;
}

impl<W: BcsrWords> TopologyCounts for BcsrHypergraph<'_, W> {
    fn element_count(&self) -> usize {
        self.vertex_count()
    }

    fn relation_count(&self) -> usize {
        self.hyperedge_count()
    }
}

impl<W: BcsrWords> IncidenceCounts for BcsrHypergraph<'_, W> {
    fn incidence_count(&self) -> usize {
        self.counts().total_incidences
    }
}

impl<W: BcsrWords> DenseElementIndex for BcsrHypergraph<'_, W> {
    fn element_bound(&self) -> usize {
        self.vertex_count()
    }

    fn element_index(&self, element: BcsrVertexId<W::VertexIndex>) -> usize {
        index_to_usize_validated(element.get())
    }
}

impl<W: BcsrWords> DenseRelationIndex for BcsrHypergraph<'_, W> {
    fn relation_bound(&self) -> usize {
        self.hyperedge_count()
    }

    fn relation_index(&self, relation: BcsrHyperedgeId<W::RelationIndex>) -> usize {
        index_to_usize_validated(relation.get())
    }
}

impl<W: BcsrWords> DenseIncidenceIndex for BcsrHypergraph<'_, W> {
    fn incidence_bound(&self) -> usize {
        self.counts().total_incidences
    }

    fn incidence_index(&self, incidence: BcsrParticipantId<W::IncidenceIndex>) -> usize {
        index_to_usize_validated(incidence.get())
    }
}

impl<W: BcsrWords> ContainsElement for BcsrHypergraph<'_, W> {
    fn contains_element(&self, element: BcsrVertexId<W::VertexIndex>) -> bool {
        index_to_usize_validated(element.get()) < self.counts().vertex_count
    }
}

impl<W: BcsrWords> ContainsRelation for BcsrHypergraph<'_, W> {
    fn contains_relation(&self, relation: BcsrHyperedgeId<W::RelationIndex>) -> bool {
        index_to_usize_validated(relation.get()) < self.counts().hyperedge_count
    }
}

impl<W: BcsrWords> ContainsIncidence for BcsrHypergraph<'_, W> {
    fn contains_incidence(&self, incidence: BcsrParticipantId<W::IncidenceIndex>) -> bool {
        index_to_usize_validated(incidence.get()) < self.counts().total_incidences
    }
}

impl<W: BcsrWords> IncidenceElement for BcsrHypergraph<'_, W> {
    fn incidence_element(
        &self,
        incidence: BcsrParticipantId<W::IncidenceIndex>,
    ) -> BcsrVertexId<W::VertexIndex> {
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

impl<W: BcsrWords> IncidenceRelation for BcsrHypergraph<'_, W> {
    fn incidence_relation(
        &self,
        incidence: BcsrParticipantId<W::IncidenceIndex>,
    ) -> BcsrHyperedgeId<W::RelationIndex> {
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

impl<W: BcsrWords> IncidenceRole for BcsrHypergraph<'_, W> {
    fn incidence_role(&self, incidence: BcsrParticipantId<W::IncidenceIndex>) -> BcsrRole {
        if index_to_usize_validated(incidence.get()) < self.counts().p_outgoing {
            BcsrRole::Head
        } else {
            BcsrRole::Tail
        }
    }
}

impl<W: BcsrWords> RelationIncidences for BcsrHypergraph<'_, W> {
    type Incidences<'view>
        = BcsrChainedRelationIncidences<W::IncidenceIndex>
    where
        Self: 'view;

    fn relation_incidences(
        &self,
        relation: BcsrHyperedgeId<W::RelationIndex>,
    ) -> Self::Incidences<'_> {
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

impl<W: BcsrWords> ElementIncidences for BcsrHypergraph<'_, W> {
    type Incidences<'view>
        = BcsrElementIncidences<'view, W::OffsetWord, W::VertexWord, W::RelationWord>
    where
        Self: 'view;

    fn element_incidences(&self, element: BcsrVertexId<W::VertexIndex>) -> Self::Incidences<'_> {
        self.detached_element_incidences(element)
    }
}

impl<W: BcsrWords> RelationIncidenceCount for BcsrHypergraph<'_, W> {
    fn relation_incidence_count(&self, relation: BcsrHyperedgeId<W::RelationIndex>) -> usize {
        let sections = self.sections();
        let h_index = index_to_usize_validated(relation.get());
        let head_size = index_to_usize_validated(sections.head_offsets[h_index + 1].get())
            - index_to_usize_validated(sections.head_offsets[h_index].get());
        let tail_size = index_to_usize_validated(sections.tail_offsets[h_index + 1].get())
            - index_to_usize_validated(sections.tail_offsets[h_index].get());
        head_size + tail_size
    }
}

impl<W: BcsrWords> ElementIncidenceCount for BcsrHypergraph<'_, W> {
    fn element_incidence_count(&self, element: BcsrVertexId<W::VertexIndex>) -> usize {
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

impl<W: BcsrWords> HyperedgeParticipants for BcsrHypergraph<'_, W> {
    type Participants<'view>
        = BcsrChainedParticipants<'view, W::VertexWord>
    where
        Self: 'view;

    fn hyperedge_participants(
        &self,
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> Self::Participants<'_> {
        self.detached_hyperedge_participants(hyperedge)
    }
}

impl<W: BcsrWords> IncidentHyperedges for BcsrHypergraph<'_, W> {
    type IncidentHyperedges<'view>
        = BcsrChainedHyperedges<'view, W::RelationWord>
    where
        Self: 'view;

    fn incident_hyperedges(
        &self,
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> Self::IncidentHyperedges<'_> {
        self.detached_incident_hyperedges(vertex)
    }
}

impl<W: BcsrWords> DirectedHyperedgeParticipants for BcsrHypergraph<'_, W> {
    type SourceParticipants<'view>
        = BcsrVertexSlice<'view, W::VertexWord>
    where
        Self: 'view;

    type TargetParticipants<'view>
        = BcsrVertexSlice<'view, W::VertexWord>
    where
        Self: 'view;

    fn source_participants(
        &self,
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> Self::SourceParticipants<'_> {
        self.detached_source_participants(hyperedge)
    }

    fn target_participants(
        &self,
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> Self::TargetParticipants<'_> {
        self.detached_target_participants(hyperedge)
    }
}

impl<W: BcsrWords> DirectedHyperedgeIncidences for BcsrHypergraph<'_, W> {
    type SourceIncidences<'view>
        = BcsrParticipantSlice<W::IncidenceIndex>
    where
        Self: 'view;

    type TargetIncidences<'view>
        = BcsrParticipantSlice<W::IncidenceIndex>
    where
        Self: 'view;

    fn source_incidences(
        &self,
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> Self::SourceIncidences<'_> {
        let sections = self.sections();
        let h_index = index_to_usize_validated(hyperedge.get());
        let head_start = index_to_usize_validated(sections.head_offsets[h_index].get());
        let head_end = index_to_usize_validated(sections.head_offsets[h_index + 1].get());
        BcsrParticipantSlice::new(head_start, head_end, 0)
    }

    fn target_incidences(
        &self,
        hyperedge: BcsrHyperedgeId<W::RelationIndex>,
    ) -> Self::TargetIncidences<'_> {
        let sections = self.sections();
        let h_index = index_to_usize_validated(hyperedge.get());
        let tail_start = index_to_usize_validated(sections.tail_offsets[h_index].get());
        let tail_end = index_to_usize_validated(sections.tail_offsets[h_index + 1].get());
        BcsrParticipantSlice::new(tail_start, tail_end, self.counts().p_outgoing)
    }
}

impl<W: BcsrWords> DirectedVertexHyperedges for BcsrHypergraph<'_, W> {
    type OutgoingHyperedges<'view>
        = BcsrHyperedgeSlice<'view, W::RelationWord>
    where
        Self: 'view;

    type IncomingHyperedges<'view>
        = BcsrHyperedgeSlice<'view, W::RelationWord>
    where
        Self: 'view;

    fn outgoing_hyperedges(
        &self,
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> Self::OutgoingHyperedges<'_> {
        self.detached_outgoing_hyperedges(vertex)
    }

    fn incoming_hyperedges(
        &self,
        vertex: BcsrVertexId<W::VertexIndex>,
    ) -> Self::IncomingHyperedges<'_> {
        self.detached_incoming_hyperedges(vertex)
    }
}

impl<W: BcsrWords> ElementSuccessors for BcsrHypergraph<'_, W> {
    type Successors<'view>
        = BcsrSuccessorVertices<'view, W::RelationWord, W::OffsetWord, W::VertexWord>
    where
        Self: 'view;

    fn element_successors(&self, vertex: BcsrVertexId<W::VertexIndex>) -> Self::Successors<'_> {
        self.detached_element_successors(vertex)
    }
}

impl<W: BcsrWords> ElementPredecessors for BcsrHypergraph<'_, W> {
    type Predecessors<'view>
        = BcsrPredecessorVertices<'view, W::RelationWord, W::OffsetWord, W::VertexWord>
    where
        Self: 'view;

    fn element_predecessors(&self, vertex: BcsrVertexId<W::VertexIndex>) -> Self::Predecessors<'_> {
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
