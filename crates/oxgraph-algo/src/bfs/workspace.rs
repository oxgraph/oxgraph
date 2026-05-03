//! Allocating reusable workspace for dense-indexed BFS (forward and reverse).

use alloc::vec::Vec;
use core::{iter::FusedIterator, marker::PhantomData};

use oxgraph_topology::{
    ContainsElement, ElementId, ElementIndex, ElementPredecessors, ElementSuccessors,
};

use crate::bfs::{
    BfsError,
    core::{Bfs, Forward, Reverse, SeededWorkspaceFrontier, ValidatedStart},
};

/// Owned reusable workspace for epoch-marked indexed BFS, branded to a
/// specific topology view type.
///
/// The workspace owns visited marks and queue storage so callers with `alloc`
/// can reuse memory across many traversals over the same view type without
/// manually managing slices. The `G` type parameter brands the workspace at
/// compile time: a `BfsWorkspace<ViewA>` cannot be passed to a traversal of
/// `ViewB`. This closes the cross-view reuse hazard by type rather than
/// by runtime check.
///
/// `PhantomData<fn() -> G>` is covariant in `G` (function-return position is
/// covariant) and crucially keeps the workspace's `Send` / `Sync`
/// independent of `G`'s thread-safety, so the workspace remains usable from
/// threads that `G` itself cannot cross.
///
/// The same workspace is usable for both forward and reverse traversal.
///
/// # Performance
///
/// Memory usage is `O(b)` for the largest element bound used with the
/// workspace. Reusing the workspace avoids repeated heap allocation and
/// avoids full visited clears except when the internal epoch overflows.
#[derive(Clone, Debug)]
pub struct BfsWorkspace<G>
where
    G: ContainsElement + ElementIndex,
{
    /// Dense visited epoch marks indexed by `ElementIndex::element_index`.
    marks: Vec<u32>,
    /// Queue storage for discovered elements.
    queue: Vec<ElementId<G>>,
    /// Current non-zero traversal epoch. Zero is reserved for "never visited".
    epoch: u32,
    /// Brands the workspace to a specific view type without coupling
    /// `Send`/`Sync` to `G`.
    _graph: PhantomData<fn() -> G>,
}

impl<G> Default for BfsWorkspace<G>
where
    G: ContainsElement + ElementIndex,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<G> BfsWorkspace<G>
where
    G: ContainsElement + ElementIndex,
{
    /// Creates an empty reusable BFS workspace branded to view type `G`.
    ///
    /// Storage grows on first use to fit `graph.element_bound()`. The view
    /// type is typically inferred from the workspace's first use; pass an
    /// explicit turbofish (`BfsWorkspace::<MyView>::new()`) when inference
    /// cannot pick it up.
    ///
    /// # Performance
    ///
    /// Runs in `O(1)` and performs no heap allocation.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            marks: Vec::new(),
            queue: Vec::new(),
            epoch: 0,
            _graph: PhantomData,
        }
    }

    /// Creates a reusable BFS workspace pre-sized for a specific view.
    ///
    /// Equivalent to [`BfsWorkspace::with_element_bound`] with
    /// `graph.element_bound()`, but lets the view type bind by inference
    /// from the borrowed reference.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(graph.element_bound())` mark storage and
    /// reserves `O(graph.element_bound())` queue capacity.
    #[must_use]
    pub fn for_graph(graph: &G) -> Self {
        Self::with_element_bound(graph.element_bound())
    }

    /// Creates a reusable BFS workspace with capacity for `element_bound`
    /// elements.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(element_bound)` mark storage and reserves
    /// `O(element_bound)` queue capacity.
    #[must_use]
    pub fn with_element_bound(element_bound: usize) -> Self {
        Self {
            marks: alloc::vec![0; element_bound],
            queue: Vec::with_capacity(element_bound),
            epoch: 0,
            _graph: PhantomData,
        }
    }

    /// Returns the mark capacity currently available without growing storage.
    ///
    /// # Performance
    ///
    /// Runs in `O(1)`.
    #[must_use]
    pub const fn element_bound_capacity(&self) -> usize {
        self.marks.len()
    }

    /// Ensures the workspace can support `element_bound` dense element indexes.
    ///
    /// # Performance
    ///
    /// Runs in `O(1)` when existing storage is large enough. Otherwise, grows
    /// mark storage and queue capacity by `O(element_bound)`.
    fn ensure_element_bound(&mut self, element_bound: usize) {
        if self.marks.len() < element_bound {
            self.marks.resize(element_bound, 0);
        }
        if self.queue.capacity() < element_bound {
            self.queue.reserve(element_bound - self.queue.capacity());
        }
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
    /// Runs in `O(1)` except when the epoch overflows, where it clears all mark
    /// entries in `O(self.element_bound_capacity())`.
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

/// Forward BFS iterator borrowing an owned reusable workspace.
///
/// Constructed only via [`breadth_first_search_with_workspace`]. The internal
/// storage policy ([`SeededWorkspaceFrontier`]) is intentionally not part of
/// the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` elements and `m` successor entries
/// inspected, traversal is `O(n + m)` and uses workspace-owned `O(b)` memory
/// for `b = graph.element_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearchWorkspace<'graph, 'workspace, G>
where
    G: ContainsElement + ElementSuccessors + ElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededWorkspaceFrontier<'workspace, G>, Forward>,
}

