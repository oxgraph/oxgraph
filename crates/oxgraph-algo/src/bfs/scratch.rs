//! Scratch-backed no-allocation indexed BFS (forward and reverse).

use core::iter::FusedIterator;

use oxgraph_topology::{
    ContainsElement, DenseElementIndex, ElementId, ElementPredecessors, ElementSuccessors,
};

use crate::bfs::{
    BfsError,
    core::{Bfs, Forward, Reverse, SeededByteFlagFrontier, ValidatedScratch},
};

/// Forward BFS iterator using caller-provided byte-flag scratch.
///
/// Constructed only via [`breadth_first_search_with_scratch`]. The internal
/// storage policy ([`SeededByteFlagFrontier`]) is intentionally not part of
/// the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is `O(n + m)` and uses `O(b)` caller-provided scratch
/// memory for `b = graph.element_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearchScratch<'graph, 'scratch, G>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededByteFlagFrontier<'scratch, G>, Forward>,
}

impl<G> Iterator for BreadthFirstSearchScratch<'_, '_, G>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearchScratch<'_, '_, G> where
    G: ContainsElement + ElementSuccessors + DenseElementIndex
{
}

/// Creates a forward breadth-first traversal using caller-provided scratch.
///
/// Traversal follows outgoing connections and yields each reachable element at
/// most once. The start element is yielded first. `visited` and `queue` must
/// each have at least `graph.element_bound()` entries. The scratch slices are
/// cleared and reused by the traversal; their previous contents are ignored.
///
/// `start` and all IDs returned by `graph.element_successors` must be valid
/// for `graph` and must map to indexes below `graph.element_bound()`. An
/// expansion-target index at or past the bound observed at construction time
/// will panic with [`BfsError::NeighborIndexOutOfBounds`]; this is a topology
/// contract violation, not a recoverable failure.
///
/// # Errors
///
/// Returns [`BfsError::VisitedTooSmall`] or [`BfsError::QueueTooSmall`] when
/// the corresponding scratch slice is smaller than `graph.element_bound()`,
/// [`BfsError::StartElementNotContained`] when `start` is not contained in
/// `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start` maps outside
/// `graph.element_bound()`. Errors are returned in that order: scratch sizes
/// are checked before start-element validation.
///
/// # Performance
///
/// Construction clears `O(b)` visited entries for `b = graph.element_bound()`
/// and performs no heap allocation. Traversal over a reachable subgraph with
/// `n` elements and `m` successor entries inspected is `O(n + m)` and performs
/// no heap allocation. Reusing scratch across traversals reuses the same
/// memory.
pub fn breadth_first_search_with_scratch<'graph, 'scratch, G>(
    graph: &'graph G,
    start: ElementId<G>,
    visited: &'scratch mut [u8],
    queue: &'scratch mut [ElementId<G>],
) -> Result<BreadthFirstSearchScratch<'graph, 'scratch, G>, BfsError>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    let witness = ValidatedScratch::new(graph, start, visited.len(), queue.len())?;
    let frontier = SeededByteFlagFrontier::new(visited, queue, start, witness);
    Ok(BreadthFirstSearchScratch {
        inner: Bfs::new(graph, frontier),
    })
}

/// Reverse BFS iterator using caller-provided byte-flag scratch.
///
/// Constructed only via [`reverse_breadth_first_search_with_scratch`]. The
/// internal storage policy ([`SeededByteFlagFrontier`]) is intentionally not
/// part of the public API.
///
/// # Performance
///
/// For a reverse-reachable subgraph with `n` elements and `m` predecessor
/// entries inspected, traversal is `O(n + m)` and uses `O(b)` caller-provided
/// scratch memory for `b = graph.element_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct ReverseBreadthFirstSearchScratch<'graph, 'scratch, G>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier and reverse
    /// direction selector.
    inner: Bfs<'graph, G, SeededByteFlagFrontier<'scratch, G>, Reverse>,
}

impl<G> Iterator for ReverseBreadthFirstSearchScratch<'_, '_, G>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for ReverseBreadthFirstSearchScratch<'_, '_, G> where
    G: ContainsElement + ElementPredecessors + DenseElementIndex
{
}

/// Creates a reverse breadth-first traversal using caller-provided scratch.
///
/// Traversal follows incoming connections and yields each reverse-reachable
/// element at most once. The start element is yielded first. `visited` and
/// `queue` must each have at least `graph.element_bound()` entries. The
/// scratch slices are cleared and reused by the traversal; their previous
/// contents are ignored.
///
/// `start` and all IDs returned by `graph.element_predecessors` must be valid
/// for `graph` and must map to indexes below `graph.element_bound()`. An
/// expansion-target index at or past the bound observed at construction time
/// will panic with [`BfsError::NeighborIndexOutOfBounds`]; this is a topology
/// contract violation, not a recoverable failure.
///
/// # Errors
///
/// Returns [`BfsError::VisitedTooSmall`] or [`BfsError::QueueTooSmall`] when
/// the corresponding scratch slice is smaller than `graph.element_bound()`,
/// [`BfsError::StartElementNotContained`] when `start` is not contained in
/// `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start` maps outside
/// `graph.element_bound()`. Errors are returned in that order.
///
/// # Performance
///
/// Same shape as [`breadth_first_search_with_scratch`] with predecessor
/// expansion in place of successor expansion.
pub fn reverse_breadth_first_search_with_scratch<'graph, 'scratch, G>(
    graph: &'graph G,
    start: ElementId<G>,
    visited: &'scratch mut [u8],
    queue: &'scratch mut [ElementId<G>],
) -> Result<ReverseBreadthFirstSearchScratch<'graph, 'scratch, G>, BfsError>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    let witness = ValidatedScratch::new(graph, start, visited.len(), queue.len())?;
    let frontier = SeededByteFlagFrontier::new(visited, queue, start, witness);
    Ok(ReverseBreadthFirstSearchScratch {
        inner: Bfs::new(graph, frontier),
    })
}
