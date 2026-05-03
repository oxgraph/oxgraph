//! Allocating arbitrary-ID BFS using a `BTreeSet` visited set (forward and reverse).

use alloc::collections::{BTreeSet, VecDeque};
use core::iter::FusedIterator;

use oxgraph_topology::{ElementId, ElementPredecessors, ElementSuccessors, TopologyBase};

use crate::bfs::core::{Bfs, BfsStep, Forward, Reverse, Sealed};

/// Owned `BTreeSet`-backed visited set and `VecDeque` frontier for
/// arbitrary-ID BFS. Direction-agnostic: the same state drives both forward
/// and reverse traversals.
///
/// # Performance
///
/// Construction is `O(log 1)` for the initial visited-set insertion and uses
/// `O(1)` queue storage before traversal begins.
#[derive(Clone, Debug)]
struct GenericBTreeState<G>
where
    G: TopologyBase,
{
    /// Elements discovered but not yet yielded.
    queue: VecDeque<ElementId<G>>,
    /// Elements already discovered.
    visited: BTreeSet<ElementId<G>>,
}

impl<G> GenericBTreeState<G>
where
    G: TopologyBase,
{
    /// Seeds the frontier and visited set with `start`.
    ///
    /// # Performance
    ///
    /// `O(log 1)` for the initial visited-set insertion.
    fn new(start: ElementId<G>) -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(start);
        let mut visited = BTreeSet::new();
        visited.insert(start);
        Self { queue, visited }
    }
}

impl<G> Sealed for GenericBTreeState<G> where G: TopologyBase {}

impl<G> BfsStep<G> for GenericBTreeState<G>
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

/// Forward BFS iterator for arbitrary element ID spaces using a `BTreeSet`.
///
/// Constructed only via [`breadth_first_search_generic`]. The internal
/// storage policy is intentionally not part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is `O((n + m) log n)` and memory usage is `O(n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct GenericBreadthFirstSearch<'graph, G>
where
    G: ElementSuccessors,
{
    /// Underlying generic BFS driver carrying the state policy.
    inner: Bfs<'graph, G, GenericBTreeState<G>, Forward>,
}

impl<G> Iterator for GenericBreadthFirstSearch<'_, G>
where
    G: ElementSuccessors,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for GenericBreadthFirstSearch<'_, G> where G: ElementSuccessors {}

/// Creates an allocating forward breadth-first traversal for arbitrary
/// element ID spaces.
///
/// Traversal follows outgoing connections and yields each reachable element
/// at most once. The start element is yielded first. This fallback does not
/// require dense element indexes, so it works for topology views with sparse,
/// generated, remote, or otherwise non-indexable element IDs.
///
/// `start` and all IDs returned by `graph.element_successors` must be valid
/// for `graph`.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is `O((n + m) log n)` because the implementation
/// tracks visited elements in a `BTreeSet`. Memory usage is `O(n)`.
pub fn breadth_first_search_generic<G>(
    graph: &G,
    start: ElementId<G>,
) -> GenericBreadthFirstSearch<'_, G>
where
    G: ElementSuccessors,
{
    GenericBreadthFirstSearch {
        inner: Bfs::new(graph, GenericBTreeState::new(start)),
    }
}

/// Reverse BFS iterator for arbitrary element ID spaces using a `BTreeSet`.
///
/// Constructed only via [`reverse_breadth_first_search_generic`]. The
/// internal storage policy is intentionally not part of the public API.
///
/// # Performance
///
/// For a reverse-reachable subgraph with `n` elements and `m` predecessor
/// entries inspected, traversal is `O((n + m) log n)` and memory usage is
/// `O(n)`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct GenericReverseBreadthFirstSearch<'graph, G>
where
    G: ElementPredecessors,
{
    /// Underlying generic BFS driver carrying the state policy and reverse
    /// direction selector.
    inner: Bfs<'graph, G, GenericBTreeState<G>, Reverse>,
}

impl<G> Iterator for GenericReverseBreadthFirstSearch<'_, G>
where
    G: ElementPredecessors,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for GenericReverseBreadthFirstSearch<'_, G> where G: ElementPredecessors {}

/// Creates an allocating reverse breadth-first traversal for arbitrary
/// element ID spaces.
///
/// Traversal follows incoming connections and yields each reverse-reachable
/// element at most once. Same fallback / contract / performance shape as
/// [`breadth_first_search_generic`]; the only difference is that expansion
/// uses [`ElementPredecessors`](oxgraph_topology::ElementPredecessors)
/// instead of [`ElementSuccessors`](oxgraph_topology::ElementSuccessors).
///
/// # Performance
///
/// Same as [`breadth_first_search_generic`] with predecessor expansion in
/// place of successor expansion.
pub fn reverse_breadth_first_search_generic<G>(
    graph: &G,
    start: ElementId<G>,
) -> GenericReverseBreadthFirstSearch<'_, G>
where
    G: ElementPredecessors,
{
    GenericReverseBreadthFirstSearch {
        inner: Bfs::new(graph, GenericBTreeState::new(start)),
    }
}
