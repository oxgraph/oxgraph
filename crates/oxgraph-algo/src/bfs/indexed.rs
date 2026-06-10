//! Allocating dense-indexed BFS (forward and reverse).

use core::iter::FusedIterator;

use oxgraph_topology::{
    ContainsElement, DenseElementIndex, ElementId, ElementPredecessors, ElementSuccessors,
};

use crate::bfs::{
    BfsError,
    core::{Bfs, Forward, Reverse, SeededOwnedFrontier, ValidatedStart},
};

/// Forward BFS iterator using owned dense scratch storage.
///
/// Constructed only via [`breadth_first_search`]. The internal storage policy
/// ([`SeededOwnedFrontier`]) is intentionally not part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements, `m` successor entries
/// inspected, and element index bound `b`, traversal is `O(n + m)` and memory
/// usage is `O(b + n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearch<'graph, G>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededOwnedFrontier<G>, Forward>,
}

impl<G> Iterator for BreadthFirstSearch<'_, G>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearch<'_, G> where
    G: ContainsElement + ElementSuccessors + DenseElementIndex
{
}

/// Creates an allocating forward breadth-first traversal using dense element
/// indexes.
///
/// Traversal follows outgoing connections and yields each reachable element
/// at most once. The start element is yielded first. This is the convenience
/// path for topology views that can map element IDs to dense scratch-storage
/// indexes.
///
/// `start` and all IDs returned by `graph.element_successors` must be valid
/// for `graph` and must map to indexes below `graph.element_bound()`. An
/// expansion-target index at or past the bound observed at construction time
/// will panic with [`BfsError::NeighborIndexOutOfBounds`]; this is a topology
/// contract violation, not a recoverable failure.
///
/// # Errors
///
/// Returns [`BfsError::StartElementNotContained`] when `start` is not
/// contained in `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start`
/// maps outside `graph.element_bound()`. Allocation failure is not
/// represented by this error and follows the allocator's behavior.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements, `m` successor entries
/// inspected, and element index bound `b`, construction allocates `O(b)`
/// visited storage and frontier capacity. Traversal is `O(n + m)`. Memory
/// usage is `O(b + n)`.
pub fn breadth_first_search<G>(
    graph: &G,
    start: ElementId<G>,
) -> Result<BreadthFirstSearch<'_, G>, BfsError>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    let witness = ValidatedStart::new(graph, start)?;
    let frontier = SeededOwnedFrontier::new(start, witness);
    Ok(BreadthFirstSearch {
        inner: Bfs::new(graph, frontier),
    })
}

/// Reverse BFS iterator using owned dense scratch storage.
///
/// Constructed only via [`reverse_breadth_first_search`]. The internal
/// storage policy ([`SeededOwnedFrontier`]) is intentionally not part of the
/// public API.
///
/// # Performance
///
/// For a reverse-reachable subgraph with `n` elements, `m` predecessor
/// entries inspected, and element index bound `b`, traversal is `O(n + m)`
/// and memory usage is `O(b + n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct ReverseBreadthFirstSearch<'graph, G>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier and reverse
    /// direction selector.
    inner: Bfs<'graph, G, SeededOwnedFrontier<G>, Reverse>,
}

impl<G> Iterator for ReverseBreadthFirstSearch<'_, G>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for ReverseBreadthFirstSearch<'_, G> where
    G: ContainsElement + ElementPredecessors + DenseElementIndex
{
}

/// Creates an allocating reverse breadth-first traversal using dense element
/// indexes.
///
/// Traversal follows incoming connections and yields each reverse-reachable
/// element at most once. Same allocation / contract / error shape as
/// [`breadth_first_search`]; the only difference is that expansion uses
/// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors) instead of
/// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors).
///
/// # Errors
///
/// Same as [`breadth_first_search`].
///
/// # Performance
///
/// Same as [`breadth_first_search`] with predecessor expansion in place of
/// successor expansion.
pub fn reverse_breadth_first_search<G>(
    graph: &G,
    start: ElementId<G>,
) -> Result<ReverseBreadthFirstSearch<'_, G>, BfsError>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    let witness = ValidatedStart::new(graph, start)?;
    let frontier = SeededOwnedFrontier::new(start, witness);
    Ok(ReverseBreadthFirstSearch {
        inner: Bfs::new(graph, frontier),
    })
}
