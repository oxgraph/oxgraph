//! Demonstrates traversing a borrowed bipartite-CSR hypergraph through
//! `oxgraph-hyper` traits.

use oxgraph_hyper::{
    DirectedHyperedgeParticipants, DirectedVertexSuccessors, HyperedgeParticipants,
    IncidentHyperedges,
};
use oxgraph_hyper_bcsr::{BcsrError, BcsrHyperedgeId, BcsrHypergraph, BcsrSections, BcsrVertexId};

/// Runs the example and reports validation errors.
fn main() -> Result<(), BcsrError> {
    // Three vertices, two directed hyperedges:
    //   h0:  head={v0}        tail={v1, v2}
    //   h1:  head={v1}        tail={v2}
    static HEAD_OFFSETS: &[u32] = &[0, 1, 2];
    static HEAD_PARTICIPANTS: &[u32] = &[0, 1];
    static TAIL_OFFSETS: &[u32] = &[0, 2, 3];
    static TAIL_PARTICIPANTS: &[u32] = &[1, 2, 2];
    static VERTEX_OUTGOING_OFFSETS: &[u32] = &[0, 1, 2, 2];
    static VERTEX_OUTGOING_HYPEREDGES: &[u32] = &[0, 1];
    static VERTEX_INCOMING_OFFSETS: &[u32] = &[0, 0, 1, 3];
    static VERTEX_INCOMING_HYPEREDGES: &[u32] = &[0, 0, 1];

    let view = BcsrHypergraph::open(BcsrSections {
        head_offsets: HEAD_OFFSETS,
        head_participants: HEAD_PARTICIPANTS,
        tail_offsets: TAIL_OFFSETS,
        tail_participants: TAIL_PARTICIPANTS,
        vertex_outgoing_offsets: VERTEX_OUTGOING_OFFSETS,
        vertex_outgoing_hyperedges: VERTEX_OUTGOING_HYPEREDGES,
        vertex_incoming_offsets: VERTEX_INCOMING_OFFSETS,
        vertex_incoming_hyperedges: VERTEX_INCOMING_HYPEREDGES,
    })?;

    println!("vertices={}", view.vertex_count());
    println!("hyperedges={}", view.hyperedge_count());

    let h0 = BcsrHyperedgeId(0);
    let heads: Vec<_> = view.source_participants(h0).collect();
    let tails: Vec<_> = view.target_participants(h0).collect();
    println!("h0 head={heads:?} tail={tails:?}");

    let participants: Vec<_> = view.hyperedge_participants(h0).collect();
    println!("h0 participants (head ++ tail)={participants:?}");

    let v2 = BcsrVertexId(2);
    let incident: Vec<_> = view.incident_hyperedges(v2).collect();
    println!("v2 incident hyperedges={incident:?}");

    let successors: Vec<_> = view.successor_vertices(BcsrVertexId(0)).collect();
    println!("successors of v0 (through h0's tail)={successors:?}");

    Ok(())
}
