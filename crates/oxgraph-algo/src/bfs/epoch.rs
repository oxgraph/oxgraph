//! Epoch-marked no-allocation indexed BFS (forward and reverse).

use core::{iter::FusedIterator, marker::PhantomData};

use oxgraph_topology::{
    ContainsElement, DenseElementIndex, ElementId, ElementPredecessors, ElementSuccessors,
};

use crate::bfs::{
    BfsError,
    core::{Bfs, EpochCounter, Forward, Reverse, SeededEpochFrontier, ValidatedScratch},
};

/// Reusable caller-provided scratch for epoch-marked indexed BFS, branded to
/// a specific topology view type.
///
/// The mark slice stores traversal epochs instead of byte flags. Reusing this
/// value across traversals avoids clearing all `graph.element_bound()` entries
/// each time; only epoch overflow performs a full mark clear.
///
/// The `G` type parameter brands the scratch at compile time: a
/// `BfsEpochScratch<'_, ViewA>` cannot be passed to a traversal of `ViewB`.
/// `PhantomData<fn() -> G>` is covariant in `G` (function-return position is
/// covariant) and crucially keeps `Send` / `Sync` independent of `G`'s
/// thread-safety, so the scratch remains usable from threads that `G` itself
/// cannot cross.
///
/// The same scratch is usable for both forward and reverse traversal because
/// epoch advancement is direction-agnostic and the visited semantics of
/// "already discovered along the current expansion" hold regardless of
/// whether expansion follows successors or predecessors.
///
/// # Performance
///
/// Construction clears `O(m)` mark entries for `m = marks.len()`. Each traversal
/// advances the epoch in `O(1)` except on `u32` overflow, where it clears `O(m)`
/// marks before continuing. Memory usage is caller-provided `O(m + q)` for mark
/// and queue lengths `m` and `q`.
pub struct BfsEpochScratch<'scratch, G>
where
    G: ContainsElement + DenseElementIndex,
{
    /// Dense visited epoch marks indexed by `DenseElementIndex::element_index`.
    marks: &'scratch mut [u32],
    /// Queue storage for discovered elements.
    queue: &'scratch mut [ElementId<G>],
    /// Epoch counter that owns the per-traversal advance / overflow logic.
    epoch: EpochCounter,
    /// Brands the scratch to a specific topology view type without coupling
    /// `Send`/`Sync` to `G`.
    _graph: PhantomData<fn() -> G>,
}

impl<'scratch, G> BfsEpochScratch<'scratch, G>
where
    G: ContainsElement + DenseElementIndex,
{
    /// Creates reusable epoch scratch over caller-provided mark and queue
    /// slices.
    ///
    /// The view type `G` is bound by the queue slice's element type
    /// `ElementId<G>`; type inference picks it up from the queue argument
    /// when `ElementId<G>` is unique. When inference cannot resolve `G`
    /// (because multiple view types share the same `ElementId`), pair this
    /// with [`BfsEpochScratch::for_graph`] or use turbofish.
    ///
    /// Existing mark contents are cleared so the first traversal starts from
    /// a known epoch state. Queue contents are ignored.
    ///
    /// # Performance
    ///
    /// Clears `O(m)` entries for `m = marks.len()` and performs no heap
    /// allocation.
    pub fn new(marks: &'scratch mut [u32], queue: &'scratch mut [ElementId<G>]) -> Self {
        marks.fill(0);
        Self {
            marks,
            queue,
            epoch: EpochCounter::default(),
            _graph: PhantomData,
        }
    }

    /// Creates reusable epoch scratch with the view type bound from a
    /// borrowed reference.
    ///
    /// Equivalent to [`BfsEpochScratch::new`] but takes `&G` purely as a
    /// type witness so callers do not need turbofish when multiple view
    /// types share `ElementId`. The view reference is consumed only for type
    /// inference; its runtime value is not retained.
    ///
    /// # Performance
    ///
    /// Clears `O(m)` entries for `m = marks.len()` and performs no heap
    /// allocation.
    pub fn for_graph(
        _graph: &G,
        marks: &'scratch mut [u32],
        queue: &'scratch mut [ElementId<G>],
    ) -> Self {
        Self::new(marks, queue)
    }

    /// Returns the number of mark entries available to traversals.
    ///
    /// # Performance
    ///
    /// Runs in `O(1)`.
    #[must_use]
    pub const fn mark_capacity(&self) -> usize {
        self.marks.len()
    }

    /// Returns the number of queue slots available to traversals.
    ///
    /// # Performance
    ///
    /// Runs in `O(1)`.
    #[must_use]
    pub const fn queue_capacity(&self) -> usize {
        self.queue.len()
    }

    /// Advances to the next epoch and borrows the mark/queue slices for the
    /// level-synchronous bounded driver in [`crate::bfs::bounded`].
    ///
    /// Returns the freshly advanced epoch alongside disjoint mutable borrows of
    /// the mark and queue storage, so the bounded BFS reuses this scratch with
    /// the same epoch-overflow discipline as the per-element iterators.
    ///
    /// # Performance
    ///
    /// `O(1)` except on epoch overflow, which clears `O(marks.len())` marks.
    pub(in crate::bfs) fn bounded_parts(&mut self) -> (&mut [u32], &mut [ElementId<G>], u32) {
        let epoch = self.epoch.advance(self.marks);
        (&mut *self.marks, &mut *self.queue, epoch)
    }
}

