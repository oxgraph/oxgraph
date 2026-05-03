//! Topology and hypergraph trait implementations for [`BcsrHypergraph`].
//!
//! [`BcsrHypergraph`]: crate::BcsrHypergraph

use oxgraph_hyper::{
    ContainsElement, ContainsIncidence, ContainsRelation, DirectedHyperedgeParticipants,
    ElementIncidenceCount, ElementIncidences, ElementIndex, ElementPredecessors, ElementSuccessors,
    HyperedgeParticipantCount, HyperedgeParticipants, HypergraphCounts, IncidenceBase,
    IncidenceCounts, IncidenceElement, IncidenceIndex, IncidenceRelation, IncidenceRole,
    IncidentHyperedgeCount, IncidentHyperedges, RelationIncidenceCount, RelationIncidences,
    RelationIndex, TopologyBase, TopologyCounts,
};

use crate::{
    id::{BcsrHyperedgeId, BcsrParticipantId, BcsrVertexId},
    internal::{
        iter::{
            BcsrChainedHyperedges, BcsrChainedParticipants, BcsrChainedRelationIncidences,
            BcsrElementIncidences, BcsrHyperedgeSlice, BcsrParticipantSlice,
            BcsrPredecessorVertices, BcsrSuccessorVertices, BcsrVertexSlice,
        },
        validation::u32_to_usize_validated,
        view::BcsrHypergraph,
    },
    role::BcsrRole,
    word::BcsrWord,
};

impl<Word: BcsrWord> TopologyBase for BcsrHypergraph<'_, Word> {
    type ElementId = BcsrVertexId;
    type RelationId = BcsrHyperedgeId;
}

impl<Word: BcsrWord> IncidenceBase for BcsrHypergraph<'_, Word> {
    type IncidenceId = BcsrParticipantId;
    type Role = BcsrRole;
}

impl<Word: BcsrWord> TopologyCounts for BcsrHypergraph<'_, Word> {
    fn element_count(&self) -> usize {
        self.vertex_count()
    }

    fn relation_count(&self) -> usize {
        self.hyperedge_count()
    }
}

impl<Word: BcsrWord> IncidenceCounts for BcsrHypergraph<'_, Word> {
    fn incidence_count(&self) -> usize {
        u32_to_usize_validated(self.counts().total_incidences)
    }
}

impl<Word: BcsrWord> HypergraphCounts for BcsrHypergraph<'_, Word> {}

impl<Word: BcsrWord> ElementIndex for BcsrHypergraph<'_, Word> {
    fn element_bound(&self) -> usize {
        self.vertex_count()
    }

    fn element_index(&self, element: BcsrVertexId) -> usize {
        u32_to_usize_validated(element.0)
    }
}

impl<Word: BcsrWord> RelationIndex for BcsrHypergraph<'_, Word> {
    fn relation_bound(&self) -> usize {
        self.hyperedge_count()
    }

    fn relation_index(&self, relation: BcsrHyperedgeId) -> usize {
        u32_to_usize_validated(relation.0)
    }
}

impl<Word: BcsrWord> IncidenceIndex for BcsrHypergraph<'_, Word> {
    fn incidence_bound(&self) -> usize {
        u32_to_usize_validated(self.counts().total_incidences)
    }

    fn incidence_index(&self, incidence: BcsrParticipantId) -> usize {
        u32_to_usize_validated(incidence.0)
    }
}

impl<Word: BcsrWord> ContainsElement for BcsrHypergraph<'_, Word> {
    fn contains_element(&self, element: BcsrVertexId) -> bool {
        element.0 < self.counts().vertex_count
    }
}

impl<Word: BcsrWord> ContainsRelation for BcsrHypergraph<'_, Word> {
    fn contains_relation(&self, relation: BcsrHyperedgeId) -> bool {
        relation.0 < self.counts().hyperedge_count
    }
}

impl<Word: BcsrWord> ContainsIncidence for BcsrHypergraph<'_, Word> {
    fn contains_incidence(&self, incidence: BcsrParticipantId) -> bool {
        incidence.0 < self.counts().total_incidences
    }
}

impl<Word: BcsrWord> IncidenceElement for BcsrHypergraph<'_, Word> {
    fn incidence_element(&self, incidence: BcsrParticipantId) -> BcsrVertexId {
        let counts = self.counts();
        let sections = self.sections();
        if incidence.0 < counts.p_outgoing {
            let position = u32_to_usize_validated(incidence.0);
            BcsrVertexId(sections.head_participants[position].get())
        } else {
            let position = u32_to_usize_validated(incidence.0 - counts.p_outgoing);
            BcsrVertexId(sections.tail_participants[position].get())
        }
    }
}

