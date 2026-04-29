//! Private BFS traversal core.
//!
//! Houses the [`BfsStep`] storage-policy trait, the unified [`Bfs`] iterator
//! wrapper that drives any `BfsStep`, the validation-witness types
//! ([`ValidatedStart`] and [`ValidatedScratch`]) every indexed entry-point must
//! mint before construction, and the canonical [`SeededByteFlagFrontier`],
//! [`SeededEpochFrontier`], [`SeededWorkspaceFrontier`], and
//! [`SeededOwnedFrontier`] storage primitives.
//!
//! Each `Seeded*Frontier` is itself the indexed `BfsStep` — there are no
//! per-variant wrapper states. The variant files in `bfs/` provide only the
//! public iterator newtype, the `Iterator` delegation, and the
//! `breadth_first_search_*` entry point that mints the witness, builds the
//! seeded frontier, and wraps it in [`Bfs`].
//!
//! Invariant chain: `ValidatedStart` / `ValidatedScratch` is constructed only
//! by validation; a `Seeded*Frontier` is constructed only by consuming the
//! corresponding witness via [`ValidatedStart::into_parts`] /
//! [`ValidatedScratch::into_parts`]; the seeded frontier's storage fields are
//! private to this module; therefore an indexed BFS step's storage cannot be
//! reached without traversal-time validation having succeeded. Witness fields
//! are also private to this module so siblings under `bfs/` cannot fabricate
//! one via struct-literal construction.

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::{iter::FusedIterator, marker::PhantomData};

use oxgraph_graph::{ContainsNode, NodeId, NodeIndex, OutgoingNeighborsGraph};

use crate::bfs::BfsError;

/// Sealing supertrait for [`BfsStep`].
///
/// Visible only inside `bfs/` (via `pub(super)` plus the private `mod core;`
/// declaration), so external crates cannot `impl Sealed for ...` and therefore
/// cannot implement [`BfsStep`]. Required because [`Bfs::new`] is
/// `pub(super)`, meaning any outside `BfsStep` impl would be a dead-end API.
///
/// # Performance
///
/// `perf: unspecified` (marker trait — no runtime cost).
pub(super) trait Sealed {}

/// Storage-policy seam for breadth-first search.
///
/// Implementors own the visited set and frontier. [`Bfs`] holds
/// the graph reference and drives the implementor through one BFS yield per
/// `Iterator::next` call.
///
/// `try_visit_and_push` receives the graph as an explicit argument because
/// indexed implementations need it to compute dense node indexes; generic
/// implementations ignore it. Holding the graph in [`Bfs`] (not
/// the state) lets `Iterator::next` borrow the graph and the state from
/// disjoint fields of `self`, satisfying the borrow checker without copying.
///
/// This trait is sealed: only `bfs/` types implement it. The four indexed
/// implementations are on the [`SeededByteFlagFrontier`],
/// [`SeededEpochFrontier`], [`SeededWorkspaceFrontier`], and
/// [`SeededOwnedFrontier`] types in this module; the two set-based
/// fallbacks live in `generic_btree.rs` and `generic_hash.rs`.
///
/// # Performance
///
/// Per-method cost is implementor-defined. [`Bfs::next`] depends on `pop`
/// being amortized `O(1)` and `try_visit_and_push` being amortized `O(1)` for
/// indexed implementors and `O(log n)` / expected `O(1)` for the
/// `BTreeSet` / `HashSet` fallbacks; see each impl's docs.
pub(super) trait BfsStep<G>: Sealed
where
    G: OutgoingNeighborsGraph,
{
    /// Removes and returns the next frontier node, or `None` when the frontier
    /// is empty.
    ///
    /// # Performance
    ///
    /// Amortized `O(1)` for every implementor in this crate. Idempotent on
    /// exhaustion: repeated calls after returning `None` continue to return
    /// `None` without advancing internal cursors.
    fn pop(&mut self) -> Option<NodeId<G>>;

    /// Marks `target` as visited if it was previously unvisited and pushes it
    /// onto the frontier in that case. Idempotent on already-visited nodes.
    /// Implementations that need a dense index for `target` use `graph` to
    /// compute it; index-free implementations ignore `graph`.
    ///
    /// # Performance
    ///
    /// `O(1)` for indexed implementors (dense byte-flag or epoch-mark
    /// lookup); `O(log n)` for the `BTreeSet` fallback; expected `O(1)` for
    /// the `HashSet` fallback.
    fn try_visit_and_push(&mut self, graph: &G, target: NodeId<G>);
}

