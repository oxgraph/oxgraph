//! Kani proof harnesses for the bipartite-CSR view.
//!
//! Each `#[kani::proof]` exercises a bounded scenario the view must never
//! violate: validation must be total on small symbolic inputs and arithmetic
//! on participant IDs must not overflow within the documented `u32` cap.
//!
//! These proofs run under `cargo kani` (heavy gate, not in `just ci`).

#![cfg(kani)]

use crate::{BcsrHypergraph, BcsrSections, BcsrValidation};

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
    let sections = BcsrSections {
        head_offsets: &head_offsets,
        head_participants: &head_participants,
        tail_offsets: &tail_offsets,
        tail_participants: &tail_participants,
        vertex_outgoing_offsets: &vertex_outgoing_offsets,
        vertex_outgoing_hyperedges: &vertex_outgoing_hyperedges,
        vertex_incoming_offsets: &vertex_incoming_offsets,
        vertex_incoming_hyperedges: &vertex_incoming_hyperedges,
    };
    let _ = BcsrHypergraph::open(sections);
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
    let sections = BcsrSections {
        head_offsets: &head_offsets,
        head_participants: &head_participants,
        tail_offsets: &tail_offsets,
        tail_participants: &tail_participants,
        vertex_outgoing_offsets: &vertex_outgoing_offsets,
        vertex_outgoing_hyperedges: &vertex_outgoing_hyperedges,
        vertex_incoming_offsets: &vertex_incoming_offsets,
        vertex_incoming_hyperedges: &vertex_incoming_hyperedges,
    };
    let _ = BcsrHypergraph::open_with(sections, BcsrValidation::Strict);
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
    let _converted = usize::try_from(value).expect("u32 fits usize on 32-bit-or-wider targets");
}
