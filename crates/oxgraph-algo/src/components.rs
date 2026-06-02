//! Weakly connected components over a topology view.
//!
//! [`connected_components`] groups the caller-provided element set by undirected
//! reachability: a directed edge connects its endpoints regardless of
//! orientation. Only edges whose target is also in the element set participate.
//! The element set is supplied explicitly because the topology traits expose no
//! global element enumeration.

use alloc::{collections::BTreeMap, vec, vec::Vec};
use core::cmp::Ordering;

use oxgraph_topology::{ElementIndex, ElementSuccessors, TopologyBase};

/// Returns the input under `cursor`'s representative root using path halving.
///
/// # Performance
///
/// This function is amortized near-`O(1)` under union by rank.
fn find(parent: &mut [usize], mut cursor: usize) -> usize {
    while parent[cursor] != cursor {
        parent[cursor] = parent[parent[cursor]];
        cursor = parent[cursor];
    }
    cursor
}

/// Unions the sets containing `left` and `right` by rank.
///
/// # Performance
///
/// This function is amortized near-`O(1)`.
fn union(parent: &mut [usize], rank: &mut [u8], left: usize, right: usize) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root == right_root {
        return;
    }
    match rank[left_root].cmp(&rank[right_root]) {
        Ordering::Less => parent[left_root] = right_root,
        Ordering::Greater => parent[right_root] = left_root,
        Ordering::Equal => {
            parent[right_root] = left_root;
            rank[left_root] += 1;
        }
    }
}

/// Groups `elements` into weakly connected components.
///
/// Duplicate input elements are de-duplicated. Each returned component lists its
/// members in input order, and components are returned in ascending
/// representative-index order, so the result is deterministic.
///
/// # Performance
///
/// This function is `O(b + n + m)` near-linear (inverse-Ackermann union-find)
/// for `b = graph.element_bound()`, `n` requested elements, and `m` inspected
/// outgoing entries; it allocates `O(b + n)`.
pub fn connected_components<G>(
    graph: &G,
    elements: &[<G as TopologyBase>::ElementId],
) -> Vec<Vec<<G as TopologyBase>::ElementId>>
where
    G: ElementIndex + ElementSuccessors,
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

    let mut parent: Vec<usize> = (0..bound).collect();
    let mut rank = vec![0u8; bound];
    for &node in &nodes {
        let source = graph.element_index(node);
        for successor in graph.element_successors(node) {
            let target = graph.element_index(successor);
            if target < bound && member[target] {
                union(&mut parent, &mut rank, source, target);
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<G::ElementId>> = BTreeMap::new();
    for &node in &nodes {
        let root = find(&mut parent, graph.element_index(node));
        groups.entry(root).or_default().push(node);
    }
    groups.into_values().collect()
}
