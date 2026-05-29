//! Static-dispatch tests for [`BcsrHypergraph`] over `oxgraph-hyper` traits.

use oxgraph_hyper::{
    ContainsElement, ContainsIncidence, ContainsRelation, DirectedHyperedgeParticipants,
    DirectedVertexPredecessors, DirectedVertexSuccessors, ElementIncidenceCount, ElementIndex,
    HyperedgeParticipantCount, HyperedgeParticipants, HypergraphCounts, IncidenceBase,
    IncidenceElement, IncidenceIndex, IncidenceRelation, IncidenceRole, IncidentHyperedgeCount,
    IncidentHyperedges, RelationIncidenceCount, RelationIncidences, RelationIndex, TopologyBase,
};
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHyperedgeId, BcsrHypergraph, BcsrNativeHypergraph, BcsrParticipantId, BcsrRole,
    BcsrSections, BcsrVertexId,
};

/// Hand-built canonical fixture (same layout as `tests/bcsr.rs`).
fn canonical_sections() -> BcsrSections<'static, u32, u32, u32> {
    static HEAD_OFFSETS: &[u32] = &[0, 1, 2];
    static HEAD_PARTICIPANTS: &[u32] = &[0, 1];
    static TAIL_OFFSETS: &[u32] = &[0, 2, 3];
    static TAIL_PARTICIPANTS: &[u32] = &[1, 2, 2];
    static VERTEX_OUTGOING_OFFSETS: &[u32] = &[0, 1, 2, 2];
    static VERTEX_OUTGOING_HYPEREDGES: &[u32] = &[0, 1];
    static VERTEX_INCOMING_OFFSETS: &[u32] = &[0, 0, 1, 3];
    static VERTEX_INCOMING_HYPEREDGES: &[u32] = &[0, 0, 1];
    BcsrSections {
        head_offsets: HEAD_OFFSETS,
        head_participants: HEAD_PARTICIPANTS,
        tail_offsets: TAIL_OFFSETS,
        tail_participants: TAIL_PARTICIPANTS,
        vertex_outgoing_offsets: VERTEX_OUTGOING_OFFSETS,
        vertex_outgoing_hyperedges: VERTEX_OUTGOING_HYPEREDGES,
        vertex_incoming_offsets: VERTEX_INCOMING_OFFSETS,
        vertex_incoming_hyperedges: VERTEX_INCOMING_HYPEREDGES,
    }
}

fn collect_source_participants<H>(view: &H, hyperedge: H::RelationId) -> Vec<H::ElementId>
where
    H: DirectedHyperedgeParticipants,
{
    view.source_participants(hyperedge).collect()
}

fn collect_target_participants<H>(view: &H, hyperedge: H::RelationId) -> Vec<H::ElementId>
where
    H: DirectedHyperedgeParticipants,
{
    view.target_participants(hyperedge).collect()
}

fn collect_hyperedge_participants<H>(view: &H, hyperedge: H::RelationId) -> Vec<H::ElementId>
where
    H: HyperedgeParticipants,
{
    view.hyperedge_participants(hyperedge).collect()
}

fn collect_incident<H>(view: &H, vertex: H::ElementId) -> Vec<H::RelationId>
where
    H: IncidentHyperedges,
{
    view.incident_hyperedges(vertex).collect()
}

fn collect_successors<H>(view: &H, vertex: H::ElementId) -> Vec<H::ElementId>
where
    H: DirectedVertexSuccessors,
{
    view.successor_vertices(vertex).collect()
}

fn collect_predecessors<H>(view: &H, vertex: H::ElementId) -> Vec<H::ElementId>
where
    H: DirectedVertexPredecessors,
{
    view.predecessor_vertices(vertex).collect()
}

fn count_relation_incidences<H>(view: &H, hyperedge: H::RelationId) -> usize
where
    H: RelationIncidences,
{
    view.relation_incidences(hyperedge).count()
}

fn vertex_count_via_trait<H: HypergraphCounts>(view: &H) -> usize {
    view.vertex_count()
}

fn hyperedge_count_via_trait<H: HypergraphCounts>(view: &H) -> usize {
    view.hyperedge_count()
}

fn relation_size_via_trait<H: RelationIncidenceCount>(view: &H, hyperedge: H::RelationId) -> usize {
    view.relation_incidence_count(hyperedge)
}

fn element_size_via_trait<H: ElementIncidenceCount>(view: &H, vertex: H::ElementId) -> usize {
    view.element_incidence_count(vertex)
}

fn participant_size_via_trait<H: HyperedgeParticipantCount>(
    view: &H,
    hyperedge: H::RelationId,
) -> usize {
    view.hyperedge_participant_count(hyperedge)
}

fn incident_size_via_trait<H: IncidentHyperedgeCount>(view: &H, vertex: H::ElementId) -> usize {
    view.incident_hyperedge_count(vertex)
}

