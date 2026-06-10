//! Longest path through a directed acyclic topology view.
//!
//! [`longest_path_dag`] returns the longest simple chain of elements reachable
//! along outgoing connections within the subgraph induced by the
//! caller-provided element set, measured by edge count. Edges to elements
//! outside that set are ignored, mirroring the visible-state convention used
//! elsewhere in this crate. The element set is supplied explicitly because the
//! topology traits expose no global element enumeration.
//!
//! The computation reuses [`topological_sort`](crate::topological_sort): a
//! valid topological order proves the induced subgraph is acyclic and lets a
//! single forward pass relax each element's best incoming chain length, after
//! which the longest chain is reconstructed from a predecessor table.

use alloc::{vec, vec::Vec};

use oxgraph_topology::{DenseElementIndex, ElementSuccessors, TopologyBase};

use crate::topological_sort;

/// Error returned when the longest path cannot be computed.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LongestPathError {
    /// The induced subgraph over the requested elements contains a cycle, so a
    /// longest *simple* path is undefined.
    Cycle,
}

impl core::fmt::Display for LongestPathError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cycle => formatter.write_str("longest path found a cycle"),
        }
    }
}

impl core::error::Error for LongestPathError {}

/// Returns the longest chain of elements along outgoing connections within the
/// subgraph induced by `elements`, or [`LongestPathError::Cycle`] when that
/// subgraph contains a cycle.
///
/// Only edges whose target is also in `elements` participate. The returned path
/// lists each element once, from the chain's start to its end; its length in
/// edges is `path.len() - 1`. An empty `elements` slice yields an empty path; a
/// single element yields a one-element path.
///
/// Ties are broken deterministically by topological order: among chains of
/// equal length the one whose end appears earliest in the topological order is
/// returned, and each element's recorded predecessor is the first source node,
/// in topological processing order, to establish that element's best length —
/// later sources that merely tie do not overwrite it (the relaxation uses a
/// strict `<`).
///
/// # Errors
///
/// Returns [`LongestPathError::Cycle`] when the induced subgraph cannot be
/// topologically ordered (including any element with a self-loop).
///
/// # Performance
///
/// This function is `O(b + n + m)` for `b = graph.element_bound()`, `n`
/// requested elements, and `m` inspected outgoing entries; it allocates
/// `O(b + n)`.
pub fn longest_path_dag<G>(
    graph: &G,
    elements: &[<G as TopologyBase>::ElementId],
) -> Result<Vec<<G as TopologyBase>::ElementId>, LongestPathError>
where
    G: DenseElementIndex + ElementSuccessors,
{
    let order = topological_sort(graph, elements).map_err(|_| LongestPathError::Cycle)?;
    if order.is_empty() {
        return Ok(Vec::new());
    }

    let bound = graph.element_bound();
    // `lengths[i]` is the longest chain (in edges) ending at element index `i`;
    // `predecessor[i]` is the element that precedes it on that chain. Successors
    // outside the induced set receive dead relaxations that never reach
    // `best_end` (only the member-only `order` returned by `topological_sort` is
    // considered, and only those nodes propagate), so no explicit membership
    // filter is needed in this pass.
    let mut lengths = vec![0usize; bound];
    let mut predecessor: Vec<Option<<G as TopologyBase>::ElementId>> = vec![None; bound];
    let mut best_end: Option<<G as TopologyBase>::ElementId> = None;
    let mut best_length = 0usize;

    for &node in &order {
        let node_index = graph.element_index(node);
        let node_length = lengths[node_index];
        if best_end.is_none() || node_length > best_length {
            best_length = node_length;
            best_end = Some(node);
        }
        for successor in graph.element_successors(node) {
            let target = graph.element_index(successor);
            if target < bound && lengths[target] < node_length + 1 {
                lengths[target] = node_length + 1;
                predecessor[target] = Some(node);
            }
        }
    }

    let mut path = Vec::with_capacity(best_length + 1);
    let mut cursor = best_end;
    while let Some(node) = cursor {
        path.push(node);
        cursor = predecessor[graph.element_index(node)];
    }
    path.reverse();
    Ok(path)
}