/// Forward BFS iterator using caller-provided epoch-marked scratch.
///
/// Constructed only via [`breadth_first_search_with_epoch_scratch`]. The
/// internal storage policy ([`SeededEpochFrontier`]) is intentionally not
/// part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is `O(n + m)` and uses `O(b)` caller-provided scratch
/// memory for `b = graph.element_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearchEpochScratch<'graph, 'borrow, G>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededEpochFrontier<'borrow, G>, Forward>,
}

impl<G> Iterator for BreadthFirstSearchEpochScratch<'_, '_, G>
where
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearchEpochScratch<'_, '_, G> where
    G: ContainsElement + ElementSuccessors + DenseElementIndex
{
}

/// Creates a forward breadth-first traversal using reusable epoch scratch.
///
/// Traversal follows outgoing connections and yields each reachable element
/// at most once. `scratch` must provide at least `graph.element_bound()` mark
/// entries and queue slots. Unlike
/// [`crate::breadth_first_search_with_scratch`], construction does not clear
/// all visited entries on every traversal except when the epoch wraps.
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
/// the corresponding scratch storage is smaller than
/// `graph.element_bound()`, [`BfsError::StartElementNotContained`] when
/// `start` is not contained in `graph`, or [`BfsError::StartIndexOutOfBounds`]
/// when `start` maps outside `graph.element_bound()`. Errors are returned in
/// that order: scratch sizes are checked before start-element validation.
///
/// # Performance
///
/// Construction is `O(1)` after scratch capacity validation except on epoch
/// overflow, where it clears `O(m)` mark entries for `m = scratch.mark_capacity()`.
/// Traversal over a reachable subgraph with `n` elements and `m` successor
/// entries inspected is `O(n + m)` and performs no heap allocation.
pub fn breadth_first_search_with_epoch_scratch<'graph, 'borrow, 'scratch, G>(
    graph: &'graph G,
    start: ElementId<G>,
    scratch: &'borrow mut BfsEpochScratch<'scratch, G>,
) -> Result<BreadthFirstSearchEpochScratch<'graph, 'borrow, G>, BfsError>
where
    'scratch: 'borrow,
    G: ContainsElement + ElementSuccessors + DenseElementIndex,
{
    let witness = ValidatedScratch::new(graph, start, scratch.marks.len(), scratch.queue.len())?;
    let epoch = scratch.epoch.advance(scratch.marks);
    let frontier = SeededEpochFrontier::new(scratch.marks, scratch.queue, epoch, start, witness);
    Ok(BreadthFirstSearchEpochScratch {
        inner: Bfs::new(graph, frontier),
    })
}

/// Reverse BFS iterator using caller-provided epoch-marked scratch.
///
/// Constructed only via [`reverse_breadth_first_search_with_epoch_scratch`].
/// The internal storage policy ([`SeededEpochFrontier`]) is intentionally not
/// part of the public API.
///
/// # Performance
///
/// For a reverse-reachable subgraph with `n` elements and `m` predecessor
/// entries inspected, traversal is `O(n + m)` and uses `O(b)` caller-provided
/// scratch memory for `b = graph.element_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct ReverseBreadthFirstSearchEpochScratch<'graph, 'borrow, G>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier and reverse
    /// direction selector.
    inner: Bfs<'graph, G, SeededEpochFrontier<'borrow, G>, Reverse>,
}

impl<G> Iterator for ReverseBreadthFirstSearchEpochScratch<'_, '_, G>
where
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for ReverseBreadthFirstSearchEpochScratch<'_, '_, G> where
    G: ContainsElement + ElementPredecessors + DenseElementIndex
{
}

/// Creates a reverse breadth-first traversal using reusable epoch scratch.
///
/// Traversal follows incoming connections and yields each reverse-reachable
/// element at most once. Same scratch / contract / error shape as
/// [`breadth_first_search_with_epoch_scratch`]; the only difference is that
/// expansion uses
/// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors) instead of
/// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors).
///
/// # Errors
///
/// Same as [`breadth_first_search_with_epoch_scratch`].
///
/// # Performance
///
/// Same as [`breadth_first_search_with_epoch_scratch`] with predecessor
/// expansion in place of successor expansion.
pub fn reverse_breadth_first_search_with_epoch_scratch<'graph, 'borrow, 'scratch, G>(
    graph: &'graph G,
    start: ElementId<G>,
    scratch: &'borrow mut BfsEpochScratch<'scratch, G>,
) -> Result<ReverseBreadthFirstSearchEpochScratch<'graph, 'borrow, G>, BfsError>
where
    'scratch: 'borrow,
    G: ContainsElement + ElementPredecessors + DenseElementIndex,
{
    let witness = ValidatedScratch::new(graph, start, scratch.marks.len(), scratch.queue.len())?;
    let epoch = scratch.epoch.advance(scratch.marks);
    let frontier = SeededEpochFrontier::new(scratch.marks, scratch.queue, epoch, start, witness);
    Ok(ReverseBreadthFirstSearchEpochScratch {
        inner: Bfs::new(graph, frontier),
    })
}