/// Directed breadth-first traversal iterator over any [`BfsStep`] storage policy.
///
/// All BFS variants exposed by this crate are type aliases over
/// `Bfs` instantiated with a particular state type. The single
/// [`Iterator`] impl below is therefore the only place the BFS loop body
/// lives.
///
/// # Performance
///
/// Per-yield cost is `O(deg(node))` outgoing-neighbor inspections plus the
/// `BfsStep` impl's amortized pop and visit-and-push cost. The wrapper itself
/// adds no work beyond field projection.
pub(super) struct Bfs<'graph, G, S>
where
    G: OutgoingNeighborsGraph,
    S: BfsStep<G>,
{
    /// Graph view being traversed.
    graph: &'graph G,
    /// Storage policy carrying visited set and frontier.
    state: S,
}

impl<'graph, G, S> Bfs<'graph, G, S>
where
    G: OutgoingNeighborsGraph,
    S: BfsStep<G>,
{
    /// Wraps a graph view and a constructed [`BfsStep`] state.
    ///
    /// `pub(super)` because callers always reach this constructor through a
    /// variant-specific `breadth_first_search_*` entry point that has already
    /// validated inputs and built the state via the corresponding
    /// `Seeded*Frontier::new` (indexed) or directly (set-based fallbacks).
    ///
    /// # Performance
    ///
    /// `O(1)` field assignment.
    pub(super) const fn new(graph: &'graph G, state: S) -> Self {
        Self { graph, state }
    }
}

impl<G, S> Iterator for Bfs<'_, G, S>
where
    G: OutgoingNeighborsGraph,
    S: BfsStep<G>,
{
    type Item = NodeId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.state.pop()?;
        for target in self.graph.outgoing_neighbors(node) {
            self.state.try_visit_and_push(self.graph, target);
        }
        Some(node)
    }
}

// Sound for every `BfsStep` impl in this crate: each `pop` is idempotent on
// exhaustion (queue cursors don't advance once empty), so once `next` returns
// `None` it continues to return `None`.
impl<G, S> FusedIterator for Bfs<'_, G, S>
where
    G: OutgoingNeighborsGraph,
    S: BfsStep<G>,
{
}

/// Witness that a `start` node is contained in a graph view and maps inside
/// its `node_bound()`.
///
/// Constructed only via [`ValidatedStart::new`], which runs the validation
/// body. Fields are private to this module, so siblings under `bfs/` cannot
/// fabricate a witness via struct-literal construction. Branded with both the
/// graph borrow lifetime and the graph type, so a witness minted for one
/// `&'graph G` cannot be passed to a state instantiated for a different graph
/// or borrow.
///
/// Used by allocating indexed entry-points (`indexed.rs`, `workspace.rs`)
/// that allocate per-traversal storage themselves and only need a validated
/// start position.
///
/// # Performance
///
/// Holds two `usize` fields and a zero-sized brand. No runtime overhead
/// versus the previous free `validate_indexed_start` helper.
#[cfg(feature = "alloc")]
#[must_use = "a validation witness encodes a runtime check; consume it via Seeded*Frontier::new"]
pub(super) struct ValidatedStart<'graph, G> {
    /// Cached dense index of the validated start node.
    start_index: usize,
    /// Cached `graph.node_bound()` observed during validation.
    bound: usize,
    /// Brands the witness against the graph borrow lifetime and the graph
    /// type. `PhantomData<&'graph G>` covaries in `'graph`, which is what we
    /// want — the witness must not outlive the borrow it was minted against.
    _graph: PhantomData<&'graph G>,
}

