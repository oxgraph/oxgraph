//! Standard-library arbitrary-ID BFS using a `HashSet` visited set
//! (forward and reverse).

use core::iter::FusedIterator;
use std::collections::HashSet;

use oxgraph_topology::{ElementId, ElementPredecessors, ElementSuccessors, TopologyId};

use crate::bfs::core::{Bfs, Forward, GenericState, Reverse, VisitedSet};

impl<T> VisitedSet<T> for HashSet<T>
where
    T: TopologyId,
{
    fn insert(&mut self, value: T) -> bool {
        Self::insert(self, value)
    }
}

/// `HashSet`-backed driver state for the generic BFS variants.
type HashBfs<'graph, G, D> = Bfs<'graph, G, GenericState<G, HashSet<ElementId<G>>>, D>;

/// Forward BFS iterator for arbitrary element ID spaces using a `HashSet`.
///
/// Constructed only via [`breadth_first_search_generic_hash`]. The internal
/// storage policy is intentionally not part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is expected `O(n + m)` and memory usage is `O(n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct HashBreadthFirstSearch<'graph, G>
where
    G: ElementSuccessors,
{
    /// Underlying generic BFS driver carrying the state policy.
    inner: HashBfs<'graph, G, Forward>,
}

impl<G> Iterator for HashBreadthFirstSearch<'_, G>
where
    G: ElementSuccessors,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for HashBreadthFirstSearch<'_, G> where G: ElementSuccessors {}

/// Creates a `std`-optimized forward BFS for arbitrary element ID spaces.
///
/// Traversal follows outgoing connections and yields each reachable element
/// at most once. The start element is yielded first. This fallback does not
/// require dense element indexes and uses `HashSet` membership tracking when
/// `std` is available.
///
/// `start` and all IDs returned by `graph.element_successors` must be valid
/// for `graph`.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is expected `O(n + m)` with hash-table membership
/// checks. Memory usage is `O(n)`.
pub fn breadth_first_search_generic_hash<G>(
    graph: &G,
    start: ElementId<G>,
) -> HashBreadthFirstSearch<'_, G>
where
    G: ElementSuccessors,
{
    HashBreadthFirstSearch {
        inner: Bfs::new(graph, GenericState::new(start)),
    }
}

/// Reverse BFS iterator for arbitrary element ID spaces using a `HashSet`.
///
/// Constructed only via [`reverse_breadth_first_search_generic_hash`]. The
/// internal storage policy is intentionally not part of the public API.
///
/// # Performance
///
/// For a reverse-reachable subgraph with `n` elements and `m` predecessor
/// entries inspected, traversal is expected `O(n + m)` and memory usage is
/// `O(n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct HashReverseBreadthFirstSearch<'graph, G>
where
    G: ElementPredecessors,
{
    /// Underlying generic BFS driver carrying the state policy and reverse
    /// direction selector.
    inner: HashBfs<'graph, G, Reverse>,
}

impl<G> Iterator for HashReverseBreadthFirstSearch<'_, G>
where
    G: ElementPredecessors,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for HashReverseBreadthFirstSearch<'_, G> where G: ElementPredecessors {}

/// Creates a `std`-optimized reverse BFS for arbitrary element ID spaces.
///
/// Traversal follows incoming connections and yields each reverse-reachable
/// element at most once. Same fallback / contract / performance shape as
/// [`breadth_first_search_generic_hash`]; the only difference is that
/// expansion uses
/// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors) instead of
/// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors).
///
/// # Performance
///
/// Same as [`breadth_first_search_generic_hash`] with predecessor expansion in
/// place of successor expansion.
pub fn reverse_breadth_first_search_generic_hash<G>(
    graph: &G,
    start: ElementId<G>,
) -> HashReverseBreadthFirstSearch<'_, G>
where
    G: ElementPredecessors,
{
    HashReverseBreadthFirstSearch {
        inner: Bfs::new(graph, GenericState::new(start)),
    }
}
