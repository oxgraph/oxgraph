//! Tests for borrowed bipartite-CSR validation and traversal.

use oxgraph_hyper::{
    ContainsElement, ContainsIncidence, ContainsRelation, DirectedHyperedgeParticipants,
    DirectedVertexPredecessors, DirectedVertexSuccessors, HyperedgeParticipants, IncidenceElement,
    IncidenceRelation, IncidenceRole, IncidentHyperedges,
};
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHyperedgeId, BcsrHypergraph, BcsrParticipantId, BcsrRole, BcsrSection,
    BcsrSections, BcsrValidation, BcsrVertexId,
};

/// Hand-built three-vertex, two-hyperedge fixture:
///
/// ```text
/// hyperedge h0:  head={v0}     tail={v1, v2}
/// hyperedge h1:  head={v1}     tail={v2}
/// ```
fn canonical_sections() -> BcsrSections<'static, u32> {
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

#[test]
fn canonical_fixture_validates_at_layout() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    assert_eq!(view.vertex_count(), 3);
    assert_eq!(view.hyperedge_count(), 2);
    assert_eq!(view.outgoing_incidence_count(), 2);
    assert_eq!(view.incoming_incidence_count(), 3);
    Ok(())
}

#[test]
fn directed_hyperedge_participants_split_head_and_tail() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    let h0_heads: Vec<_> = view.source_participants(BcsrHyperedgeId(0)).collect();
    let h0_tails: Vec<_> = view.target_participants(BcsrHyperedgeId(0)).collect();
    assert_eq!(h0_heads, vec![BcsrVertexId(0)]);
    assert_eq!(h0_tails, vec![BcsrVertexId(1), BcsrVertexId(2)]);

    let h1_heads: Vec<_> = view.source_participants(BcsrHyperedgeId(1)).collect();
    let h1_tails: Vec<_> = view.target_participants(BcsrHyperedgeId(1)).collect();
    assert_eq!(h1_heads, vec![BcsrVertexId(1)]);
    assert_eq!(h1_tails, vec![BcsrVertexId(2)]);
    Ok(())
}

#[test]
fn hyperedge_participants_concatenates_head_then_tail() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    let h0: Vec<_> = view.hyperedge_participants(BcsrHyperedgeId(0)).collect();
    assert_eq!(h0, vec![BcsrVertexId(0), BcsrVertexId(1), BcsrVertexId(2)]);
    Ok(())
}

#[test]
fn incident_hyperedges_yields_outgoing_then_incoming() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    let v0: Vec<_> = view.incident_hyperedges(BcsrVertexId(0)).collect();
    assert_eq!(v0, vec![BcsrHyperedgeId(0)]);

    let v1: Vec<_> = view.incident_hyperedges(BcsrVertexId(1)).collect();
    assert_eq!(v1, vec![BcsrHyperedgeId(1), BcsrHyperedgeId(0)]);

    let v2: Vec<_> = view.incident_hyperedges(BcsrVertexId(2)).collect();
    assert_eq!(v2, vec![BcsrHyperedgeId(0), BcsrHyperedgeId(1)]);
    Ok(())
}

#[test]
fn directed_successors_expand_through_tail_sets() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    let from_v0: Vec<_> = view.successor_vertices(BcsrVertexId(0)).collect();
    assert_eq!(from_v0, vec![BcsrVertexId(1), BcsrVertexId(2)]);

    let from_v1: Vec<_> = view.successor_vertices(BcsrVertexId(1)).collect();
    assert_eq!(from_v1, vec![BcsrVertexId(2)]);

    assert!(view.successor_vertices(BcsrVertexId(2)).next().is_none());
    Ok(())
}

#[test]
fn directed_predecessors_expand_through_head_sets() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    let to_v2: Vec<_> = view.predecessor_vertices(BcsrVertexId(2)).collect();
    assert_eq!(to_v2, vec![BcsrVertexId(0), BcsrVertexId(1)]);

    assert!(view.predecessor_vertices(BcsrVertexId(0)).next().is_none());
    Ok(())
}

#[test]
fn incidence_resolution_round_trips() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;

    let i0 = BcsrParticipantId(0);
    assert_eq!(view.incidence_element(i0), BcsrVertexId(0));
    assert_eq!(view.incidence_relation(i0), BcsrHyperedgeId(0));
    assert_eq!(view.incidence_role(i0), BcsrRole::Head);

    let p_head = match u32::try_from(view.outgoing_incidence_count()) {
        Ok(value) => value,
        Err(error) => panic!("outgoing count overflow: {error:?}"),
    };
    let i_first_tail = BcsrParticipantId(p_head);
    assert_eq!(view.incidence_role(i_first_tail), BcsrRole::Tail);
    assert_eq!(view.incidence_relation(i_first_tail), BcsrHyperedgeId(0));
    Ok(())
}