#[cfg(feature = "alloc")]
impl<'graph, G> ValidatedStart<'graph, G>
where
    G: ContainsNode + NodeIndex,
{
    /// Validates that `start` is contained in `graph` and maps inside the
    /// dense node bound.
    ///
    /// # Errors
    ///
    /// Returns [`BfsError::StartNodeNotContained`] when
    /// `graph.contains_node(start)` is false, or
    /// [`BfsError::StartIndexOutOfBounds`] when `graph.node_index(start)` is
    /// at or past `graph.node_bound()`.
    ///
    /// # Performance
    ///
    /// `O(1)` plus the cost of `graph.contains_node` and `graph.node_index`.
    pub(super) fn new(graph: &'graph G, start: NodeId<G>) -> Result<Self, BfsError> {
        let bound = graph.node_bound();
        if !graph.contains_node(start) {
            return Err(BfsError::StartNodeNotContained);
        }
        let start_index = graph.node_index(start);
        if start_index >= bound {
            return Err(BfsError::StartIndexOutOfBounds {
                index: start_index,
                bound,
            });
        }
        Ok(Self {
            start_index,
            bound,
            _graph: PhantomData,
        })
    }

    /// Returns the cached `graph.node_bound()` observed during validation.
    ///
    /// Borrowing accessor used by `workspace.rs` to size the workspace
    /// before consuming the witness via [`SeededWorkspaceFrontier::new`].
    /// The start index is never returned through a borrowing accessor —
    /// it is only released by [`Self::into_parts`], which consumes the
    /// witness exactly once.
    ///
    /// # Performance
    ///
    /// `O(1)` field read.
    pub(super) const fn bound(&self) -> usize {
        self.bound
    }

    /// Consumes the witness and returns `(start_index, bound)`.
    ///
    /// Single-call consumption: a witness can seed exactly one frontier.
    /// Used by [`SeededOwnedFrontier::new`] and [`SeededWorkspaceFrontier::new`].
    ///
    /// # Performance
    ///
    /// `O(1)` field copy.
    pub(super) const fn into_parts(self) -> (usize, usize) {
        (self.start_index, self.bound)
    }
}

/// Witness that caller-provided scratch is at least `graph.node_bound()` and
/// that `start` is contained in the graph and maps inside the bound.
///
/// Constructed only via [`ValidatedScratch::new`], which runs the validation
/// body. Fields are private to this module. Branded with both the graph
/// borrow lifetime and the graph type, so a witness minted for one
/// `&'graph G` cannot be passed to a state instantiated for a different
/// graph or borrow.
///
/// Used by no-allocation indexed entry-points (`scratch.rs`, `epoch.rs`)
/// that consume caller-provided slices and need both start-node validation
/// and scratch-capacity validation in a single pass.
///
/// # Performance
///
/// Holds two `usize` fields and a zero-sized brand. Calls
/// `graph.node_bound()` exactly once and inlines the start-node checks
/// using the cached bound; no second graph query is required.
#[must_use = "a validation witness encodes a runtime check; consume it via Seeded*Frontier::new"]
pub(super) struct ValidatedScratch<'graph, G> {
    /// Cached dense index of the validated start node.
    start_index: usize,
    /// Cached `graph.node_bound()` observed during validation.
    bound: usize,
    /// Brands the witness against the graph borrow lifetime and the graph
    /// type.
    _graph: PhantomData<&'graph G>,
}

impl<'graph, G> ValidatedScratch<'graph, G>
where
    G: ContainsNode + NodeIndex,
{
    /// Validates that the caller-provided scratch slices fit
    /// `graph.node_bound()` and that `start` lives inside the bound.
    ///
    /// # Errors
    ///
    /// Returns the first matching variant of [`BfsError`] in this order:
    /// [`BfsError::VisitedTooSmall`], [`BfsError::QueueTooSmall`],
    /// [`BfsError::StartNodeNotContained`], [`BfsError::StartIndexOutOfBounds`].
    ///
    /// # Performance
    ///
    /// `O(1)` plus the cost of `graph.contains_node` and `graph.node_index`.
    pub(super) fn new(
        graph: &'graph G,
        start: NodeId<G>,
        visited_len: usize,
        queue_len: usize,
    ) -> Result<Self, BfsError> {
        let bound = graph.node_bound();
        if visited_len < bound {
            return Err(BfsError::VisitedTooSmall {
                needed: bound,
                actual: visited_len,
            });
        }
        if queue_len < bound {
            return Err(BfsError::QueueTooSmall {
                needed: bound,
                actual: queue_len,
            });
        }
        if !graph.contains_node(start) {
            return Err(BfsError::StartNodeNotContained);
        }
        let start_index = graph.node_index(start);
        if start_index >= bound {
            return Err(BfsError::StartIndexOutOfBounds {
                index: start_index,
                bound,
            });
        }
        Ok(Self {
            start_index,
            bound,
            _graph: PhantomData,
        })
    }

    /// Consumes the witness and returns `(start_index, bound)`.
    ///
    /// Single-call consumption: a witness can seed exactly one frontier.
    /// Used by [`SeededByteFlagFrontier::new`] and [`SeededEpochFrontier::new`].
    ///
    /// # Performance
    ///
    /// `O(1)` field copy.
    pub(super) const fn into_parts(self) -> (usize, usize) {
        (self.start_index, self.bound)
    }
}

