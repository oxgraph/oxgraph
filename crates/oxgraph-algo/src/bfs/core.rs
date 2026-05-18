//! Private BFS traversal core.
//!
//! Houses the [`BfsStep`] storage-policy trait, the [`ExpansionDirection`]
//! direction-policy trait, the unified [`Bfs`] iterator wrapper that drives
//! any `BfsStep` along any direction, the validation-witness types
//! ([`ValidatedStart`] and [`ValidatedScratch`]) every indexed entry-point
//! must mint before construction, and the canonical
//! [`SeededByteFlagFrontier`], [`SeededEpochFrontier`],
//! [`SeededWorkspaceFrontier`], and [`SeededOwnedFrontier`] storage primitives.
//!
//! Each `Seeded*Frontier` is itself the indexed `BfsStep` — there are no
//! per-variant wrapper states. The variant files in `bfs/` provide only the
//! public iterator newtype, the `Iterator` delegation, and the
//! `breadth_first_search_*` and `reverse_breadth_first_search_*` entry points
//! that mint the witness, build the seeded frontier, and wrap it in [`Bfs`].
//!
//! Invariant chain: `ValidatedStart` / `ValidatedScratch` is constructed only
//! by validation; a `Seeded*Frontier` is constructed only by consuming the
//! corresponding witness via [`ValidatedStart::into_parts`] /
//! [`ValidatedScratch::into_parts`]; the seeded frontier's storage fields are
//! private to this module; therefore an indexed BFS step's storage cannot be
//! reached without traversal-time validation having succeeded. Witness fields
//! are also private to this module so siblings under `bfs/` cannot fabricate
//! one via struct-literal construction.
//!
//! Direction is encoded as a sealed [`ExpansionDirection`] policy. [`Forward`]
//! requires the topology view to expose
//! [`ElementSuccessors`](oxgraph_topology::ElementSuccessors) and walks
//! outgoing connections; [`Reverse`] requires
//! [`ElementPredecessors`](oxgraph_topology::ElementPredecessors) and walks
//! incoming connections. Both directions reuse the same storage policies.

#[cfg(feature = "alloc")]
use alloc::{collections::VecDeque, vec, vec::Vec};
use core::{iter::FusedIterator, marker::PhantomData};

use oxgraph_topology::{
    ContainsElement, ElementId, ElementIndex, ElementPredecessors, ElementSuccessors, TopologyBase,
};

use crate::bfs::BfsError;

/// Sealing supertrait for [`BfsStep`], [`ExpansionDirection`], and [`VisitedSet`].
///
/// Visible only inside `bfs/` (via `pub(super)` plus the private `mod core;`
/// declaration), so external crates cannot `impl Sealed for ...` and therefore
/// cannot implement the sealed sibling traits. Required so that those traits
/// can be `pub` (allowing them as bounds on `pub` BFS iterator types and
/// entry-function signatures) without external impls becoming reachable.
///
/// # Performance
///
/// `perf: unspecified` (marker trait — no runtime cost).
pub(super) trait Sealed {}

/// Storage-policy seam for breadth-first search.
///
/// Implementors own the visited set and frontier. [`Bfs`] holds the topology
/// reference and drives the implementor through one BFS yield per
/// `Iterator::next` call.
///
/// `try_visit_and_push` receives the topology view as an explicit argument
/// because indexed implementations need it to compute dense element indexes;
/// generic implementations ignore it. Holding the view in [`Bfs`] (not the
/// state) lets `Iterator::next` borrow the view and the state from disjoint
/// fields of `self`, satisfying the borrow checker without copying.
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
    G: TopologyBase,
{
    /// Removes and returns the next frontier element, or `None` when the
    /// frontier is empty.
    ///
    /// # Performance
    ///
    /// Amortized `O(1)` for every implementor in this crate. Idempotent on
    /// exhaustion: repeated calls after returning `None` continue to return
    /// `None` without advancing internal cursors.
    fn pop(&mut self) -> Option<ElementId<G>>;

    /// Marks `target` as visited if it was previously unvisited and pushes it
    /// onto the frontier in that case. Idempotent on already-visited elements.
    /// Implementations that need a dense index for `target` use `graph` to
    /// compute it; index-free implementations ignore `graph`.
    ///
    /// # Performance
    ///
    /// `O(1)` for indexed implementors (dense byte-flag or epoch-mark
    /// lookup); `O(log n)` for the `BTreeSet` fallback; expected `O(1)` for
    /// the `HashSet` fallback.
    fn try_visit_and_push(&mut self, graph: &G, target: ElementId<G>);
}

