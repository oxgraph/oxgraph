//! Topological ordering over a directed topology view.
//!
//! [`topological_sort`] runs Kahn's algorithm over the subgraph induced by the
//! caller-provided element set: edges to elements outside that set are ignored,
//! mirroring the visible-state convention used elsewhere in this crate. The
//! element set is supplied explicitly because the topology traits expose no
//! global element enumeration.

use alloc::{collections::VecDeque, vec, vec::Vec};

use oxgraph_topology::{DenseElementIndex, ElementSuccessors, TopologyBase};

/// Error returned when the requested elements cannot be totally ordered.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ToposortError {
    /// The induced subgraph over the requested elements contains a cycle, so
    /// no topological order exists.
    Cycle,
}

impl core::fmt::Display for ToposortError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cycle => formatter.write_str("topological sort found a cycle"),
        }
    }
}

impl core::error::Error for ToposortError {}

/// Returns a topological ordering of `elements` under the view's outgoing
/// connections, or [`ToposortError::Cycle`] when the induced subgraph has a
/// cycle.
///
/// Only edges whose target is also in `elements` participate. Duplicate input
/// elements are de-duplicated; the output lists each element exactly once.
/// Among elements with no remaining predecessor the order follows input order,
/// so the result is deterministic.
///
/// This function binds [`DenseElementIndex`], not containment: any element whose
/// index falls outside `graph.element_bound()` is silently ignored rather than
/// rejected, so pass identifiers that belong to `graph`.
///
/// # Errors
///
/// Returns [`ToposortError::Cycle`] when the induced subgraph cannot be fully
/// ordered (including any element with a self-loop).
///
/// # Performance
///
/// This function is `O(b + n + m)` for `b = graph.element_bound()`, `n`
/// requested elements, and `m` inspected outgoing entries; it allocates
/// `O(b + n)`.
pub fn topological_sort<G>(
    graph: &G,
    elements: &[<G as TopologyBase>::ElementId],
) -> Result<Vec<<G as TopologyBase>::ElementId>, ToposortError>
where
    G: DenseElementIndex + ElementSuccessors,
{
    let bound = graph.element_bound();
    let mut member = vec![false; bound];
    let mut nodes: Vec<G::ElementId> = Vec::new();
    for &element in elements {
        let index = graph.element_index(element);
        if index < bound && !member[index] {
            member[index] = true;
            nodes.push(element);
        }
    }

    let mut indegree = vec![0usize; bound];
    for &node in &nodes {
        for successor in graph.element_successors(node) {
            let target = graph.element_index(successor);
            if target < bound && member[target] {
                indegree[target] += 1;
            }
        }
    }

    let mut queue: VecDeque<G::ElementId> = VecDeque::new();
    for &node in &nodes {
        if indegree[graph.element_index(node)] == 0 {
            queue.push_back(node);
        }
    }

    let mut order = Vec::with_capacity(nodes.len());
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for successor in graph.element_successors(node) {
            let target = graph.element_index(successor);
            if target >= bound || !member[target] {
                continue;
            }
            indegree[target] -= 1;
            if indegree[target] == 0 {
                queue.push_back(successor);
            }
        }
    }

    if order.len() == nodes.len() {
        Ok(order)
    } else {
        Err(ToposortError::Cycle)
    }
}
