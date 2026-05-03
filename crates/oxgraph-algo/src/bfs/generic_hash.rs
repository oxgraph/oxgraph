//! Standard-library arbitrary-ID BFS using a `HashSet` visited set
//! (forward and reverse).

use core::iter::FusedIterator;
use std::collections::{HashSet, VecDeque};

use oxgraph_topology::{ElementId, ElementPredecessors, ElementSuccessors, TopologyBase};

use crate::bfs::core::{Bfs, BfsStep, Forward, Reverse, Sealed};

/// Owned `HashSet`-backed visited set and `VecDeque` frontier for
/// arbitrary-ID BFS. Direction-agnostic: the same state drives both forward
/// and reverse traversals.
///
/// # Performance
///
/// Construction is expected `O(1)` for the initial visited-set insertion and
/// uses `O(1)` queue storage before traversal begins.
#[derive(Clone, Debug)]
struct GenericHashState<G>
where
    G: TopologyBase,
{
    /// Elements discovered but not yet yielded.
    queue: VecDeque<ElementId<G>>,
    /// Elements already discovered.
    visited: HashSet<ElementId<G>>,
}

impl<G> GenericHashState<G>
where
    G: TopologyBase,
{
    /// Seeds the frontier and visited set with `start`.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` for the initial visited-set insertion.
    fn new(start: ElementId<G>) -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(start);
        let mut visited = HashSet::new();
        visited.insert(start);
        Self { queue, visited }
    }
}

impl<G> Sealed for GenericHashState<G> where G: TopologyBase {}

impl<G> BfsStep<G> for GenericHashState<G>
where
    G: TopologyBase,
{
    fn pop(&mut self) -> Option<ElementId<G>> {
        self.queue.pop_front()
    }

    fn try_visit_and_push(&mut self, _graph: &G, target: ElementId<G>) {
        if self.visited.insert(target) {
            self.queue.push_back(target);
        }
    }
}

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
    inner: Bfs<'graph, G, GenericHashState<G>, Forward>,
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
        inner: Bfs::new(graph, GenericHashState::new(start)),
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
    inner: Bfs<'graph, G, GenericHashState<G>, Reverse>,
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
        inner: Bfs::new(graph, GenericHashState::new(start)),
    }
}