/// Caller-slice byte-flag visited storage and queue, seeded against a
/// validated start. Implements [`BfsStep`] directly — there is no per-variant
/// wrapper state.
///
/// Construction is the only canonical path that can populate this layout: the
/// constructor takes a [`ValidatedScratch`] by value (consuming it via
/// [`ValidatedScratch::into_parts`]), clears `visited[..bound]`, marks the
/// start, seats the start at `queue[0]`, and sets `head = 0`, `tail = 1`.
///
/// # Performance
///
/// Construction clears `O(b)` visited entries for `b = witness.bound()` and
/// performs no heap allocation.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(super) struct SeededByteFlagFrontier<'scratch, G>
where
    G: NodeIndex,
{
    /// Dense visited flags indexed by `NodeIndex::node_index`.
    visited: &'scratch mut [u8],
    /// Queue storage for discovered nodes.
    queue: &'scratch mut [NodeId<G>],
    /// Next queue slot to yield.
    head: usize,
    /// One-past-last initialized queue slot.
    tail: usize,
}

impl<'scratch, G> SeededByteFlagFrontier<'scratch, G>
where
    G: ContainsNode + NodeIndex,
{
    /// Clears `visited[..bound]`, marks the validated start index, seats
    /// `start` at `queue[0]`, and primes head/tail cursors.
    ///
    /// Consumes the witness via [`ValidatedScratch::into_parts`]: a single
    /// witness cannot seed two frontiers.
    ///
    /// # Performance
    ///
    /// Clears `O(b)` visited entries for `b = witness.bound()` and performs
    /// no heap allocation.
    pub(super) fn new(
        visited: &'scratch mut [u8],
        queue: &'scratch mut [NodeId<G>],
        start: NodeId<G>,
        witness: ValidatedScratch<'_, G>,
    ) -> Self {
        let (start_index, bound) = witness.into_parts();
        visited[..bound].fill(0);
        visited[start_index] = 1;
        queue[0] = start;
        Self {
            visited,
            queue,
            head: 0,
            tail: 1,
        }
    }
}

impl<G> Sealed for SeededByteFlagFrontier<'_, G> where G: NodeIndex {}

impl<G> BfsStep<G> for SeededByteFlagFrontier<'_, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    fn pop(&mut self) -> Option<NodeId<G>> {
        if self.head == self.tail {
            return None;
        }
        let node = self.queue[self.head];
        self.head += 1;
        Some(node)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: NodeId<G>) {
        let target_index = graph.node_index(target);
        let visited_len = self.visited.len();
        let Some(slot) = self.visited.get_mut(target_index) else {
            neighbor_oob(target_index, visited_len)
        };
        if *slot == 0 {
            *slot = 1;
            self.queue[self.tail] = target;
            self.tail += 1;
        }
    }
}

