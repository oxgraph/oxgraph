//! Allocating reusable workspace for dense-indexed BFS.

use alloc::vec::Vec;
use core::{iter::FusedIterator, marker::PhantomData};

use oxgraph_graph::{ContainsNode, NodeId, NodeIndex, OutgoingNeighborsGraph};

use crate::bfs::{
    BfsError,
    core::{Bfs, SeededWorkspaceFrontier, ValidatedStart},
};

/// Owned reusable workspace for epoch-marked indexed BFS, branded to a
/// specific graph type.
///
/// The workspace owns visited marks and queue storage so callers with `alloc`
/// can reuse memory across many traversals over the same graph type without
/// manually managing slices. The `G` type parameter brands the workspace at
/// compile time: a `BfsWorkspace<GraphA>` cannot be passed to a traversal of
/// `GraphB`. This closes the cross-graph reuse hazard by type rather than
/// by runtime check.
///
/// `PhantomData<fn() -> G>` is covariant in `G` (function-return position is
/// covariant) and crucially keeps the workspace's `Send` / `Sync`
/// independent of `G`'s thread-safety, so the workspace remains usable from
/// threads that `G` itself cannot cross.
///
/// # Performance
///
/// Memory usage is `O(b)` for the largest node bound used with the workspace.
/// Reusing the workspace avoids repeated heap allocation and avoids full visited
/// clears except when the internal epoch overflows.
#[derive(Clone, Debug)]
pub struct BfsWorkspace<G>
where
    G: ContainsNode + NodeIndex,
{
    /// Dense visited epoch marks indexed by `NodeIndex::node_index`.
    marks: Vec<u32>,
    /// Queue storage for discovered nodes.
    queue: Vec<NodeId<G>>,
    /// Current non-zero traversal epoch. Zero is reserved for "never visited".
    epoch: u32,
    /// Brands the workspace to a specific graph type without coupling
    /// `Send`/`Sync` to `G`.
    _graph: PhantomData<fn() -> G>,
}

impl<G> Default for BfsWorkspace<G>
where
    G: ContainsNode + NodeIndex,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<G> BfsWorkspace<G>
where
    G: ContainsNode + NodeIndex,
{
    /// Creates an empty reusable BFS workspace branded to graph type `G`.
    ///
    /// Storage grows on first use to fit `graph.node_bound()`. The graph type
    /// is typically inferred from the workspace's first use; pass an explicit
    /// turbofish (`BfsWorkspace::<MyGraph>::new()`) when inference cannot
    /// pick it up.
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

    /// Creates a reusable BFS workspace pre-sized for a specific graph view.
    ///
    /// Equivalent to [`BfsWorkspace::with_node_bound`] with
    /// `graph.node_bound()`, but lets the graph type bind by inference from
    /// the borrowed reference.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(graph.node_bound())` mark storage and
    /// reserves `O(graph.node_bound())` queue capacity.
    #[must_use]
    pub fn for_graph(graph: &G) -> Self {
        Self::with_node_bound(graph.node_bound())
    }

    /// Creates a reusable BFS workspace with capacity for `node_bound` nodes.
    ///
    /// # Performance
    ///
    /// Allocates and initializes `O(node_bound)` mark storage and reserves
    /// `O(node_bound)` queue capacity.
    #[must_use]
    pub fn with_node_bound(node_bound: usize) -> Self {
        Self {
            marks: alloc::vec![0; node_bound],
            queue: Vec::with_capacity(node_bound),
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
    pub const fn node_bound_capacity(&self) -> usize {
        self.marks.len()
    }

    /// Ensures the workspace can support `node_bound` dense node indexes.
    ///
    /// # Performance
    ///
    /// Runs in `O(1)` when existing storage is large enough. Otherwise, grows
    /// mark storage and queue capacity by `O(node_bound)`.
    fn ensure_node_bound(&mut self, node_bound: usize) {
        if self.marks.len() < node_bound {
            self.marks.resize(node_bound, 0);
        }
        if self.queue.capacity() < node_bound {
            self.queue.reserve(node_bound - self.queue.capacity());
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
    /// entries in `O(self.node_bound_capacity())`.
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

/// Directed BFS iterator borrowing an owned reusable workspace.
///
/// Constructed only via [`breadth_first_search_with_workspace`]. The internal
/// storage policy ([`SeededWorkspaceFrontier`]) is intentionally not part of
/// the public API.
///
/// # Performance
///
/// For a reachable subgraph with `n` nodes and `m` outgoing neighbor entries
/// inspected, traversal is `O(n + m)` and uses workspace-owned `O(b)` memory
/// for `b = graph.node_bound()`.
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct BreadthFirstSearchWorkspace<'graph, 'workspace, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    /// Underlying generic BFS driver carrying the seeded frontier.
    inner: Bfs<'graph, G, SeededWorkspaceFrontier<'workspace, G>>,
}

impl<G> Iterator for BreadthFirstSearchWorkspace<'_, '_, G>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    type Item = NodeId<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<G> FusedIterator for BreadthFirstSearchWorkspace<'_, '_, G> where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex
{
}

/// Creates an indexed BFS traversal using a reusable owned workspace.
///
/// Traversal follows outgoing neighbor nodes and yields each reachable node at most once.
/// The workspace grows as needed to cover `graph.node_bound()` and is reused on
/// later traversals.
///
/// `start` and all IDs returned by `graph.outgoing_neighbors` must be valid for
/// `graph` and must map to indexes below `graph.node_bound()`. A neighbor
/// index at or past the bound observed at construction time will panic with
/// [`BfsError::NeighborIndexOutOfBounds`]; this is a graph-view contract
/// violation, not a recoverable failure.
///
/// # Errors
///
/// Returns [`BfsError::StartNodeNotContained`] when `start` is not contained
/// in `graph`, or [`BfsError::StartIndexOutOfBounds`] when `start` maps
/// outside `graph.node_bound()`. Allocation failure is not represented by
/// this error and follows the allocator's behavior.
///
/// # Performance
///
/// Construction is `O(1)` when the workspace is already large enough, except on
/// epoch overflow where it clears `O(b)` mark entries. Traversal over a
/// reachable subgraph with `n` nodes and `m` outgoing neighbor entries inspected
/// is `O(n + m)`.
pub fn breadth_first_search_with_workspace<'graph, 'workspace, G>(
    graph: &'graph G,
    start: NodeId<G>,
    workspace: &'workspace mut BfsWorkspace<G>,
) -> Result<BreadthFirstSearchWorkspace<'graph, 'workspace, G>, BfsError>
where
    G: ContainsNode + OutgoingNeighborsGraph + NodeIndex,
{
    let witness = ValidatedStart::new(graph, start)?;
    let bound = witness.bound();
    workspace.ensure_node_bound(bound);
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
