//! Standard-library arbitrary-ID BFS using a `HashSet` visited set.

use core::iter::FusedIterator;
use std::collections::{HashSet, VecDeque};

use oxgraph_graph::{NodeId, OutgoingNeighborsGraph};

use crate::bfs::core::{Bfs, BfsStep, Sealed};

/// Owned `HashSet`-backed visited set and `VecDeque` frontier for arbitrary-ID
/// BFS.
///
/// # Performance
///
/// Construction is expected `O(1)` for the initial visited-set insertion and
/// uses `O(1)` queue storage before traversal begins.
#[derive(Clone, Debug)]
struct GenericHashState<G>
where
    G: OutgoingNeighborsGraph,
{
    /// Nodes discovered but not yet yielded.
    queue: VecDeque<NodeId<G>>,
    /// Nodes already discovered.
    visited: HashSet<NodeId<G>>,
}

impl<G> GenericHashState<G>
where
    G: OutgoingNeighborsGraph,
{
    /// Seeds the frontier and visited set with `start`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` for the initial visited-set insertion.
    fn new(start: NodeId<G>) -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(start);
        let mut visited = HashSet::new();
        visited.insert(start);
        Self { queue, visited }
    }
}

impl<G> Sealed for GenericHashState<G> where G: OutgoingNeighborsGraph {}

impl<G> BfsStep<G> for GenericHashState<G>
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

/// Directed BFS iterator for arbitrary node ID spaces using a `HashSet`.
///
/// Constructed only via [`breadth_first_search_generic_hash`]. The internal
/// storage policy is intentionally not part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes and `m` outgoing neighbor entries
/// inspected, traversal is expected `O(n + m)` and memory usage is `O(n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct HashBreadthFirstSearch<'graph, G>
where
    G: OutgoingNeighborsGraph,
{
    /// Underlying generic BFS driver carrying the state policy.
    inner: Bfs<'graph, G, GenericHashState<G>>,
}

impl<G> Iterator for HashBreadthFirstSearch<'_, G>
where
    G: OutgoingNeighborsGraph,
{
    type Item = NodeId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for HashBreadthFirstSearch<'_, G> where G: OutgoingNeighborsGraph {}

/// Creates a `std`-optimized directed BFS for arbitrary node ID spaces.
///
/// Traversal follows outgoing neighbor nodes and yields each reachable node at most once.
/// The start node is yielded first. This fallback does not require dense node
/// indexes and uses `HashSet` membership tracking when `std` is available.
///
/// `start` and all IDs returned by `graph.outgoing_neighbors` must be valid for
/// `graph`.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes and `m` outgoing neighbor entries
/// inspected, traversal is expected `O(n + m)` with hash-table membership
/// checks. Memory usage is `O(n)`.
pub fn breadth_first_search_generic_hash<G>(
    graph: &G,
    start: NodeId<G>,
) -> HashBreadthFirstSearch<'_, G>
where
    G: OutgoingNeighborsGraph,
{
    HashBreadthFirstSearch {
        inner: Bfs::new(graph, GenericHashState::new(start)),
    }
}