/// Caller-slice epoch-marked visited storage and queue, seeded against a
/// validated start at a fresh traversal epoch. Implements [`BfsStep`]
/// directly.
///
/// Construction takes the witness, the scratch slices, and the epoch
/// supplied by the wrapping reusable scratch. It marks the start with the
/// epoch, seats the start at `queue[0]`, and sets `head = 0`, `tail = 1`.
/// The wrapping scratch advances the epoch outside this function (epoch
/// logic is owned by the public `BfsEpochScratch` type in `epoch.rs`); this
/// constructor merely consumes the resulting epoch value alongside the
/// witness.
///
/// # Performance
///
/// `O(1)` after capacity validation. No heap allocation.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(super) struct SeededEpochFrontier<'borrow, G>
where
    G: NodeIndex,
{
    /// Dense visited epoch marks indexed by `NodeIndex::node_index`.
    marks: &'borrow mut [u32],
    /// Queue storage for discovered nodes.
    queue: &'borrow mut [NodeId<G>],
    /// Traversal epoch treated as visited.
    epoch: u32,
    /// Next queue slot to yield.
    head: usize,
    /// One-past-last initialized queue slot.
    tail: usize,
}

impl<'borrow, G> SeededEpochFrontier<'borrow, G>
where
    G: ContainsNode + NodeIndex,
{
    /// Marks the validated start index with `epoch`, seats `start` at
    /// `queue[0]`, and primes head/tail cursors.
    ///
    /// Consumes the witness via [`ValidatedScratch::into_parts`].
    ///
    /// # Performance
    ///
    /// `O(1)`. No heap allocation.
    pub(super) fn new(
        marks: &'borrow mut [u32],
        queue: &'borrow mut [NodeId<G>],
        epoch: u32,
        start: NodeId<G>,
        witness: ValidatedScratch<'_, G>,
    ) -> Self {
        let (start_index, _bound) = witness.into_parts();
        marks[start_index] = epoch;
        queue[0] = start;
        Self {
            marks,
            queue,
            epoch,
            head: 0,
            tail: 1,
        }
    }
}

impl<G> Sealed for SeededEpochFrontier<'_, G> where G: NodeIndex {}

impl<G> BfsStep<G> for SeededEpochFrontier<'_, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    fn pop(&mut self) -> Option<NodeId<G>> {
        if self.head == self.tail {
            return None;
        }
        let node = self.queue[self.head];
        self.head += 1;
        Some(node)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: NodeId<G>) {
        let target_index = graph.node_index(target);
        let marks_len = self.marks.len();
        let Some(mark) = self.marks.get_mut(target_index) else {
            neighbor_oob(target_index, marks_len)
        };
        if *mark != self.epoch {
            *mark = self.epoch;
            self.queue[self.tail] = target;
            self.tail += 1;
        }
    }
}

/// Workspace-borrowed epoch-marked visited storage and owned queue handle for
/// indexed BFS, seeded against a validated start at a fresh traversal epoch.
/// Implements [`BfsStep`] directly.
///
/// The wrapping `BfsWorkspace` is responsible for `ensure_node_bound` and
/// `advance_epoch` before this constructor is called; this layer merely
/// consumes the resulting borrows and primes the frontier.
///
/// # Performance
///
/// `O(1)`. The wrapping workspace pays `O(b)` for any growth and `O(1)`
/// amortized for an epoch advance unless the epoch overflows.
#[cfg(feature = "alloc")]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(super) struct SeededWorkspaceFrontier<'workspace, G>
where
    G: NodeIndex,
{
    /// Dense visited epoch marks indexed by `NodeIndex::node_index`.
    marks: &'workspace mut [u32],
    /// Queue storage for discovered nodes.
    queue: &'workspace mut Vec<NodeId<G>>,
    /// Traversal epoch treated as visited.
    epoch: u32,
    /// Next queue slot to yield.
    head: usize,
}

#[cfg(feature = "alloc")]
impl<'workspace, G> SeededWorkspaceFrontier<'workspace, G>
where
    G: ContainsNode + NodeIndex,
{
    /// Clears the workspace queue, marks the validated start index with
    /// `epoch`, pushes `start`, and primes the head cursor.
    ///
    /// Consumes the witness via [`ValidatedStart::into_parts`].
    ///
    /// # Performance
    ///
    /// `O(1)` plus the queue's clear cost (which does not deallocate).
    pub(super) fn new(
        marks: &'workspace mut [u32],
        queue: &'workspace mut Vec<NodeId<G>>,
        epoch: u32,
        start: NodeId<G>,
        witness: ValidatedStart<'_, G>,
    ) -> Self {
        let (start_index, _bound) = witness.into_parts();
        queue.clear();
        marks[start_index] = epoch;
        queue.push(start);
        Self {
            marks,
            queue,
            epoch,
            head: 0,
        }
    }
}