fn check_element_index<H>(view: &H, vertex: H::ElementId, expected: usize)
where
    H: ElementIndex,
{
    assert_eq!(view.element_index(vertex), expected);
}

fn check_relation_index<H>(view: &H, hyperedge: H::RelationId, expected: usize)
where
    H: RelationIndex,
{
    assert_eq!(view.relation_index(hyperedge), expected);
}

fn check_incidence_index<H>(view: &H, incidence: H::IncidenceId, expected: usize)
where
    H: IncidenceIndex,
{
    assert_eq!(view.incidence_index(incidence), expected);
}

fn check_incidence_resolution<H>(
    view: &H,
    incidence: H::IncidenceId,
    vertex: &H::ElementId,
    hyperedge: &H::RelationId,
    role: &H::Role,
) where
    H: IncidenceElement + IncidenceRelation + IncidenceRole,
    H::ElementId: PartialEq + core::fmt::Debug,
    H::RelationId: PartialEq + core::fmt::Debug,
    H::Role: PartialEq + core::fmt::Debug,
{
    assert_eq!(&view.incidence_element(incidence), vertex);
    assert_eq!(&view.incidence_relation(incidence), hyperedge);
    assert_eq!(&view.incidence_role(incidence), role);
}

fn assert_membership<H, V, R, I>(view: &H, vertex: V, hyperedge: R, incidence: I)
where
    H: ContainsElement<ElementId = V>
        + ContainsRelation<RelationId = R>
        + ContainsIncidence<IncidenceId = I>,
    V: Copy,
    R: Copy,
    I: Copy,
{
    assert!(view.contains_element(vertex));
    assert!(view.contains_relation(hyperedge));
    assert!(view.contains_incidence(incidence));
}

#[test]
fn generic_consumers_use_oxgraph_hyper_traits() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;

    let h0 = BcsrHyperedgeId::new(0);
    assert_eq!(
        collect_source_participants(&view, h0),
        vec![BcsrVertexId::new(0)]
    );
    assert_eq!(
        collect_target_participants(&view, h0),
        vec![BcsrVertexId::new(1), BcsrVertexId::new(2)]
    );
    assert_eq!(
        collect_hyperedge_participants(&view, h0),
        vec![
            BcsrVertexId::new(0),
            BcsrVertexId::new(1),
            BcsrVertexId::new(2)
        ]
    );

    let v2 = BcsrVertexId::new(2);
    assert_eq!(
        collect_incident(&view, v2),
        vec![BcsrHyperedgeId::new(0), BcsrHyperedgeId::new(1)]
    );
    assert!(collect_successors(&view, v2).is_empty());
    assert_eq!(
        collect_predecessors(&view, v2),
        vec![BcsrVertexId::new(0), BcsrVertexId::new(1)]
    );

    assert_eq!(count_relation_incidences(&view, h0), 3);
    Ok(())
}

#[test]
fn generic_consumers_query_dense_indexes() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    check_element_index(&view, BcsrVertexId::new(2), 2);
    check_relation_index(&view, BcsrHyperedgeId::new(1), 1);
    check_incidence_index(&view, BcsrParticipantId::new(3), 3);
    Ok(())
}

#[test]
fn generic_consumers_use_count_traits() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    assert_eq!(vertex_count_via_trait(&view), 3);
    assert_eq!(hyperedge_count_via_trait(&view), 2);
    assert_eq!(relation_size_via_trait(&view, BcsrHyperedgeId::new(0)), 3);
    assert_eq!(element_size_via_trait(&view, BcsrVertexId::new(2)), 2);
    assert_eq!(
        participant_size_via_trait(&view, BcsrHyperedgeId::new(1)),
        2
    );
    assert_eq!(incident_size_via_trait(&view, BcsrVertexId::new(1)), 2);
    Ok(())
}

#[test]
fn generic_consumers_resolve_incidences() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    check_incidence_resolution(
        &view,
        BcsrParticipantId::new(0),
        &BcsrVertexId::new(0),
        &BcsrHyperedgeId::new(0),
        &BcsrRole::Head,
    );
    check_incidence_resolution(
        &view,
        BcsrParticipantId::new(2),
        &BcsrVertexId::new(1),
        &BcsrHyperedgeId::new(0),
        &BcsrRole::Tail,
    );
    Ok(())
}

#[test]
fn generic_consumers_check_membership() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    assert_membership::<_, BcsrVertexId<u32>, BcsrHyperedgeId<u32>, BcsrParticipantId<u32>>(
        &view,
        BcsrVertexId::new(0),
        BcsrHyperedgeId::new(0),
        BcsrParticipantId::new(0),
    );
    Ok(())
}

#[test]
fn topology_base_associated_types() {
    fn check<H: TopologyBase + IncidenceBase>() {}
    check::<BcsrNativeHypergraph<'_, u32, u32, u32>>();
}