impl<Word: BcsrWord> IncidenceRelation for BcsrHypergraph<'_, Word> {
    fn incidence_relation(&self, incidence: BcsrParticipantId) -> BcsrHyperedgeId {
        let counts = self.counts();
        let sections = self.sections();
        if incidence.0 < counts.p_outgoing {
            BcsrHyperedgeId(locate_owning_bucket(sections.head_offsets, incidence.0))
        } else {
            let position = incidence.0 - counts.p_outgoing;
            BcsrHyperedgeId(locate_owning_bucket(sections.tail_offsets, position))
        }
    }
}

impl<Word: BcsrWord> IncidenceRole for BcsrHypergraph<'_, Word> {
    fn incidence_role(&self, incidence: BcsrParticipantId) -> BcsrRole {
        if incidence.0 < self.counts().p_outgoing {
            BcsrRole::Head
        } else {
            BcsrRole::Tail
        }
    }
}

impl<Word: BcsrWord> RelationIncidences for BcsrHypergraph<'_, Word> {
    type Incidences<'view>
        = BcsrChainedRelationIncidences
    where
        Self: 'view;

    fn relation_incidences(&self, relation: BcsrHyperedgeId) -> Self::Incidences<'_> {
        let sections = self.sections();
        let p_outgoing = self.counts().p_outgoing;
        let h_index = u32_to_usize_validated(relation.0);
        let head_start = sections.head_offsets[h_index].get();
        let head_end = sections.head_offsets[h_index + 1].get();
        let tail_start = sections.tail_offsets[h_index].get();
        let tail_end = sections.tail_offsets[h_index + 1].get();
        BcsrParticipantSlice::new(head_start, head_end, 0)
            .chain(BcsrParticipantSlice::new(tail_start, tail_end, p_outgoing))
    }
}

impl<Word: BcsrWord> ElementIncidences for BcsrHypergraph<'_, Word> {
    type Incidences<'view>
        = BcsrElementIncidences<'view, Word>
    where
        Self: 'view;

    fn element_incidences(&self, element: BcsrVertexId) -> Self::Incidences<'_> {
        let sections = self.sections();
        let counts = self.counts();
        let v_index = u32_to_usize_validated(element.0);
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
        BcsrElementIncidences::new(element.0, counts.p_outgoing, outgoing, incoming, sections)
    }
}

impl<Word: BcsrWord> RelationIncidenceCount for BcsrHypergraph<'_, Word> {
    fn relation_incidence_count(&self, relation: BcsrHyperedgeId) -> usize {
        let sections = self.sections();
        let h_index = u32_to_usize_validated(relation.0);
        let head_size =
            sections.head_offsets[h_index + 1].get() - sections.head_offsets[h_index].get();
        let tail_size =
            sections.tail_offsets[h_index + 1].get() - sections.tail_offsets[h_index].get();
        u32_to_usize_validated(head_size) + u32_to_usize_validated(tail_size)
    }
}

impl<Word: BcsrWord> ElementIncidenceCount for BcsrHypergraph<'_, Word> {
    fn element_incidence_count(&self, element: BcsrVertexId) -> usize {
        let sections = self.sections();
        let v_index = u32_to_usize_validated(element.0);
        let out_size = sections.vertex_outgoing_offsets[v_index + 1].get()
            - sections.vertex_outgoing_offsets[v_index].get();
        let in_size = sections.vertex_incoming_offsets[v_index + 1].get()
            - sections.vertex_incoming_offsets[v_index].get();
        u32_to_usize_validated(out_size) + u32_to_usize_validated(in_size)
    }
}

impl<Word: BcsrWord> HyperedgeParticipantCount for BcsrHypergraph<'_, Word> {
    fn hyperedge_participant_count(&self, hyperedge: BcsrHyperedgeId) -> usize {
        self.relation_incidence_count(hyperedge)
    }
}

impl<Word: BcsrWord> IncidentHyperedgeCount for BcsrHypergraph<'_, Word> {
    fn incident_hyperedge_count(&self, vertex: BcsrVertexId) -> usize {
        self.element_incidence_count(vertex)
    }
}

impl<Word: BcsrWord> HyperedgeParticipants for BcsrHypergraph<'_, Word> {
    type Participants<'view>
        = BcsrChainedParticipants<'view, Word>
    where
        Self: 'view;

    fn hyperedge_participants(&self, hyperedge: BcsrHyperedgeId) -> Self::Participants<'_> {
        let sections = self.sections();
        let h_index = u32_to_usize_validated(hyperedge.0);
        let head = vertex_bucket(sections.head_offsets, sections.head_participants, h_index);
        let tail = vertex_bucket(sections.tail_offsets, sections.tail_participants, h_index);
        BcsrVertexSlice::new(head).chain(BcsrVertexSlice::new(tail))
    }
}

