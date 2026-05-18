//! Allocating arbitrary-ID BFS using a set-backed visited policy
//! (forward and reverse, `BTreeSet` and `HashSet`).
//!
//! Public consumers spell one of:
//! - [`GenericBreadthFirstSearch`] / [`GenericReverseBreadthFirstSearch`] — `alloc` only,
//!   `BTreeSet` visited set, `O((n + m) log n)`.
//! - [`HashBreadthFirstSearch`] / [`HashReverseBreadthFirstSearch`] — `std` feature, `HashSet`
//!   visited set, expected `O(n + m)`.
//!
//! Each pair binds the same expansion direction trait
//! ([`ElementSuccessors`] for forward, [`ElementPredecessors`] for reverse).
//! Storage policy is shared internally via [`GenericState`].

use alloc::collections::BTreeSet;
use core::iter::FusedIterator;
#[cfg(feature = "std")]
use std::collections::HashSet;

use oxgraph_topology::{ElementId, ElementPredecessors, ElementSuccessors, TopologyId};

use crate::bfs::core::{Bfs, Forward, GenericState, Reverse, VisitedSet};

/// `BTreeSet`-backed driver state used by the generic BFS variants.
type BTreeBfs<'graph, G, D> = Bfs<'graph, G, GenericState<G, BTreeSet<ElementId<G>>>, D>;

/// `HashSet`-backed driver state used by the `std`-only BFS variants.
#[cfg(feature = "std")]
type HashBfs<'graph, G, D> = Bfs<'graph, G, GenericState<G, HashSet<ElementId<G>>>, D>;

impl<T> VisitedSet<T> for BTreeSet<T>
where
    T: TopologyId,
{
    fn insert(&mut self, value: T) -> bool {
        Self::insert(self, value)
    }
}

#[cfg(feature = "std")]
impl<T> VisitedSet<T> for HashSet<T>
where
    T: TopologyId,
{
    fn insert(&mut self, value: T) -> bool {
        Self::insert(self, value)
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
    inner: BTreeBfs<'graph, G, Forward>,
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

/// Reverse BFS iterator for arbitrary element ID spaces using a `BTreeSet`.
///
/// Constructed only via [`reverse_breadth_first_search_generic`]. Same shape
/// as [`GenericBreadthFirstSearch`]; expansion follows
/// [`ElementPredecessors`].
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
    inner: BTreeBfs<'graph, G, Reverse>,
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

/// Forward BFS iterator for arbitrary element ID spaces using a `HashSet`.
///
/// Constructed only via [`breadth_first_search_generic_hash`].
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is expected `O(n + m)` and memory usage is `O(n)`.
#[cfg(feature = "std")]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct HashBreadthFirstSearch<'graph, G>
where
    G: ElementSuccessors,
{
    /// Underlying generic BFS driver carrying the state policy.
    inner: HashBfs<'graph, G, Forward>,
}

#[cfg(feature = "std")]
impl<G> Iterator for HashBreadthFirstSearch<'_, G>
where
    G: ElementSuccessors,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

#[cfg(feature = "std")]
impl<G> FusedIterator for HashBreadthFirstSearch<'_, G> where G: ElementSuccessors {}

/// Reverse BFS iterator for arbitrary element ID spaces using a `HashSet`.
///
/// Constructed only via [`reverse_breadth_first_search_generic_hash`].
///
/// # Performance
///
/// For a reverse-reachable subgraph with `n` elements and `m` predecessor
/// entries inspected, traversal is expected `O(n + m)` and memory usage is
/// `O(n)`.
#[cfg(feature = "std")]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct HashReverseBreadthFirstSearch<'graph, G>
where
    G: ElementPredecessors,
{
    /// Underlying generic BFS driver carrying the state policy and reverse
    /// direction selector.
    inner: HashBfs<'graph, G, Reverse>,
}

#[cfg(feature = "std")]
impl<G> Iterator for HashReverseBreadthFirstSearch<'_, G>
where
    G: ElementPredecessors,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

#[cfg(feature = "std")]
impl<G> FusedIterator for HashReverseBreadthFirstSearch<'_, G> where G: ElementPredecessors {}

/// Creates an allocating forward breadth-first traversal for arbitrary
/// element ID spaces using a `BTreeSet` visited set.
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
        inner: Bfs::new(graph, GenericState::new(start)),
    }
}

/// Creates an allocating reverse breadth-first traversal for arbitrary
/// element ID spaces using a `BTreeSet` visited set.
///
/// Same fallback / contract / performance shape as
/// [`breadth_first_search_generic`]; expansion uses
/// [`ElementPredecessors`] instead of [`ElementSuccessors`].
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
        inner: Bfs::new(graph, GenericState::new(start)),
    }
}

/// Creates a `std`-optimized forward BFS for arbitrary element ID spaces
/// using a `HashSet` visited set.
///
/// Traversal follows outgoing connections and yields each reachable element
/// at most once. The start element is yielded first. Uses hash-table
/// membership for expected `O(1)` checks.
///
/// `start` and all IDs returned by `graph.element_successors` must be valid
/// for `graph`.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is expected `O(n + m)` with hash-table membership
/// checks. Memory usage is `O(n)`.
#[cfg(feature = "std")]
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

/// Creates a `std`-optimized reverse BFS for arbitrary element ID spaces
/// using a `HashSet` visited set.
///
/// Same fallback / contract / performance shape as
/// [`breadth_first_search_generic_hash`]; expansion uses
/// [`ElementPredecessors`] instead of [`ElementSuccessors`].
///
/// # Performance
///
/// Same as [`breadth_first_search_generic_hash`] with predecessor expansion
/// in place of successor expansion.
#[cfg(feature = "std")]
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
