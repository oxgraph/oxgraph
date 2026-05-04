//! Kani proof harnesses for the CSR view.
//!
//! Each `#[kani::proof]` exercises a bounded scenario the view must never
//! violate: validation must be total (no panic on arbitrary input), and a
//! validated view must keep every traversal index in bounds.
//!
//! These proofs run under `cargo kani` (heavy gate, not in `just ci`).

#![cfg(kani)]

use oxgraph_graph::{ElementSuccessors, OutgoingGraph};

use crate::{CsrGraph, CsrNodeId};

/// CSR validation must be total — return `Ok` or a typed `CsrError`, never
/// panic — for any combination of node count, offsets, and targets up to
/// `N = 3` nodes and `M = 4` edges.
///
/// The offsets slice length is non-deterministic in `[0, 4]`, so the
/// proof exercises the wrong-length path ([`CsrError::OffsetLength`]) as
/// well as well-formed inputs. Validation must remain panic-free in both
/// regimes.
#[kani::proof]
#[kani::unwind(8)]
fn validate_total_n3_m4() {
    let node_count: u32 = kani::any();
    kani::assume(node_count <= 3);

    let offsets_storage: [u32; 4] = kani::any();
    let targets: [u32; 4] = kani::any();

    let offsets_take: usize = kani::any();
    kani::assume(offsets_take <= offsets_storage.len());
    let offsets_slice = &offsets_storage[..offsets_take];

    let _ = CsrGraph::<u32>::validate(node_count, offsets_slice, &targets);
}

/// A successfully validated CSR view never produces an out-of-bounds target
/// slice access during outgoing traversal. For any node id in `[0, node_count)`,
/// `element_successors` yields targets entirely from inside the validated
/// `targets` slice (`start <= end <= targets.len()`).
///
/// Proven for `N = 2` nodes, `M = 2` edges. The bound is tight enough for kani
/// while wide enough to exercise both empty-row and non-empty-row outgoing
/// ranges plus monotonic-offset edge cases.
#[kani::proof]
#[kani::unwind(8)]
fn validated_traversal_in_bounds_n2_m2() {
    // Two nodes, up to two edges.
    let offsets: [u32; 3] = kani::any();
    let targets: [u32; 2] = kani::any();

    let graph = match CsrGraph::<u32>::validate(2, &offsets, &targets) {
        Ok(value) => value,
        Err(_) => return,
    };

    // For each valid node id, traverse and accumulate a checksum. The act of
    // calling `element_successors` exercises `targets[start..end]`; if any
    // index were out of bounds, the slice would panic and kani would catch it.
    for node_index in 0..2u32 {
        let node = CsrNodeId(node_index);
        let mut count = 0u32;
        for _target in graph.element_successors(node) {
            count = count.wrapping_add(1);
        }
        // out_degree must equal the iterator count, with both <= 2.
        assert!(count <= 2);
        let edges_iter_count: u32 = graph.outgoing_edges(node).count() as u32;
        assert_eq!(count, edges_iter_count);
    }
}

/// A successfully validated CSR view's `final_offset == targets.len()` is
/// load-bearing for the bounds proof above. This proof certifies that the
/// validation predicate enforces it for all accepted inputs at `N = 3`.
#[kani::proof]
#[kani::unwind(8)]
fn validated_final_offset_matches_targets_len_n3() {
    let offsets: [u32; 4] = kani::any();
    let targets: [u32; 3] = kani::any();

    let graph = match CsrGraph::<u32>::validate(3, &offsets, &targets) {
        Ok(value) => value,
        Err(_) => return,
    };

    let final_offset = graph.offsets()[graph.offsets().len() - 1];
    assert_eq!(final_offset as usize, graph.targets().len());
}
