//! Scratch-backed no-allocation indexed BFS.

use core::iter::FusedIterator;

use oxgraph_graph::{ContainsNode, NodeId, NodeIndex, OutgoingNeighborsGraph};

use crate::bfs::{
    BfsError,
    core::{Bfs, SeededByteFlagFrontier, ValidatedScratch},
};

/// Directed BFS iterator using caller-provided byte-flag scratch.
///
/// Constructed only via [`breadth_first_search_with_scratch`]. The internal
/// storage policy ([`SeededByteFlagFrontier`]) is intentionally not part of
/// the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes and `m` outgoing neighbor entries
/// inspected, traversal is `O(n + m)` and uses `O(b)` caller-provided scratch
/// memory for `b = graph.node_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearchScratch<'graph, 'scratch, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededByteFlagFrontier<'scratch, G>>,
}

impl<G> Iterator for BreadthFirstSearchScratch<'_, '_, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    type Item = NodeId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearchScratch<'_, '_, G> where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex
{
}

/// Creates a directed breadth-first traversal using caller-provided scratch.
///
/// Traversal follows outgoing neighbor nodes and yields each reachable node at most once.
/// The start node is yielded first. `visited` and `queue` must each have at
/// least `graph.node_bound()` entries. The scratch slices are cleared and reused
/// by the traversal; their previous contents are ignored.
///
/// `start` and all IDs returned by `graph.outgoing_neighbors` must be valid for
/// `graph` and must map to indexes below `graph.node_bound()`. A neighbor
/// index at or past the bound observed at construction time will panic with
/// [`BfsError::NeighborIndexOutOfBounds`]; this is a graph-view contract
/// violation, not a recoverable failure.
///
/// # Errors
///
/// Returns [`BfsError::VisitedTooSmall`] or [`BfsError::QueueTooSmall`] when
/// the corresponding scratch slice is smaller than `graph.node_bound()`,
/// [`BfsError::StartNodeNotContained`] when `start` is not contained in
/// `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start` maps outside
/// `graph.node_bound()`. Errors are returned in that order: scratch sizes are
/// checked before start-node validation.
///
/// # Performance
///
/// Construction clears `O(b)` visited entries for `b = graph.node_bound()` and
/// performs no heap allocation. Traversal over a reachable subgraph with `n`
/// nodes and `m` outgoing neighbor entries inspected is `O(n + m)` and performs
/// no heap allocation. Reusing scratch across traversals reuses the same memory.
pub fn breadth_first_search_with_scratch<'graph, 'scratch, G>(
    graph: &'graph G,
    start: NodeId<G>,
    visited: &'scratch mut [u8],
    queue: &'scratch mut [NodeId<G>],
) -> Result<BreadthFirstSearchScratch<'graph, 'scratch, G>, BfsError>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    let witness = ValidatedScratch::new(graph, start, visited.len(), queue.len())?;
    let frontier = SeededByteFlagFrontier::new(visited, queue, start, witness);
    Ok(BreadthFirstSearchScratch {
        inner: Bfs::new(graph, frontier),
    })
}