impl<G> Iterator for BreadthFirstSearchWorkspace<'_, '_, G>
where
    G: ContainsElement + ElementSuccessors + ElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearchWorkspace<'_, '_, G> where
    G: ContainsElement + ElementSuccessors + ElementIndex
{
}

/// Creates a forward indexed BFS traversal using a reusable owned workspace.
///
/// Traversal follows outgoing connections and yields each reachable element
/// at most once. The workspace grows as needed to cover `graph.element_bound()`
/// and is reused on later traversals.
///
/// `start` and all IDs returned by `graph.element_successors` must be valid
/// for `graph` and must map to indexes below `graph.element_bound()`. An
/// expansion-target index at or past the bound observed at construction time
/// will panic with [`BfsError::NeighborIndexOutOfBounds`]; this is a topology
/// contract violation, not a recoverable failure.
///
/// # Errors
///
/// Returns [`BfsError::StartElementNotContained`] when `start` is not
/// contained in `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start`
/// maps outside `graph.element_bound()`. Allocation failure is not
/// represented by this error and follows the allocator's behavior.
///
/// # Performance
///
/// Construction is `O(1)` when the workspace is already large enough, except
/// on epoch overflow where it clears `O(b)` mark entries. Traversal over a
/// reachable subgraph with `n` elements and `m` successor entries inspected is
/// `O(n + m)`.
pub fn breadth_first_search_with_workspace<'graph, 'workspace, G>(
    graph: &'graph G,
    start: ElementId<G>,
    workspace: &'workspace mut BfsWorkspace<G>,
) -> Result<BreadthFirstSearchWorkspace<'graph, 'workspace, G>, BfsError>
where
    G: ContainsElement + ElementSuccessors + ElementIndex,
{
    let witness = ValidatedStart::new(graph, start)?;
    let bound = witness.bound();
    workspace.ensure_element_bound(bound);
    let epoch = workspace.advance_epoch();
    let frontier = SeededWorkspaceFrontier::new(
        &mut workspace.marks,
        &mut workspace.queue,
        epoch,
        start,
        witness,
    );
    Ok(BreadthFirstSearchWorkspace {
        inner: Bfs::new(graph, frontier),
    })
}

/// Reverse BFS iterator borrowing an owned reusable workspace.
///
/// Constructed only via [`reverse_breadth_first_search_with_workspace`]. The
/// internal storage policy ([`SeededWorkspaceFrontier`]) is intentionally not
/// part of the public API.
///
/// # Performance
///
/// For a reverse-reachable subgraph with `n` elements and `m` predecessor
/// entries inspected, traversal is `O(n + m)` and uses workspace-owned `O(b)`
/// memory for `b = graph.element_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct ReverseBreadthFirstSearchWorkspace<'graph, 'workspace, G>
where
    G: ContainsElement + ElementPredecessors + ElementIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier and reverse
    /// direction selector.
    inner: Bfs<'graph, G, SeededWorkspaceFrontier<'workspace, G>, Reverse>,
}

impl<G> Iterator for ReverseBreadthFirstSearchWorkspace<'_, '_, G>
where
    G: ContainsElement + ElementPredecessors + ElementIndex,
{
    type Item = ElementId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for ReverseBreadthFirstSearchWorkspace<'_, '_, G> where
    G: ContainsElement + ElementPredecessors + ElementIndex
{
}

/// Creates a reverse indexed BFS traversal using a reusable owned workspace.
///
/// Traversal follows incoming connections and yields each reverse-reachable
/// element at most once. Same workspace / contract / error shape as
/// [`breadth_first_search_with_workspace`]; the only difference is that
/// expansion uses
/// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors) instead of
/// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors).
///
/// # Errors
///
/// Same as [`breadth_first_search_with_workspace`].
///
/// # Performance
///
/// Same as [`breadth_first_search_with_workspace`] with predecessor expansion
/// in place of successor expansion.
pub fn reverse_breadth_first_search_with_workspace<'graph, 'workspace, G>(
    graph: &'graph G,
    start: ElementId<G>,
    workspace: &'workspace mut BfsWorkspace<G>,
) -> Result<ReverseBreadthFirstSearchWorkspace<'graph, 'workspace, G>, BfsError>
where
    G: ContainsElement + ElementPredecessors + ElementIndex,
{
    let witness = ValidatedStart::new(graph, start)?;
    let bound = witness.bound();
    workspace.ensure_element_bound(bound);
    let epoch = workspace.advance_epoch();
    let frontier = SeededWorkspaceFrontier::new(
        &mut workspace.marks,
        &mut workspace.queue,
        epoch,
        start,
        witness,
    );
    Ok(ReverseBreadthFirstSearchWorkspace {
        inner: Bfs::new(graph, frontier),
    })
}
