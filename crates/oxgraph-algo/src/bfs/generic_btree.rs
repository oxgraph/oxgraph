//! Allocating arbitrary-ID BFS using a `BTreeSet` visited set.

use alloc::collections::{BTreeSet, VecDeque};
use core::iter::FusedIterator;

use oxgraph_graph::{NodeId, OutgoingNeighborsGraph};

use crate::bfs::core::{Bfs, BfsStep, Sealed};

/// Owned `BTreeSet`-backed visited set and `VecDeque` frontier for arbitrary-ID
/// BFS.
///
/// # Performance
///
/// Construction is `O(log 1)` for the initial visited-set insertion and uses
/// `O(1)` queue storage before traversal begins.
#[derive(Clone, Debug)]
struct GenericBTreeState<G>
where
    G: OutgoingNeighborsGraph,
{
    /// Nodes discovered but not yet yielded.
    queue: VecDeque<NodeId<G>>,
    /// Nodes already discovered.
    visited: BTreeSet<NodeId<G>>,
}

impl<G> GenericBTreeState<G>
where
    G: OutgoingNeighborsGraph,
{
    /// Seeds the frontier and visited set with `start`.
    ///
    /// # Performance
    ///
    /// `O(log 1)` for the initial visited-set insertion.
    fn new(start: NodeId<G>) -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(start);
        let mut visited = BTreeSet::new();
        visited.insert(start);
        Self { queue, visited }
    }
}

impl<G> Sealed for GenericBTreeState<G> where G: OutgoingNeighborsGraph {}

impl<G> BfsStep<G> for GenericBTreeState<G>
where
    G: OutgoingNeighborsGraph,
{
    fn pop(&mut self) -> Option<NodeId<G>> {
        self.queue.pop_front()
    }

    fn try_visit_and_push(&mut self, _graph: &G, target: NodeId<G>) {
        if self.visited.insert(target) {
            self.queue.push_back(target);
        }
    }
}

/// Directed BFS iterator for arbitrary node ID spaces using a `BTreeSet`.
///
/// Constructed only via [`breadth_first_search_generic`]. The internal storage
/// policy is intentionally not part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes and `m` outgoing neighbor entries
/// inspected, traversal is `O((n + m) log n)` and memory usage is `O(n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct GenericBreadthFirstSearch<'graph, G>
where
    G: OutgoingNeighborsGraph,
{
    /// Underlying generic BFS driver carrying the state policy.
    inner: Bfs<'graph, G, GenericBTreeState<G>>,
}

impl<G> Iterator for GenericBreadthFirstSearch<'_, G>
where
    G: OutgoingNeighborsGraph,
{
    type Item = NodeId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for GenericBreadthFirstSearch<'_, G> where G: OutgoingNeighborsGraph {}

/// Creates an allocating directed breadth-first traversal for arbitrary node ID spaces.
///
/// Traversal follows outgoing neighbor nodes and yields each reachable node at most once.
/// The start node is yielded first. This fallback does not require dense node
/// indexes, so it works for graph views with sparse, generated, remote, or
/// otherwise non-indexable node IDs.
///
/// `start` and all IDs returned by `graph.outgoing_neighbors` must be valid for
/// `graph`.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes and `m` outgoing neighbor entries
/// inspected, traversal is `O((n + m) log n)` because the implementation tracks
/// visited nodes in a `BTreeSet`. Memory usage is `O(n)`.
pub fn breadth_first_search_generic<G>(
    graph: &G,
    start: NodeId<G>,
) -> GenericBreadthFirstSearch<'_, G>
where
    G: OutgoingNeighborsGraph,
{
    GenericBreadthFirstSearch {
        inner: Bfs::new(graph, GenericBTreeState::new(start)),
    }
}