/// Direction-policy seam for breadth-first search.
///
/// [`Forward`] expands an element through
/// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors); [`Reverse`]
/// expands through
/// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors). The
/// associated iterator type is the topology's own borrowed expansion
/// iterator, so direction selection introduces no allocation and one
/// monomorphized call per `Bfs::next` step.
///
/// This trait is sealed: only [`Forward`] and [`Reverse`] implement it.
///
/// # Performance
///
/// `expand` should be `O(1)` to create the iterator and `O(k)` to yield `k`
/// elements, with no allocation; that contract follows directly from
/// `ElementSuccessors` / `ElementPredecessors`.
pub(super) trait ExpansionDirection<G>: Sealed
where
    G: TopologyBase,
{
    /// Iterator over elements reached from one input element along this
    /// direction.
    ///
    /// # Performance
    ///
    /// Advancing the iterator should be amortized `O(1)`.
    type Iter<'view>: Iterator<Item = ElementId<G>>
    where
        G: 'view;

    /// Returns elements reachable from `element` along this direction.
    ///
    /// # Performance
    ///
    /// Expected `O(1)` to create the iterator; yielding `k` elements is
    /// expected `O(k)`.
    fn expand(graph: &G, element: ElementId<G>) -> Self::Iter<'_>;
}

/// Forward-direction expansion policy: walks outgoing connections from each
/// element via [`ElementSuccessors`](oxgraph_topology::ElementSuccessors).
///
/// Used by [`breadth_first_search`](crate::breadth_first_search) and the
/// other `breadth_first_search_*` entry points.
///
/// # Performance
///
/// Zero-sized; carries no runtime state. Monomorphizes to the topology's
/// `element_successors` call site.
pub(super) struct Forward;

impl Sealed for Forward {}

impl<G> ExpansionDirection<G> for Forward
where
    G: ElementSuccessors,
{
    type Iter<'view>
        = <G as ElementSuccessors>::Successors<'view>
    where
        G: 'view;

    fn expand(graph: &G, element: ElementId<G>) -> Self::Iter<'_> {
        <G as ElementSuccessors>::element_successors(graph, element)
    }
}

/// Reverse-direction expansion policy: walks incoming connections to each
/// element via [`ElementPredecessors`](oxgraph_topology::ElementPredecessors).
///
/// Used by [`reverse_breadth_first_search`](crate::reverse_breadth_first_search)
/// and the other `reverse_breadth_first_search_*` entry points.
///
/// # Performance
///
/// Zero-sized; carries no runtime state. Monomorphizes to the topology's
/// `element_predecessors` call site.
pub(super) struct Reverse;

impl Sealed for Reverse {}

impl<G> ExpansionDirection<G> for Reverse
where
    G: ElementPredecessors,
{
    type Iter<'view>
        = <G as ElementPredecessors>::Predecessors<'view>
    where
        G: 'view;

    fn expand(graph: &G, element: ElementId<G>) -> Self::Iter<'_> {
        <G as ElementPredecessors>::element_predecessors(graph, element)
    }
}

/// Breadth-first traversal iterator over any [`BfsStep`] storage policy along
/// any [`ExpansionDirection`].
///
/// All BFS variants exposed by this crate are type aliases over `Bfs`
/// instantiated with a particular state type and direction. The single
/// [`Iterator`] impl below is therefore the only place the BFS loop body
/// lives.
///
/// # Performance
///
/// Per-yield cost is `O(deg(element))` expansion-target inspections plus the
/// `BfsStep` impl's amortized pop and visit-and-push cost. The wrapper itself
/// adds no work beyond field projection and one direct call into
/// `D::expand`.
pub(super) struct Bfs<'graph, G, S, D>
where
    G: TopologyBase,
    S: BfsStep<G>,
    D: ExpansionDirection<G>,
{
    /// Topology view being traversed.
    graph: &'graph G,
    /// Storage policy carrying visited set and frontier.
    state: S,
    /// Zero-sized direction selector; monomorphizes to a single
    /// `D::expand` call site per yield.
    _direction: PhantomData<D>,
}

