//! Kani proof harnesses for the bipartite-CSR view.
//!
//! Each `#[kani::proof]` exercises a bounded scenario the view must never
//! violate: validation must be total on small symbolic inputs and arithmetic
//! on participant IDs must not overflow within the documented `u32` cap.
//!
//! These proofs run under `cargo kani` (heavy gate, not in `just ci`).

#![cfg(kani)]

use crate::{BcsrNativeHypergraph, BcsrSections, BcsrValidation};

/// `BcsrHypergraph::open` over a tiny symbolic input must return `Result` and
/// never panic at [`BcsrValidation::Layout`].
#[kani::proof]
#[kani::unwind(4)]
fn validate_layout_total() {
    let head_offsets: [u32; 2] = kani::any();
    let tail_offsets: [u32; 2] = kani::any();
    let head_participants: [u32; 1] = kani::any();
    let tail_participants: [u32; 1] = kani::any();
    let vertex_outgoing_offsets: [u32; 2] = kani::any();
    let vertex_outgoing_hyperedges: [u32; 1] = kani::any();
    let vertex_incoming_offsets: [u32; 2] = kani::any();
    let vertex_incoming_hyperedges: [u32; 1] = kani::any();
    let sections = BcsrSections::for_kani(
        &head_offsets,
        &head_participants,
        &tail_offsets,
        &tail_participants,
        &vertex_outgoing_offsets,
        &vertex_outgoing_hyperedges,
        &vertex_incoming_offsets,
        &vertex_incoming_hyperedges,
    );
    let _ = BcsrNativeHypergraph::<u32, u32, u32>::open(sections);
}

/// `BcsrHypergraph::open_with(Strict)` is also total on tiny symbolic inputs.
#[kani::proof]
#[kani::unwind(4)]
fn validate_strict_total() {
    let head_offsets: [u32; 2] = kani::any();
    let tail_offsets: [u32; 2] = kani::any();
    let head_participants: [u32; 1] = kani::any();
    let tail_participants: [u32; 1] = kani::any();
    let vertex_outgoing_offsets: [u32; 2] = kani::any();
    let vertex_outgoing_hyperedges: [u32; 1] = kani::any();
    let vertex_incoming_offsets: [u32; 2] = kani::any();
    let vertex_incoming_hyperedges: [u32; 1] = kani::any();
    let sections = BcsrSections::for_kani(
        &head_offsets,
        &head_participants,
        &tail_offsets,
        &tail_participants,
        &vertex_outgoing_offsets,
        &vertex_outgoing_hyperedges,
        &vertex_incoming_offsets,
        &vertex_incoming_hyperedges,
    );
    let _ = BcsrNativeHypergraph::<u32, u32, u32>::open_with(sections, BcsrValidation::Strict);
}

/// Participant-ID arithmetic must not overflow within the documented cap:
/// for any `(p_head, p_tail)` whose sum fits in `u32`, every position
/// `i ∈ [0, p_head + p_tail)` is reachable as a `u32`.
#[kani::proof]
fn participant_id_arithmetic_no_overflow() {
    let p_head: u32 = kani::any();
    let p_tail: u32 = kani::any();
    let total = p_head.checked_add(p_tail);
    if let Some(total) = total {
        kani::assume(total > 0);
        let i: u32 = kani::any();
        kani::assume(i < total);
        if i >= p_head {
            let local = i - p_head;
            assert!(local < p_tail);
        } else {
            assert!(i < p_head);
        }
    }
}

/// `u32 -> usize` narrowing must be lossless on every supported target
/// (`usize::BITS >= 32`). The compile-time bound below makes the contract
/// explicit; if a future target weakens it, this assertion fails to compile.
#[kani::proof]
fn u32_to_usize_safe_on_target_platforms() {
    const _: () = assert!(usize::BITS >= 32);
    let value: u32 = kani::any();
    let _converted = match usize::try_from(value) {
        Ok(converted) => converted,
        Err(_) => unreachable!(),
    };
}

