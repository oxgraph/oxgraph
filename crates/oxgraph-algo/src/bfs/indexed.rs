//! Allocating dense-indexed BFS.

use core::iter::FusedIterator;

use oxgraph_graph::{ContainsNode, NodeId, NodeIndex, OutgoingNeighborsGraph};

use crate::bfs::{
    BfsError,
    core::{Bfs, SeededOwnedFrontier, ValidatedStart},
};

/// Directed BFS iterator using owned dense scratch storage.
///
/// Constructed only via [`breadth_first_search`]. The internal storage policy
/// ([`SeededOwnedFrontier`]) is intentionally not part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes, `m` outgoing neighbor entries
/// inspected, and node index bound `b`, traversal is `O(n + m)` and memory
/// usage is `O(b + n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearch<'graph, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededOwnedFrontier<G>>,
}

impl<G> Iterator for BreadthFirstSearch<'_, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    type Item = NodeId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearch<'_, G> where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex
{
}

/// Creates an allocating directed breadth-first traversal using dense node indexes.
///
/// Traversal follows outgoing neighbor nodes and yields each reachable node at most once.
/// The start node is yielded first. This is the convenience path for graph views
/// that can map node IDs to dense scratch-storage indexes.
///
/// `start` and all IDs returned by `graph.outgoing_neighbors` must be valid for
/// `graph` and must map to indexes below `graph.node_bound()`. A neighbor
/// index at or past the bound observed at construction time will panic with
/// [`BfsError::NeighborIndexOutOfBounds`]; this is a graph-view contract
/// violation, not a recoverable failure.
///
/// # Errors
///
/// Returns [`BfsError::StartNodeNotContained`] when `start` is not contained
/// in `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start` maps
/// outside `graph.node_bound()`. Allocation failure is not represented by
/// this error and follows the allocator's behavior.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes, `m` outgoing neighbor entries
/// inspected, and node index bound `b`, construction allocates `O(b)` visited
/// storage and frontier capacity. Traversal is `O(n + m)`. Memory usage is
/// `O(b + n)`.
pub fn breadth_first_search<G>(
    graph: &G,
    start: NodeId<G>,
) -> Result<BreadthFirstSearch<'_, G>, BfsError>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    let witness = ValidatedStart::new(graph, start)?;
    let frontier = SeededOwnedFrontier::new(start, witness);
    Ok(BreadthFirstSearch {
        inner: Bfs::new(graph, frontier),
    })
}