impl<'graph, G, S, D> Bfs<'graph, G, S, D>
where
    G: TopologyBase,
    S: BfsStep<G>,
    D: ExpansionDirection<G>,
{
    /// Wraps a topology view and a constructed [`BfsStep`] state.
    ///
    /// `pub(super)` because callers always reach this constructor through a
    /// variant-specific `breadth_first_search_*` /
    /// `reverse_breadth_first_search_*` entry point that has already
    /// validated inputs and built the state via the corresponding
    /// `Seeded*Frontier::new` (indexed) or directly (set-based fallbacks).
    ///
    /// # Performance
    ///
    /// `O(1)` field assignment.
    pub(super) const fn new(graph: &'graph G, state: S) -> Self {
        Self {
            graph,
            state,
            _direction: PhantomData,
        }
    }
}

impl<G, S, D> Iterator for Bfs<'_, G, S, D>
where
    G: TopologyBase,
    S: BfsStep<G>,
    D: ExpansionDirection<G>,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        let element = self.state.pop()?;
        for target in D::expand(self.graph, element) {
            self.state.try_visit_and_push(self.graph, target);
        }
        Some(element)
    }
}

// Sound for every `BfsStep` impl in this crate: each `pop` is idempotent on
// exhaustion (queue cursors don't advance once empty), so once `next` returns
// `None` it continues to return `None`.
impl<G, S, D> FusedIterator for Bfs<'_, G, S, D>
where
    G: TopologyBase,
    S: BfsStep<G>,
    D: ExpansionDirection<G>,
{
}

/// Witness that a `start` element is contained in a topology view and maps
/// inside its `element_bound()`.
///
/// Constructed only via [`ValidatedStart::new`], which runs the validation
/// body. Fields are private to this module, so siblings under `bfs/` cannot
/// fabricate a witness via struct-literal construction. Branded with both the
/// view borrow lifetime and the view type, so a witness minted for one
/// `&'graph G` cannot be passed to a state instantiated for a different view
/// or borrow.
///
/// Used by allocating indexed entry-points (`indexed.rs`, `workspace.rs`)
/// that allocate per-traversal storage themselves and only need a validated
/// start position.
///
/// # Performance
///
/// Holds two `usize` fields and a zero-sized brand. No runtime overhead
/// versus a free `validate_indexed_start` helper.
#[cfg(feature = "alloc")]
#[must_use = "a validation witness encodes a runtime check; consume it via Seeded*Frontier::new"]
pub(super) struct ValidatedStart<'graph, G> {
    /// Cached dense index of the validated start element.
    start_index: usize,
    /// Cached `graph.element_bound()` observed during validation.
    bound: usize,
    /// Brands the witness against the view borrow lifetime and the view
    /// type. `PhantomData<&'graph G>` covaries in `'graph`, which is what we
    /// want — the witness must not outlive the borrow it was minted against.
    _graph: PhantomData<&'graph G>,
}