impl<Word: BcsrWord> IncidentHyperedges for BcsrHypergraph<'_, Word> {
    type IncidentHyperedges<'view>
        = BcsrChainedHyperedges<'view, Word>
    where
        Self: 'view;

    fn incident_hyperedges(&self, vertex: BcsrVertexId) -> Self::IncidentHyperedges<'_> {
        let sections = self.sections();
        let v_index = u32_to_usize_validated(vertex.0);
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
}

impl<Word: BcsrWord> DirectedHyperedgeParticipants for BcsrHypergraph<'_, Word> {
    type SourceParticipants<'view>
        = BcsrVertexSlice<'view, Word>
    where
        Self: 'view;

    type TargetParticipants<'view>
        = BcsrVertexSlice<'view, Word>
    where
        Self: 'view;

    fn source_participants(&self, hyperedge: BcsrHyperedgeId) -> Self::SourceParticipants<'_> {
        let sections = self.sections();
        let h_index = u32_to_usize_validated(hyperedge.0);
        let head = vertex_bucket(sections.head_offsets, sections.head_participants, h_index);
        BcsrVertexSlice::new(head)
    }

    fn target_participants(&self, hyperedge: BcsrHyperedgeId) -> Self::TargetParticipants<'_> {
        let sections = self.sections();
        let h_index = u32_to_usize_validated(hyperedge.0);
        let tail = vertex_bucket(sections.tail_offsets, sections.tail_participants, h_index);
        BcsrVertexSlice::new(tail)
    }
}

impl<Word: BcsrWord> ElementSuccessors for BcsrHypergraph<'_, Word> {
    type Successors<'view>
        = BcsrSuccessorVertices<'view, Word>
    where
        Self: 'view;

    fn element_successors(&self, vertex: BcsrVertexId) -> Self::Successors<'_> {
        let sections = self.sections();
        let v_index = u32_to_usize_validated(vertex.0);
        let outgoing = vertex_bucket(
            sections.vertex_outgoing_offsets,
            sections.vertex_outgoing_hyperedges,
            v_index,
        );
        BcsrSuccessorVertices::new(outgoing, sections.tail_offsets, sections.tail_participants)
    }
}

impl<Word: BcsrWord> ElementPredecessors for BcsrHypergraph<'_, Word> {
    type Predecessors<'view>
        = BcsrPredecessorVertices<'view, Word>
    where
        Self: 'view;

    fn element_predecessors(&self, vertex: BcsrVertexId) -> Self::Predecessors<'_> {
        let sections = self.sections();
        let v_index = u32_to_usize_validated(vertex.0);
        let incoming = vertex_bucket(
            sections.vertex_incoming_offsets,
            sections.vertex_incoming_hyperedges,
            v_index,
        );
        BcsrPredecessorVertices::new(incoming, sections.head_offsets, sections.head_participants)
    }
}

/// Returns the slice `values[offsets[index].get()..offsets[index + 1].get()]`.
///
/// This helper is used by every per-bucket trait impl. Callers must pass an
/// `index` that has already been validated to be in range; behaviour is
/// otherwise the standard slice-index panic.
///
/// # Performance
///
/// This function is `O(1)`.
fn vertex_bucket<'view, Word: BcsrWord>(
    offsets: &'view [Word],
    values: &'view [Word],
    index: usize,
) -> &'view [Word] {
    let start = u32_to_usize_validated(offsets[index].get());
    let end = u32_to_usize_validated(offsets[index + 1].get());
    &values[start..end]
}

/// Linear-bound binary search over an offset array, returning the bucket
/// index that owns the absolute position `target`.
///
/// `offsets` is required to be monotonic non-decreasing with `offsets[0] = 0`
/// and `offsets[hyperedge_count] = total`. The search returns the unique
/// index `h` such that `offsets[h] <= target < offsets[h + 1]`.
///
/// # Performance
///
/// This function is `O(log hyperedge_count)`.
fn locate_owning_bucket<Word: BcsrWord>(offsets: &[Word], target: u32) -> u32 {
    let mut low = 0_usize;
    let mut high = offsets.len() - 1;
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if offsets[mid].get() <= target {
            low = mid;
        } else {
            high = mid;
        }
    }
    match u32::try_from(low) {
        Ok(value) => value,
        Err(_error) => unreachable!("validated bucket index must fit u32"),
    }
}
