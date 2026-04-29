//! Epoch-marked no-allocation indexed BFS.

use core::{iter::FusedIterator, marker::PhantomData};

use oxgraph_graph::{ContainsNode, NodeId, NodeIndex, OutgoingNeighborsGraph};

use crate::bfs::{
    BfsError,
    core::{Bfs, SeededEpochFrontier, ValidatedScratch},
};

/// Reusable caller-provided scratch for epoch-marked indexed BFS, branded to
/// a specific graph type.
///
/// The mark slice stores traversal epochs instead of byte flags. Reusing this
/// value across traversals avoids clearing all `graph.node_bound()` entries each
/// time; only epoch overflow performs a full mark clear.
///
/// The `G` type parameter brands the scratch at compile time: a
/// `BfsEpochScratch<'_, GraphA>` cannot be passed to a traversal of `GraphB`.
/// `PhantomData<fn() -> G>` is covariant in `G` (function-return position is
/// covariant) and crucially keeps `Send` / `Sync` independent of `G`'s
/// thread-safety, so the scratch remains usable from threads that `G` itself
/// cannot cross.
///
/// # Performance
///
/// Construction clears `O(m)` mark entries for `m = marks.len()`. Each traversal
/// advances the epoch in `O(1)` except on `u32` overflow, where it clears `O(m)`
/// marks before continuing. Memory usage is caller-provided `O(m + q)` for mark
/// and queue lengths `m` and `q`.
pub struct BfsEpochScratch<'scratch, G>
where
    G: ContainsNode + NodeIndex,
{
    /// Dense visited epoch marks indexed by `NodeIndex::node_index`.
    marks: &'scratch mut [u32],
    /// Queue storage for discovered nodes.
    queue: &'scratch mut [NodeId<G>],
    /// Current non-zero traversal epoch. Zero is reserved for "never visited".
    epoch: u32,
    /// Brands the scratch to a specific graph type without coupling
    /// `Send`/`Sync` to `G`.
    _graph: PhantomData<fn() -> G>,
}

impl<'scratch, G> BfsEpochScratch<'scratch, G>
where
    G: ContainsNode + NodeIndex,
{
    /// Creates reusable epoch scratch over caller-provided mark and queue
    /// slices.
    ///
    /// The graph type `G` is bound by the queue slice's element type
    /// `NodeId<G>`; type inference picks it up from the queue argument when
    /// `NodeId<G>` is unique. When inference cannot resolve `G` (because
    /// multiple graph types share the same `NodeId`), pair this with
    /// [`BfsEpochScratch::for_graph`] or use turbofish.
    ///
    /// Existing mark contents are cleared so the first traversal starts from
    /// a known epoch state. Queue contents are ignored.
    ///
    /// # Performance
    ///
    /// Clears `O(m)` entries for `m = marks.len()` and performs no heap
    /// allocation.
    pub fn new(marks: &'scratch mut [u32], queue: &'scratch mut [NodeId<G>]) -> Self {
        marks.fill(0);
        Self {
            marks,
            queue,
            epoch: 0,
            _graph: PhantomData,
        }
    }

    /// Creates reusable epoch scratch with the graph type bound from a
    /// borrowed reference.
    ///
    /// Equivalent to [`BfsEpochScratch::new`] but takes `&G` purely as a
    /// type witness so callers do not need turbofish when multiple graph
    /// types share `NodeId`. The graph reference is consumed only for type
    /// inference; its runtime value is not retained.
    ///
    /// # Performance
    ///
    /// Clears `O(m)` entries for `m = marks.len()` and performs no heap
    /// allocation.
    pub fn for_graph(
        _graph: &G,
        marks: &'scratch mut [u32],
        queue: &'scratch mut [NodeId<G>],
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

    /// Advances to the next traversal epoch and returns it.
    ///
    /// On `u32` overflow, clears every mark to zero and resets the epoch to
    /// `1`, preserving the invariant that mark `0` represents "never visited"
    /// and any non-zero mark equal to the current epoch represents
    /// "visited this traversal".
    ///
    /// # Performance
    ///
    /// Runs in `O(1)` except when the epoch overflows, where it clears `O(m)`
    /// mark entries for `m = self.mark_capacity()`.
    fn advance_epoch(&mut self) -> u32 {
        if self.epoch == u32::MAX {
            self.marks.fill(0);
            self.epoch = 1;
        } else {
            self.epoch += 1;
        }
        self.epoch
    }
}

/// Directed BFS iterator using caller-provided epoch-marked scratch.
///
/// Constructed only via [`breadth_first_search_with_epoch_scratch`]. The
/// internal storage policy ([`SeededEpochFrontier`]) is intentionally not
/// part of the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes and `m` outgoing neighbor entries
/// inspected, traversal is `O(n + m)` and uses `O(b)` caller-provided scratch
/// memory for `b = graph.node_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearchEpochScratch<'graph, 'borrow, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededEpochFrontier<'borrow, G>>,
}

impl<G> Iterator for BreadthFirstSearchEpochScratch<'_, '_, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    type Item = NodeId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearchEpochScratch<'_, '_, G> where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex
{
}

/// Creates a directed breadth-first traversal using reusable epoch scratch.
///
/// Traversal follows outgoing neighbor nodes and yields each reachable node at most once.
/// `scratch` must provide at least `graph.node_bound()` mark entries and queue
/// slots. Unlike [`crate::breadth_first_search_with_scratch`], construction does
/// not clear all visited entries on every traversal except when the epoch wraps.
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
/// the corresponding scratch storage is smaller than `graph.node_bound()`,
/// [`BfsError::StartNodeNotContained`] when `start` is not contained in
/// `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start` maps outside
/// `graph.node_bound()`. Errors are returned in that order: scratch sizes are
/// checked before start-node validation.
///
/// # Performance
///
/// Construction is `O(1)` after scratch capacity validation except on epoch
/// overflow, where it clears `O(m)` mark entries for `m = scratch.mark_capacity()`.
/// Traversal over a reachable subgraph with `n` nodes and `m` outgoing neighbor
/// entries inspected is `O(n + m)` and performs no heap allocation.
pub fn breadth_first_search_with_epoch_scratch<'graph, 'borrow, 'scratch, G>(
    graph: &'graph G,
    start: NodeId<G>,
    scratch: &'borrow mut BfsEpochScratch<'scratch, G>,
) -> Result<BreadthFirstSearchEpochScratch<'graph, 'borrow, G>, BfsError>
where
    'scratch: 'borrow,
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    let witness = ValidatedScratch::new(graph, start, scratch.marks.len(), scratch.queue.len())?;
    let epoch = scratch.advance_epoch();
    let frontier = SeededEpochFrontier::new(scratch.marks, scratch.queue, epoch, start, witness);
    Ok(BreadthFirstSearchEpochScratch {
        inner: Bfs::new(graph, frontier),
    })
}