/// **Layout-implies-bounds**: a `BcsrHypergraph` returned from
/// [`BcsrHypergraph::open`] (default `Layout` validation) must keep every
/// public traversal call within the validated participant slices.
///
/// Proven for two hyperedges and two vertices with up to two participants
/// per hyperedge per side, across every public traversal entry point:
/// `hyperedge_participants`, `source_participants`, `target_participants`,
/// `incident_hyperedges`, `element_successors`, and `element_predecessors`.
/// If any traversal slice-accessed an out-of-range index, kani would catch
/// the panic.
#[kani::proof]
#[kani::unwind(8)]
fn validated_traversal_in_bounds_h2_v2() {
    use oxgraph_hyper::{
        DirectedHyperedgeParticipants, ElementPredecessors, ElementSuccessors,
        HyperedgeParticipants, IncidentHyperedges,
    };

    use crate::{BcsrHyperedgeId, BcsrVertexId};

    let head_offsets: [u32; 3] = kani::any();
    let tail_offsets: [u32; 3] = kani::any();
    let head_participants: [u32; 2] = kani::any();
    let tail_participants: [u32; 2] = kani::any();
    let vertex_outgoing_offsets: [u32; 3] = kani::any();
    let vertex_outgoing_hyperedges: [u32; 2] = kani::any();
    let vertex_incoming_offsets: [u32; 3] = kani::any();
    let vertex_incoming_hyperedges: [u32; 2] = kani::any();
    let sections = BcsrSections::for_kani(
        &head_offsets,
        &head_participants,
        &tail_offsets,
        &tail_participants,
        &vertex_outgoing_offsets,
        &vertex_outgoing_hyperedges,
        &vertex_incoming_offsets,
        &vertex_incoming_hyperedges,
    );

    let view = match BcsrNativeHypergraph::<u32, u32, u32>::open(sections) {
        Ok(value) => value,
        Err(_) => return,
    };

    // Hyperedge-side: walk participants for every hyperedge.
    for h in 0..2u32 {
        let hid = BcsrHyperedgeId(h);
        let mut count = 0u32;
        for _ in view.hyperedge_participants(hid) {
            count = count.wrapping_add(1);
        }
        let _ = count;
        let mut count = 0u32;
        for _ in view.source_participants(hid) {
            count = count.wrapping_add(1);
        }
        let _ = count;
        let mut count = 0u32;
        for _ in view.target_participants(hid) {
            count = count.wrapping_add(1);
        }
        let _ = count;
    }

    // Vertex-side: walk incident hyperedges and successor/predecessor vertices.
    for v in 0..2u32 {
        let vid = BcsrVertexId(v);
        let mut count = 0u32;
        for _ in view.incident_hyperedges(vid) {
            count = count.wrapping_add(1);
        }
        let _ = count;
        let mut count = 0u32;
        for _ in view.element_successors(vid) {
            count = count.wrapping_add(1);
        }
        let _ = count;
        let mut count = 0u32;
        for _ in view.element_predecessors(vid) {
            count = count.wrapping_add(1);
        }
        let _ = count;
    }
}

/// **Strict-implies-count-symmetry**: a view accepted at
/// [`BcsrValidation::Strict`] must report equal totals when participant
/// counts are summed across all hyperedges from the hyperedge side and
/// across all vertices from the vertex side.
///
/// At Strict, validation enforces that the multiset of `(vertex, hyperedge)`
/// pairs reachable via head/tail participants equals the multiset reachable
/// via vertex-outgoing / vertex-incoming hyperedges. This proof certifies
/// the projection on totals — the load-bearing consequence used by BFS and
/// any other algorithm that traverses both directions. With `H = V = 2`,
/// the totals are non-trivially split across multiple buckets, so the proof
/// rules out per-bucket accounting bugs that an `H = V = 1` smoke proof
/// would miss.
#[kani::proof]
#[kani::unwind(8)]
fn strict_implies_count_symmetry_h2_v2() {
    use oxgraph_hyper::{HyperedgeParticipants, IncidentHyperedges};

    use crate::{BcsrHyperedgeId, BcsrVertexId};

    let head_offsets: [u32; 3] = kani::any();
    let tail_offsets: [u32; 3] = kani::any();
    let head_participants: [u32; 2] = kani::any();
    let tail_participants: [u32; 2] = kani::any();
    let vertex_outgoing_offsets: [u32; 3] = kani::any();
    let vertex_outgoing_hyperedges: [u32; 2] = kani::any();
    let vertex_incoming_offsets: [u32; 3] = kani::any();
    let vertex_incoming_hyperedges: [u32; 2] = kani::any();
    let sections = BcsrSections::for_kani(
        &head_offsets,
        &head_participants,
        &tail_offsets,
        &tail_participants,
        &vertex_outgoing_offsets,
        &vertex_outgoing_hyperedges,
        &vertex_incoming_offsets,
        &vertex_incoming_hyperedges,
    );

    let view =
        match BcsrNativeHypergraph::<u32, u32, u32>::open_with(sections, BcsrValidation::Strict) {
            Ok(value) => value,
            Err(_) => return,
        };

    // Total participants counted across all hyperedges (heads + tails).
    let mut hyperedge_side_total = 0u32;
    for h in 0..2u32 {
        for _ in view.hyperedge_participants(BcsrHyperedgeId(h)) {
            hyperedge_side_total = hyperedge_side_total.wrapping_add(1);
        }
    }

    // Total incidences counted across all vertices (incoming + outgoing).
    let mut vertex_side_total = 0u32;
    for v in 0..2u32 {
        for _ in view.incident_hyperedges(BcsrVertexId(v)) {
            vertex_side_total = vertex_side_total.wrapping_add(1);
        }
    }

    // Strict accepted ⇒ head/tail multisets equal outgoing/incoming
    // multisets ⇒ totals agree across the two directions.
    assert_eq!(hyperedge_side_total, vertex_side_total);
}