#[cfg(feature = "alloc")]
impl<'graph, G> ValidatedStart<'graph, G>
where
    G: ContainsElement + ElementIndex,
{
    /// Validates that `start` is contained in `graph` and maps inside the
    /// dense element bound.
    ///
    /// # Errors
    ///
    /// Returns [`BfsError::StartElementNotContained`] when
    /// `graph.contains_element(start)` is false, or
    /// [`BfsError::StartIndexOutOfBounds`] when `graph.element_index(start)`
    /// is at or past `graph.element_bound()`.
    ///
    /// # Performance
    ///
    /// `O(1)` plus the cost of `graph.contains_element` and
    /// `graph.element_index`.
    pub(super) fn new(graph: &'graph G, start: ElementId<G>) -> Result<Self, BfsError> {
        let bound = graph.element_bound();
        if !graph.contains_element(start) {
            return Err(BfsError::StartElementNotContained);
        }
        let start_index = graph.element_index(start);
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

    /// Returns the cached `graph.element_bound()` observed during validation.
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

/// Witness that caller-provided scratch is at least `graph.element_bound()`
/// and that `start` is contained in the view and maps inside the bound.
///
/// Constructed only via [`ValidatedScratch::new`], which runs the validation
/// body. Fields are private to this module. Branded with both the view
/// borrow lifetime and the view type, so a witness minted for one
/// `&'graph G` cannot be passed to a state instantiated for a different view
/// or borrow.
///
/// Used by no-allocation indexed entry-points (`scratch.rs`, `epoch.rs`)
/// that consume caller-provided slices and need both start-element validation
/// and scratch-capacity validation in a single pass.
///
/// # Performance
///
/// Holds two `usize` fields and a zero-sized brand. Calls
/// `graph.element_bound()` exactly once and inlines the start-element checks
/// using the cached bound; no second view query is required.
#[must_use = "a validation witness encodes a runtime check; consume it via Seeded*Frontier::new"]
pub(super) struct ValidatedScratch<'graph, G> {
    /// Cached dense index of the validated start element.
    start_index: usize,
    /// Cached `graph.element_bound()` observed during validation.
    bound: usize,
    /// Brands the witness against the view borrow lifetime and the view
    /// type.
    _graph: PhantomData<&'graph G>,
}

impl<'graph, G> ValidatedScratch<'graph, G>
where
    G: ContainsElement + ElementIndex,
{
    /// Validates that the caller-provided scratch slices fit
    /// `graph.element_bound()` and that `start` lives inside the bound.
    ///
    /// # Errors
    ///
    /// Returns the first matching variant of [`BfsError`] in this order:
    /// [`BfsError::VisitedTooSmall`], [`BfsError::QueueTooSmall`],
    /// [`BfsError::StartElementNotContained`],
    /// [`BfsError::StartIndexOutOfBounds`].
    ///
    /// # Performance
    ///
    /// `O(1)` plus the cost of `graph.contains_element` and
    /// `graph.element_index`.
    pub(super) fn new(
        graph: &'graph G,
        start: ElementId<G>,
        visited_len: usize,
        queue_len: usize,
    ) -> Result<Self, BfsError> {
        let bound = graph.element_bound();
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
        if !graph.contains_element(start) {
            return Err(BfsError::StartElementNotContained);
        }
        let start_index = graph.element_index(start);
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
    G: ElementIndex,
{
    /// Dense visited flags indexed by `ElementIndex::element_index`.
    visited: &'scratch mut [u8],
    /// Queue storage for discovered elements.
    queue: &'scratch mut [ElementId<G>],
    /// Next queue slot to yield.
    head: usize,
    /// One-past-last initialized queue slot.
    tail: usize,
}

impl<'scratch, G> SeededByteFlagFrontier<'scratch, G>
where
    G: ContainsElement + ElementIndex,
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
        queue: &'scratch mut [ElementId<G>],
        start: ElementId<G>,
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

impl<G> Sealed for SeededByteFlagFrontier<'_, G> where G: ElementIndex {}

impl<G> BfsStep<G> for SeededByteFlagFrontier<'_, G>
where
    G: ContainsElement + ElementIndex,
{
    fn pop(&mut self) -> Option<ElementId<G>> {
        if self.head == self.tail {
            return None;
        }
        let element = self.queue[self.head];
        self.head += 1;
        Some(element)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: ElementId<G>) {
        let target_index = graph.element_index(target);
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
    G: ElementIndex,
{
    /// Dense visited epoch marks indexed by `ElementIndex::element_index`.
    marks: &'borrow mut [u32],
    /// Queue storage for discovered elements.
    queue: &'borrow mut [ElementId<G>],
    /// Traversal epoch treated as visited.
    epoch: u32,
    /// Next queue slot to yield.
    head: usize,
    /// One-past-last initialized queue slot.
    tail: usize,
}

impl<'borrow, G> SeededEpochFrontier<'borrow, G>
where
    G: ContainsElement + ElementIndex,
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
        queue: &'borrow mut [ElementId<G>],
        epoch: u32,
        start: ElementId<G>,
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

impl<G> Sealed for SeededEpochFrontier<'_, G> where G: ElementIndex {}

impl<G> BfsStep<G> for SeededEpochFrontier<'_, G>
where
    G: ContainsElement + ElementIndex,
{
    fn pop(&mut self) -> Option<ElementId<G>> {
        if self.head == self.tail {
            return None;
        }
        let element = self.queue[self.head];
        self.head += 1;
        Some(element)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: ElementId<G>) {
        let target_index = graph.element_index(target);
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
/// The wrapping `BfsWorkspace` is responsible for `ensure_element_bound` and
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
    G: ElementIndex,
{
    /// Dense visited epoch marks indexed by `ElementIndex::element_index`.
    marks: &'workspace mut [u32],
    /// Queue storage for discovered elements.
    queue: &'workspace mut Vec<ElementId<G>>,
    /// Traversal epoch treated as visited.
    epoch: u32,
    /// Next queue slot to yield.
    head: usize,
}

#[cfg(feature = "alloc")]
impl<'workspace, G> SeededWorkspaceFrontier<'workspace, G>
where
    G: ContainsElement + ElementIndex,
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
        queue: &'workspace mut Vec<ElementId<G>>,
        epoch: u32,
        start: ElementId<G>,
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
impl<G> Sealed for SeededWorkspaceFrontier<'_, G> where G: ElementIndex {}

#[cfg(feature = "alloc")]
impl<G> BfsStep<G> for SeededWorkspaceFrontier<'_, G>
where
    G: ContainsElement + ElementIndex,
{
    fn pop(&mut self) -> Option<ElementId<G>> {
        let element = self.queue.get(self.head).copied()?;
        self.head += 1;
        Some(element)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: ElementId<G>) {
        let target_index = graph.element_index(target);
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
    G: ElementIndex,
{
    /// Dense visited flags indexed by `ElementIndex::element_index`.
    visited: Vec<u8>,
    /// Elements discovered in yield order.
    queue: Vec<ElementId<G>>,
    /// Next queue slot to yield.
    head: usize,
}

#[cfg(feature = "alloc")]
impl<G> SeededOwnedFrontier<G>
where
    G: ContainsElement + ElementIndex,
{
    /// Allocates `vec![0; bound]` for visited, primes `vec![start]` for the
    /// queue, and marks the validated start index.
    ///
    /// Consumes the witness via [`ValidatedStart::into_parts`].
    ///
    /// # Performance
    ///
    /// Allocates `O(b)` visited storage and frontier capacity.
    pub(super) fn new(start: ElementId<G>, witness: ValidatedStart<'_, G>) -> Self {
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
impl<G> Sealed for SeededOwnedFrontier<G> where G: ElementIndex {}

#[cfg(feature = "alloc")]
impl<G> BfsStep<G> for SeededOwnedFrontier<G>
where
    G: ContainsElement + ElementIndex,
{
    fn pop(&mut self) -> Option<ElementId<G>> {
        let element = self.queue.get(self.head).copied()?;
        self.head += 1;
        Some(element)
    }

    fn try_visit_and_push(&mut self, graph: &G, target: ElementId<G>) {
        let target_index = graph.element_index(target);
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

/// Visited-set policy for the set-based BFS fallbacks.
///
/// Implementors own the visited container; [`GenericState`] supplies the
/// `VecDeque` frontier so the two set-based variants in `generic_btree.rs`
/// and `generic_hash.rs` share one [`BfsStep`] body.
///
/// # Performance
///
/// `insert` cost is implementor-defined: `O(log n)` for `BTreeSet`, expected
/// `O(1)` for `HashSet`.
#[cfg(feature = "alloc")]
pub(super) trait VisitedSet<T>: Default {
    /// Inserts `value`; returns `true` when the element was newly added,
    /// `false` when already present.
    ///
    /// # Performance
    ///
    /// Implementor-defined; see the trait-level docs.
    fn insert(&mut self, value: T) -> bool;
}

/// Owned `VecDeque` frontier paired with a generic visited set for
/// arbitrary-ID BFS. Direction-agnostic: drives both forward and reverse
/// traversals.
///
/// The two set-based fallbacks (`BTreeSet` in `generic_btree.rs`, `HashSet`
/// in `generic_hash.rs`) share this state through the [`VisitedSet`] policy.
///
/// # Performance
///
/// `pop` is amortized `O(1)`; `try_visit_and_push` is the visited set's
/// `insert` cost.
#[cfg(feature = "alloc")]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(super) struct GenericState<G, V>
where
    G: TopologyBase,
    V: VisitedSet<ElementId<G>>,
{
    /// Frontier: elements discovered but not yet yielded.
    queue: VecDeque<ElementId<G>>,
    /// Already-discovered elements.
    visited: V,
}

#[cfg(feature = "alloc")]
impl<G, V> GenericState<G, V>
where
    G: TopologyBase,
    V: VisitedSet<ElementId<G>>,
{
    /// Seeds the frontier and visited set with `start`.
    ///
    /// # Performance
    ///
    /// One visited-set insertion plus one `VecDeque::push_back`. See the
    /// concrete [`VisitedSet`] impl for the insertion cost.
    pub(super) fn new(start: ElementId<G>) -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(start);
        let mut visited = V::default();
        visited.insert(start);
        Self { queue, visited }
    }
}

#[cfg(feature = "alloc")]
impl<G, V> Sealed for GenericState<G, V>
where
    G: TopologyBase,
    V: VisitedSet<ElementId<G>>,
{
}

#[cfg(feature = "alloc")]
impl<G, V> BfsStep<G> for GenericState<G, V>
where
    G: TopologyBase,
    V: VisitedSet<ElementId<G>>,
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

/// Monotonic traversal-epoch counter with an automatic overflow reset.
///
/// Both [`crate::bfs::BfsEpochScratch`] and [`crate::bfs::BfsWorkspace`]
/// share this state: a non-zero `u32` epoch that marks "visited this
/// traversal" against a caller-owned `[u32]` mark slice. The method
/// [`Self::advance`] is the single place that handles epoch increment and
/// `u32::MAX` overflow (which clears the marks back to zero and resets the
/// epoch to `1`).
///
/// # Performance
///
/// `advance` is `O(1)` except on overflow, where it clears the supplied
/// `marks` slice in `O(marks.len())`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct EpochCounter {
    /// Current non-zero traversal epoch. Zero is reserved for "never
    /// visited"; the first call to [`Self::advance`] returns `1`.
    epoch: u32,
}

impl EpochCounter {
    /// Creates a counter at epoch `0` (so the first [`Self::advance`]
    /// returns `1`). Provided so callers in `const` context can construct
    /// without going through [`Default`].
    ///
    /// # Performance
    ///
    /// `O(1)` field initialization.
    pub(super) const fn new() -> Self {
        Self { epoch: 0 }
    }

    /// Advances to the next traversal epoch and returns it.
    ///
    /// On `u32` overflow, clears every entry in `marks` to zero and resets
    /// the epoch to `1`, preserving the invariant that mark `0` represents
    /// "never visited" and any non-zero mark equal to the current epoch
    /// represents "visited this traversal".
    ///
    /// # Performance
    ///
    /// `O(1)` unless the epoch overflows, where the mark clear is
    /// `O(marks.len())`.
    pub(super) fn advance(&mut self, marks: &mut [u32]) -> u32 {
        if self.epoch == u32::MAX {
            marks.fill(0);
            self.epoch = 1;
        } else {
            self.epoch += 1;
        }
        self.epoch
    }
}

/// Reports an out-of-bound expansion-target index discovered during traversal.
///
/// Indicates the topology view violated its
/// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors) or
/// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors) contract —
/// an expanded element's dense index was at or past the `element_bound()`
/// cached at validation time. Panics with
/// [`BfsError::NeighborIndexOutOfBounds`]'s `Display` message so the failure
/// reads consistently with all other BFS validation errors.
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