#[test]
fn containment_reports_in_range_ids() -> Result<(), BcsrError> {
    let view = BcsrHypergraph::open(canonical_sections())?;
    assert!(view.contains_element(BcsrVertexId(0)));
    assert!(view.contains_element(BcsrVertexId(2)));
    assert!(!view.contains_element(BcsrVertexId(3)));
    assert!(view.contains_relation(BcsrHyperedgeId(1)));
    assert!(!view.contains_relation(BcsrHyperedgeId(2)));
    assert!(view.contains_incidence(BcsrParticipantId(4)));
    assert!(!view.contains_incidence(BcsrParticipantId(5)));
    Ok(())
}

#[test]
fn empty_hypergraph_validates() -> Result<(), BcsrError> {
    let head_offsets: [u32; 1] = [0];
    let tail_offsets: [u32; 1] = [0];
    let head_participants: [u32; 0] = [];
    let tail_participants: [u32; 0] = [];
    let vertex_outgoing_offsets: [u32; 1] = [0];
    let vertex_outgoing_hyperedges: [u32; 0] = [];
    let vertex_incoming_offsets: [u32; 1] = [0];
    let vertex_incoming_hyperedges: [u32; 0] = [];
    let view = BcsrHypergraph::open(BcsrSections {
        head_offsets: &head_offsets,
        head_participants: &head_participants,
        tail_offsets: &tail_offsets,
        tail_participants: &tail_participants,
        vertex_outgoing_offsets: &vertex_outgoing_offsets,
        vertex_outgoing_hyperedges: &vertex_outgoing_hyperedges,
        vertex_incoming_offsets: &vertex_incoming_offsets,
        vertex_incoming_hyperedges: &vertex_incoming_hyperedges,
    })?;
    assert_eq!(view.vertex_count(), 0);
    assert_eq!(view.hyperedge_count(), 0);
    Ok(())
}

/// Truncated head offsets that disagree with `tail_offsets` on `hyperedge_count`.
static SHORT_HEAD_OFFSETS: &[u32] = &[0, 1];

/// Non-monotonic head offsets ([0, 5, 2] — offset[2] < offset[1]).
static BAD_HEAD_OFFSETS: &[u32] = &[0, 5, 2];

/// `head_participants` with a vertex ID outside `0..vertex_count`.
static BAD_HEAD_PARTICIPANTS: &[u32] = &[99, 1];

/// Tampered vertex-major outgoing array used to exercise the Strict-only
/// cross-direction check. Vertex 0's bucket points at `h1` instead of `h0`,
/// which is layout-valid but semantically inconsistent.
static TAMPERED_OUTGOING_HYPEREDGES: &[u32] = &[1, 1];

#[test]
fn rejects_offset_length_mismatch() {
    let mut sections = canonical_sections();
    sections.head_offsets = SHORT_HEAD_OFFSETS;
    let result = BcsrHypergraph::open(sections);
    let Err(BcsrError::HyperedgeOffsetLengthMismatch { .. }) = result else {
        panic!("expected HyperedgeOffsetLengthMismatch, got {result:?}");
    };
}

#[test]
fn rejects_non_monotonic_offsets() {
    let mut sections = canonical_sections();
    sections.head_offsets = BAD_HEAD_OFFSETS;
    let result = BcsrHypergraph::open(sections);
    let Err(BcsrError::NonMonotonicOffset { section, .. }) = result else {
        panic!("expected NonMonotonicOffset rejection, got {result:?}");
    };
    assert_eq!(section, BcsrSection::HeadOffsets);
}

#[test]
fn rejects_vertex_out_of_range() {
    let mut sections = canonical_sections();
    sections.head_participants = BAD_HEAD_PARTICIPANTS;
    let result = BcsrHypergraph::open(sections);
    let Err(BcsrError::VertexOutOfRange { vertex, .. }) = result else {
        panic!("expected VertexOutOfRange, got {result:?}");
    };
    assert_eq!(vertex, 99);
}

#[test]
fn strict_validation_catches_cross_direction_mismatch() -> Result<(), BcsrError> {
    let mut sections = canonical_sections();
    sections.vertex_outgoing_hyperedges = TAMPERED_OUTGOING_HYPEREDGES;
    BcsrHypergraph::open(sections)?;
    let result = BcsrHypergraph::open_with(sections, BcsrValidation::Strict);
    let Err(BcsrError::CrossDirectionMismatch { .. }) = result else {
        panic!("expected CrossDirectionMismatch, got {result:?}");
    };
    Ok(())
}
