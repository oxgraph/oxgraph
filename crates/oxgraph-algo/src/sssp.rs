//! Single-source shortest path lengths over an unweighted topology view.
//!
//! [`shortest_path_lengths`] runs a breadth-first layering from a source over
//! the subgraph induced by the caller-provided element set, returning the hop
//! distance to every reachable member. Only edges whose target is also in that
//! set participate. The element set is supplied explicitly because the topology
//! traits expose no global element enumeration. Weighted shortest paths
//! (Dijkstra over a selected `RelationWeight`) are a separate future entry
//! point.

use alloc::{collections::VecDeque, vec, vec::Vec};

use oxgraph_topology::{DenseElementIndex, ElementSuccessors, TopologyBase};

/// Returns the hop distance from `source` to every reachable member of
/// `elements`, including `source` itself at distance `0`.
///
/// Returns an empty result when `source` is not one of `elements`. Results are
/// in breadth-first discovery order (non-decreasing distance), so the output is
/// deterministic.
///
/// # Performance
///
/// This function is `O(b + n + m)` for `b = graph.element_bound()`, `n`
/// requested elements, and `m` inspected outgoing entries; it allocates
/// `O(b + n)`.
pub fn shortest_path_lengths<G>(
    graph: &G,
    source: <G as TopologyBase>::ElementId,
    elements: &[<G as TopologyBase>::ElementId],
) -> Vec<(<G as TopologyBase>::ElementId, u64)>
where
    G: DenseElementIndex + ElementSuccessors,
{
    let bound = graph.element_bound();
    let mut member = vec![false; bound];
    for &element in elements {
        let index = graph.element_index(element);
        if index < bound {
            member[index] = true;
        }
    }

    let source_index = graph.element_index(source);
    if source_index >= bound || !member[source_index] {
        return Vec::new();
    }

    let mut visited = vec![false; bound];
    let mut distances: Vec<(G::ElementId, u64)> = Vec::new();
    let mut queue: VecDeque<(G::ElementId, u64)> = VecDeque::new();
    visited[source_index] = true;
    queue.push_back((source, 0));
    while let Some((node, distance)) = queue.pop_front() {
        distances.push((node, distance));
        for successor in graph.element_successors(node) {
            let target = graph.element_index(successor);
            if target < bound && member[target] && !visited[target] {
                visited[target] = true;
                queue.push_back((successor, distance + 1));
            }
        }
    }
    distances
}