#[cfg(feature = "alloc")]
impl<G> Sealed for SeededWorkspaceFrontier<'_, G> where G: NodeIndex {}

#[cfg(feature = "alloc")]
impl<G> BfsStep<G> for SeededWorkspaceFrontier<'_, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    fn pop(&mut self) -> Option<NodeId<G>> {
        let node = self.queue.get(self.head).copied()?;
        self.head += 1;
        Some(node)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: NodeId<G>) {
        let target_index = graph.node_index(target);
        let marks_len = self.marks.len();
        let Some(mark) = self.marks.get_mut(target_index) else {
            neighbor_oob(target_index, marks_len)
        };
        if *mark != self.epoch {
            *mark = self.epoch;
            self.queue.push(target);
        }
    }
}

/// Owned-vec byte-flag visited storage and queue, seeded against a validated
/// start. Implements [`BfsStep`] directly.
///
/// Allocates fresh visited and queue vectors sized to the witness's bound.
/// Used by the allocating non-reusable indexed BFS in `indexed.rs`.
///
/// # Performance
///
/// Allocates `O(b)` visited storage and frontier capacity for
/// `b = witness.bound()`.
#[cfg(feature = "alloc")]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(super) struct SeededOwnedFrontier<G>
where
    G: NodeIndex,
{
    /// Dense visited flags indexed by `NodeIndex::node_index`.
    visited: Vec<u8>,
    /// Nodes discovered in yield order.
    queue: Vec<NodeId<G>>,
    /// Next queue slot to yield.
    head: usize,
}

#[cfg(feature = "alloc")]
impl<G> SeededOwnedFrontier<G>
where
    G: ContainsNode + NodeIndex,
{
    /// Allocates `vec![0; bound]` for visited, primes `vec![start]` for the
    /// queue, and marks the validated start index.
    ///
    /// Consumes the witness via [`ValidatedStart::into_parts`].
    ///
    /// # Performance
    ///
    /// Allocates `O(b)` visited storage and frontier capacity.
    pub(super) fn new(start: NodeId<G>, witness: ValidatedStart<'_, G>) -> Self {
        let (start_index, bound) = witness.into_parts();
        let mut queue = Vec::with_capacity(bound);
        queue.push(start);
        let mut visited = vec![0; bound];
        visited[start_index] = 1;
        Self {
            visited,
            queue,
            head: 0,
        }
    }
}

#[cfg(feature = "alloc")]
impl<G> Sealed for SeededOwnedFrontier<G> where G: NodeIndex {}

#[cfg(feature = "alloc")]
impl<G> BfsStep<G> for SeededOwnedFrontier<G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    fn pop(&mut self) -> Option<NodeId<G>> {
        let node = self.queue.get(self.head).copied()?;
        self.head += 1;
        Some(node)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: NodeId<G>) {
        let target_index = graph.node_index(target);
        let visited_len = self.visited.len();
        let Some(slot) = self.visited.get_mut(target_index) else {
            neighbor_oob(target_index, visited_len)
        };
        if *slot == 0 {
            *slot = 1;
            self.queue.push(target);
        }
    }
}

/// Reports an out-of-bound neighbor index discovered during traversal.
///
/// Indicates the graph view violated its
/// [`OutgoingNeighborsGraph`](oxgraph_graph::OutgoingNeighborsGraph) contract
/// — a neighbor's dense index was at or past the `node_bound()` cached at
/// validation time. Panics with [`BfsError::NeighborIndexOutOfBounds`]'s
/// `Display` message so the failure reads consistently with all other BFS
/// validation errors.
///
/// Returns `!`, so callers can use it inline as the `else` arm of a
/// `let Some(_) = ... else { ... };` over a slice `get_mut`.
///
/// # Performance
///
/// `#[cold]` and `#[track_caller]`. Diverges via `panic!`; never returns.
#[cold]
#[track_caller]
pub(super) fn neighbor_oob(index: usize, bound: usize) -> ! {
    panic!("{}", BfsError::NeighborIndexOutOfBounds { index, bound })
}
