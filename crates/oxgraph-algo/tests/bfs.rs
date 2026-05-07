//! Tests for substrate-agnostic BFS over oxgraph-topology traits.
//!
//! BFS is exercised against three substrates:
//!
//! - an in-test `FixtureGraph` that impls only the topology traits BFS needs, covering both forward
//!   and reverse directions;
//! - the `oxgraph-csr::CsrGraph` graph layout (forward only — CSR is forward-only by design);
//! - the `oxgraph-hyper-bcsr::BcsrHypergraph` hypergraph layout (forward and reverse).

use oxgraph_algo::{
    BfsEpochScratch, BfsError, breadth_first_search_with_epoch_scratch,
    breadth_first_search_with_scratch, reverse_breadth_first_search_with_epoch_scratch,
    reverse_breadth_first_search_with_scratch,
};
#[cfg(feature = "alloc")]
use oxgraph_algo::{
    BfsWorkspace, breadth_first_search, breadth_first_search_generic,
    breadth_first_search_with_workspace, reverse_breadth_first_search,
    reverse_breadth_first_search_generic, reverse_breadth_first_search_with_workspace,
};
#[cfg(feature = "std")]
use oxgraph_algo::{breadth_first_search_generic_hash, reverse_breadth_first_search_generic_hash};
use oxgraph_csr::{
    CsrError, CsrNativeGraph, CsrNodeId, CsrSnapshotError, CsrSnapshotGraph,
    SNAPSHOT_KIND_CSR_OFFSETS_U32, SNAPSHOT_KIND_CSR_TARGETS_U32,
};
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHypergraph, BcsrSnapshotError, BcsrVertexId, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS, SNAPSHOT_KIND_BCSR_TAIL_OFFSETS,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotError};
use oxgraph_topology::{
    ContainsElement, ElementId, ElementIndex, ElementPredecessors, ElementSuccessors, TopologyBase,
};
use proptest::prelude::*;
use zerocopy::byteorder::{LE, U32};

/// Error returned while opening snapshot-backed CSR fixtures.
#[derive(Debug, Eq, PartialEq)]
enum SnapshotFixtureError {
    /// Snapshot validation failed.
    Snapshot(SnapshotError),
    /// CSR snapshot adaptor failed.
    Adaptor(CsrSnapshotError<u32, u32>),
    /// BFS construction failed.
    Bfs(BfsError),
}

impl From<SnapshotError> for SnapshotFixtureError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<CsrSnapshotError<u32, u32>> for SnapshotFixtureError {
    fn from(error: CsrSnapshotError<u32, u32>) -> Self {
        Self::Adaptor(error)
    }
}

impl std::fmt::Display for SnapshotFixtureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot validation failed: {error}"),
            Self::Adaptor(error) => write!(formatter, "CSR adaptor failed: {error}"),
            Self::Bfs(error) => write!(formatter, "BFS construction failed: {error}"),
        }
    }
}

impl std::error::Error for SnapshotFixtureError {}

/// Node identifier used by the fixture graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Node(usize);

/// Edge identifier used by the fixture graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Edge(usize);

/// Small graph fixture implementing only the traits BFS requires.
///
/// Carries both successor and predecessor adjacency so the same fixture
/// covers forward and reverse traversal end-to-end.
#[derive(Debug)]
struct FixtureGraph {
    /// Direct outgoing neighbor nodes per node.
    outgoing_neighbors: &'static [&'static [Node]],
    /// Direct incoming neighbor nodes per node.
    incoming_neighbors: &'static [&'static [Node]],
}

impl TopologyBase for FixtureGraph {
    type ElementId = Node;
    type RelationId = Edge;
}

impl ElementIndex for FixtureGraph {
    fn element_bound(&self) -> usize {
        self.outgoing_neighbors.len()
    }

    fn element_index(&self, element: Node) -> usize {
        element.0
    }
}

impl ContainsElement for FixtureGraph {
    fn contains_element(&self, element: Node) -> bool {
        element.0 < self.outgoing_neighbors.len()
    }
}

impl ElementSuccessors for FixtureGraph {
    type Successors<'view>
        = core::iter::Copied<core::slice::Iter<'view, Node>>
    where
        Self: 'view;

    fn element_successors(&self, node: Node) -> Self::Successors<'_> {
        self.outgoing_neighbors[node.0].iter().copied()
    }
}

impl ElementPredecessors for FixtureGraph {
    type Predecessors<'view>
        = core::iter::Copied<core::slice::Iter<'view, Node>>
    where
        Self: 'view;

    fn element_predecessors(&self, node: Node) -> Self::Predecessors<'_> {
        self.incoming_neighbors[node.0].iter().copied()
    }
}

/// Returns a graph shaped like `0 -> {1, 2}`, `1 -> {3}`, `2 -> {3}`.
///
/// Reverse adjacency is the obvious transpose:
/// `0 <- {}`, `1 <- {0}`, `2 <- {0}`, `3 <- {1, 2}`.
fn fixture() -> FixtureGraph {
    static OUT_N_0: &[Node] = &[Node(1), Node(2)];
    static OUT_N_1: &[Node] = &[Node(3)];
    static OUT_N_2: &[Node] = &[Node(3)];
    static OUT_N_3: &[Node] = &[];
    static OUTGOING_NEIGHBORS: &[&[Node]] = &[OUT_N_0, OUT_N_1, OUT_N_2, OUT_N_3];

    static IN_N_0: &[Node] = &[];
    static IN_N_1: &[Node] = &[Node(0)];
    static IN_N_2: &[Node] = &[Node(0)];
    static IN_N_3: &[Node] = &[Node(1), Node(2)];
    static INCOMING_NEIGHBORS: &[&[Node]] = &[IN_N_0, IN_N_1, IN_N_2, IN_N_3];

    FixtureGraph {
        outgoing_neighbors: OUTGOING_NEIGHBORS,
        incoming_neighbors: INCOMING_NEIGHBORS,
    }
}

/// Runs forward scratch-backed BFS and collects the order for assertions.
fn scratch_order<G>(graph: &G, start: ElementId<G>) -> Result<Vec<ElementId<G>>, BfsError>
where
    G: ContainsElement + ElementSuccessors + ElementIndex,
{
    let bound = graph.element_bound();
    let mut visited = vec![0; bound];
    let mut queue = vec![start; bound];
    Ok(breadth_first_search_with_scratch(graph, start, &mut visited, &mut queue)?.collect())
}

/// Runs reverse scratch-backed BFS and collects the order for assertions.
fn reverse_scratch_order<G>(graph: &G, start: ElementId<G>) -> Result<Vec<ElementId<G>>, BfsError>
where
    G: ContainsElement + ElementPredecessors + ElementIndex,
{
    let bound = graph.element_bound();
    let mut visited = vec![0; bound];
    let mut queue = vec![start; bound];
    Ok(
        reverse_breadth_first_search_with_scratch(graph, start, &mut visited, &mut queue)?
            .collect(),
    )
}

/// Runs forward epoch-scratch-backed BFS and collects the order.
fn epoch_order<G>(graph: &G, start: ElementId<G>) -> Result<Vec<ElementId<G>>, BfsError>
where
    G: ContainsElement + ElementSuccessors + ElementIndex,
{
    let bound = graph.element_bound();
    let mut marks = vec![0; bound];
    let mut queue = vec![start; bound];
    let mut scratch = BfsEpochScratch::for_graph(graph, &mut marks, &mut queue);
    Ok(breadth_first_search_with_epoch_scratch(graph, start, &mut scratch)?.collect())
}

/// Runs reverse epoch-scratch-backed BFS and collects the order.
fn reverse_epoch_order<G>(graph: &G, start: ElementId<G>) -> Result<Vec<ElementId<G>>, BfsError>
where
    G: ContainsElement + ElementPredecessors + ElementIndex,
{
    let bound = graph.element_bound();
    let mut marks = vec![0; bound];
    let mut queue = vec![start; bound];
    let mut scratch = BfsEpochScratch::for_graph(graph, &mut marks, &mut queue);
    Ok(reverse_breadth_first_search_with_epoch_scratch(graph, start, &mut scratch)?.collect())
}

/// Runs forward workspace-backed BFS and collects the order.
#[cfg(feature = "alloc")]
fn workspace_order<G>(graph: &G, start: ElementId<G>) -> Result<Vec<ElementId<G>>, BfsError>
where
    G: ContainsElement + ElementSuccessors + ElementIndex,
{
    let mut workspace = BfsWorkspace::new();
    Ok(breadth_first_search_with_workspace(graph, start, &mut workspace)?.collect())
}

/// Runs reverse workspace-backed BFS and collects the order.
#[cfg(feature = "alloc")]
fn reverse_workspace_order<G>(graph: &G, start: ElementId<G>) -> Result<Vec<ElementId<G>>, BfsError>
where
    G: ContainsElement + ElementPredecessors + ElementIndex,
{
    let mut workspace = BfsWorkspace::new();
    Ok(reverse_breadth_first_search_with_workspace(graph, start, &mut workspace)?.collect())
}

#[test]
fn bfs_runs_over_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        scratch_order(&graph, Node(0)),
        Ok(vec![Node(0), Node(1), Node(2), Node(3)])
    );
}

#[test]
fn reverse_bfs_runs_over_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        reverse_scratch_order(&graph, Node(3)),
        Ok(vec![Node(3), Node(1), Node(2), Node(0)])
    );
}

#[test]
fn epoch_bfs_matches_scratch_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(epoch_order(&graph, Node(0)), scratch_order(&graph, Node(0)));
}

#[test]
fn reverse_epoch_bfs_matches_reverse_scratch_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        reverse_epoch_order(&graph, Node(3)),
        reverse_scratch_order(&graph, Node(3))
    );
}

#[cfg(feature = "alloc")]
#[test]
fn allocating_bfs_matches_generic_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        breadth_first_search(&graph, Node(0)).map(std::iter::Iterator::collect::<Vec<_>>),
        Ok(breadth_first_search_generic(&graph, Node(0)).collect::<Vec<_>>())
    );
}

#[cfg(feature = "alloc")]
#[test]
fn reverse_allocating_bfs_matches_reverse_generic_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        reverse_breadth_first_search(&graph, Node(3)).map(std::iter::Iterator::collect::<Vec<_>>),
        Ok(reverse_breadth_first_search_generic(&graph, Node(3)).collect::<Vec<_>>())
    );
}

#[cfg(feature = "alloc")]
#[test]
fn workspace_bfs_matches_scratch_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        workspace_order(&graph, Node(0)),
        scratch_order(&graph, Node(0))
    );
}

#[cfg(feature = "alloc")]
#[test]
fn reverse_workspace_bfs_matches_reverse_scratch_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        reverse_workspace_order(&graph, Node(3)),
        reverse_scratch_order(&graph, Node(3))
    );
}

#[cfg(feature = "std")]
#[test]
fn hash_bfs_matches_generic_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        breadth_first_search_generic_hash(&graph, Node(0)).collect::<Vec<_>>(),
        breadth_first_search_generic(&graph, Node(0)).collect::<Vec<_>>()
    );
}

#[cfg(feature = "std")]
#[test]
fn reverse_hash_bfs_matches_reverse_generic_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        reverse_breadth_first_search_generic_hash(&graph, Node(3)).collect::<Vec<_>>(),
        reverse_breadth_first_search_generic(&graph, Node(3)).collect::<Vec<_>>()
    );
}

#[test]
fn scratch_bfs_rejects_small_visited_slice() {
    let graph = fixture();
    let mut visited = [0; 3];
    let mut queue = [Node(0); 4];

    assert_eq!(
        breadth_first_search_with_scratch(&graph, Node(0), &mut visited, &mut queue).err(),
        Some(BfsError::VisitedTooSmall {
            needed: 4,
            actual: 3,
        })
    );
}

#[test]
fn scratch_bfs_rejects_small_queue_slice() {
    let graph = fixture();
    let mut visited = [0; 4];
    let mut queue = [Node(0); 3];

    assert_eq!(
        breadth_first_search_with_scratch(&graph, Node(0), &mut visited, &mut queue).err(),
        Some(BfsError::QueueTooSmall {
            needed: 4,
            actual: 3,
        })
    );
}

#[test]
fn scratch_bfs_rejects_uncontained_start_element() {
    let graph = fixture();
    let mut visited = [0; 4];
    let mut queue = [Node(0); 4];

    assert_eq!(
        breadth_first_search_with_scratch(&graph, Node(4), &mut visited, &mut queue).err(),
        Some(BfsError::StartElementNotContained)
    );
}

#[test]
fn scratch_bfs_reports_scratch_errors_before_start_validation() {
    let graph = fixture();
    let mut visited = [0; 3];
    let mut queue = [Node(0); 3];

    assert_eq!(
        breadth_first_search_with_scratch(&graph, Node(4), &mut visited, &mut queue).err(),
        Some(BfsError::VisitedTooSmall {
            needed: 4,
            actual: 3,
        })
    );
}

#[test]
fn scratch_bfs_reuses_and_clears_scratch() {
    let graph = fixture();
    let mut visited = [1; 4];
    let mut queue = [Node(3); 4];

    let first = breadth_first_search_with_scratch(&graph, Node(0), &mut visited, &mut queue)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(first, Ok(vec![Node(0), Node(1), Node(2), Node(3)]));

    let second = breadth_first_search_with_scratch(&graph, Node(2), &mut visited, &mut queue)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(second, Ok(vec![Node(2), Node(3)]));
}

#[test]
fn epoch_bfs_reuses_scratch_without_full_clear() {
    let graph = fixture();
    let mut marks = [0; 4];
    let mut queue = [Node(3); 4];
    let mut scratch = BfsEpochScratch::for_graph(&graph, &mut marks, &mut queue);

    let first = breadth_first_search_with_epoch_scratch(&graph, Node(0), &mut scratch)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(first, Ok(vec![Node(0), Node(1), Node(2), Node(3)]));

    let second = breadth_first_search_with_epoch_scratch(&graph, Node(2), &mut scratch)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(second, Ok(vec![Node(2), Node(3)]));
}

#[test]
fn reverse_epoch_bfs_shares_scratch_with_forward() {
    let graph = fixture();
    let mut marks = [0; 4];
    let mut queue = [Node(0); 4];
    let mut scratch = BfsEpochScratch::for_graph(&graph, &mut marks, &mut queue);

    // Forward then reverse using the same scratch — epoch advancement isolates
    // the two traversals without a full mark clear.
    let forward = breadth_first_search_with_epoch_scratch(&graph, Node(0), &mut scratch)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(forward, Ok(vec![Node(0), Node(1), Node(2), Node(3)]));

    let reverse = reverse_breadth_first_search_with_epoch_scratch(&graph, Node(3), &mut scratch)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(reverse, Ok(vec![Node(3), Node(1), Node(2), Node(0)]));
}

#[test]
fn epoch_bfs_rejects_small_mark_slice() {
    let graph = fixture();
    let mut marks = [0; 3];
    let mut queue = [Node(0); 4];
    let mut scratch = BfsEpochScratch::for_graph(&graph, &mut marks, &mut queue);

    assert_eq!(
        breadth_first_search_with_epoch_scratch(&graph, Node(0), &mut scratch).err(),
        Some(BfsError::VisitedTooSmall {
            needed: 4,
            actual: 3,
        })
    );
}

#[test]
fn epoch_bfs_rejects_small_queue_slice() {
    let graph = fixture();
    let mut marks = [0; 4];
    let mut queue = [Node(0); 3];
    let mut scratch = BfsEpochScratch::for_graph(&graph, &mut marks, &mut queue);

    assert_eq!(
        breadth_first_search_with_epoch_scratch(&graph, Node(0), &mut scratch).err(),
        Some(BfsError::QueueTooSmall {
            needed: 4,
            actual: 3,
        })
    );
}

/// Misbehaving fixture that yields a successor index past `element_bound()`
/// from element 0, exercising the structured `NeighborIndexOutOfBounds` panic.
struct OutOfBoundFixture;

impl TopologyBase for OutOfBoundFixture {
    type ElementId = Node;
    type RelationId = Edge;
}

impl ElementIndex for OutOfBoundFixture {
    fn element_bound(&self) -> usize {
        2
    }

    fn element_index(&self, element: Node) -> usize {
        element.0
    }
}

impl ContainsElement for OutOfBoundFixture {
    fn contains_element(&self, element: Node) -> bool {
        element.0 < self.element_bound()
    }
}

impl ElementSuccessors for OutOfBoundFixture {
    type Successors<'view>
        = core::iter::Copied<core::slice::Iter<'view, Node>>
    where
        Self: 'view;

    fn element_successors(&self, node: Node) -> Self::Successors<'_> {
        // Element 0 yields a successor whose index (7) is past element_bound (2).
        // Element 1 yields no successors.
        static OUT_OF_BOUND: &[Node] = &[Node(7)];
        static EMPTY: &[Node] = &[];
        match node.0 {
            0 => OUT_OF_BOUND.iter().copied(),
            _ => EMPTY.iter().copied(),
        }
    }
}

/// Extracts the panic payload's message string in a form compatible with
/// what `panic!("{}", ...)` produces — `String` for formatted payloads,
/// `&'static str` for the literal-only path.
fn panic_message(payload: &(dyn core::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
        .unwrap_or("<non-string panic payload>")
}

#[test]
fn scratch_bfs_panics_with_neighbor_oob_message_on_bad_neighbor() {
    let graph = OutOfBoundFixture;
    let mut visited = [0u8; 2];
    let mut queue = [Node(0); 2];

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match breadth_first_search_with_scratch(&graph, Node(0), &mut visited, &mut queue) {
            Ok(traversal) => traversal.collect::<Vec<_>>(),
            Err(error) => {
                panic!("scratch BFS construction must succeed for a valid start: {error}")
            }
        }
    }));

    let Err(payload) = result else {
        panic!("scratch traversal must panic on out-of-bound expanded element")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("expanded element index 7 is outside element index bound 2"),
        "panic message did not match BfsError::NeighborIndexOutOfBounds Display: {message}"
    );
}

#[test]
fn epoch_bfs_panics_with_neighbor_oob_message_on_bad_neighbor() {
    let graph = OutOfBoundFixture;
    let mut marks = [0u32; 2];
    let mut queue = [Node(0); 2];
    let mut scratch = BfsEpochScratch::for_graph(&graph, &mut marks, &mut queue);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match breadth_first_search_with_epoch_scratch(&graph, Node(0), &mut scratch) {
            Ok(traversal) => traversal.collect::<Vec<_>>(),
            Err(error) => panic!("epoch BFS construction must succeed for a valid start: {error}"),
        }
    }));

    let Err(payload) = result else {
        panic!("epoch traversal must panic on out-of-bound expanded element")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("expanded element index 7 is outside element index bound 2"),
        "panic message did not match BfsError::NeighborIndexOutOfBounds Display: {message}"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn allocating_bfs_panics_with_neighbor_oob_message_on_bad_neighbor() {
    let graph = OutOfBoundFixture;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || match breadth_first_search(&graph, Node(0)) {
            Ok(traversal) => traversal.collect::<Vec<_>>(),
            Err(error) => {
                panic!("allocating BFS construction must succeed for a valid start: {error}")
            }
        },
    ));

    let Err(payload) = result else {
        panic!("allocating traversal must panic on out-of-bound expanded element")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("expanded element index 7 is outside element index bound 2"),
        "panic message did not match BfsError::NeighborIndexOutOfBounds Display: {message}"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn workspace_bfs_panics_with_neighbor_oob_message_on_bad_neighbor() {
    let graph = OutOfBoundFixture;
    let mut workspace = BfsWorkspace::new();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match breadth_first_search_with_workspace(&graph, Node(0), &mut workspace) {
            Ok(traversal) => traversal.collect::<Vec<_>>(),
            Err(error) => {
                panic!("workspace BFS construction must succeed for a valid start: {error}")
            }
        }
    }));

    let Err(payload) = result else {
        panic!("workspace traversal must panic on out-of-bound expanded element")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("expanded element index 7 is outside element index bound 2"),
        "panic message did not match BfsError::NeighborIndexOutOfBounds Display: {message}"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn workspace_bfs_reuses_owned_storage() {
    let graph = fixture();
    let mut workspace = BfsWorkspace::new();

    let first = breadth_first_search_with_workspace(&graph, Node(0), &mut workspace)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(first, Ok(vec![Node(0), Node(1), Node(2), Node(3)]));
    assert_eq!(workspace.element_bound_capacity(), 4);

    let second = breadth_first_search_with_workspace(&graph, Node(2), &mut workspace)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(second, Ok(vec![Node(2), Node(3)]));
}

#[cfg(feature = "alloc")]
#[test]
fn workspace_shares_between_forward_and_reverse() {
    let graph = fixture();
    let mut workspace = BfsWorkspace::new();

    let forward = breadth_first_search_with_workspace(&graph, Node(0), &mut workspace)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(forward, Ok(vec![Node(0), Node(1), Node(2), Node(3)]));

    let reverse = reverse_breadth_first_search_with_workspace(&graph, Node(3), &mut workspace)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(reverse, Ok(vec![Node(3), Node(1), Node(2), Node(0)]));
}

#[test]
fn bfs_runs_over_csr_graph() -> Result<(), CsrError<u32, u32>> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 3, 3];

    let graph = CsrNativeGraph::<u32, u32>::validate(4, OFFSETS, TARGETS)?;

    assert_eq!(
        scratch_order(&graph, CsrNodeId(0)),
        Ok(vec![CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)])
    );
    assert_eq!(
        epoch_order(&graph, CsrNodeId(0)),
        scratch_order(&graph, CsrNodeId(0))
    );

    Ok(())
}

#[cfg(feature = "alloc")]
#[test]
fn allocating_bfs_runs_over_csr_graph() -> Result<(), CsrError<u32, u32>> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 3, 3];

    let graph = CsrNativeGraph::<u32, u32>::validate(4, OFFSETS, TARGETS)?;

    assert_eq!(
        breadth_first_search(&graph, CsrNodeId(0)).map(std::iter::Iterator::collect::<Vec<_>>),
        Ok(vec![CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)])
    );
    assert_eq!(
        workspace_order(&graph, CsrNodeId(0)),
        scratch_order(&graph, CsrNodeId(0))
    );

    Ok(())
}

#[cfg(feature = "std")]
#[test]
fn hash_bfs_runs_over_csr_graph() -> Result<(), CsrError<u32, u32>> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 3, 3];

    let graph = CsrNativeGraph::<u32, u32>::validate(4, OFFSETS, TARGETS)?;

    assert_eq!(
        breadth_first_search_generic_hash(&graph, CsrNodeId(0)).collect::<Vec<_>>(),
        vec![CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)]
    );

    Ok(())
}

#[test]
fn default_bfs_runs_over_snapshot_sections() {
    let bytes = valid_snapshot_bytes();

    assert_eq!(
        snapshot_csr_order(&bytes),
        Ok(vec![CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)])
    );
}

/// Runs scratch-backed BFS on a CSR graph opened from snapshot fixture bytes.
fn snapshot_csr_order(bytes: &[u8]) -> Result<Vec<CsrNodeId<u32>>, SnapshotFixtureError> {
    let snapshot = Snapshot::open(bytes)?;
    let graph = CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot)?;
    scratch_order(&graph, CsrNodeId(0)).map_err(SnapshotFixtureError::Bfs)
}

/// Encodes a sequence of `u32` words as a little-endian byte vector.
fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Builds a valid v1 snapshot byte vector for BFS tests.
fn valid_snapshot_bytes() -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    if let Err(error) = builder.add_section(
        SNAPSHOT_KIND_CSR_OFFSETS_U32,
        0,
        2,
        words_to_bytes(&[0, 2, 3, 4, 4]),
    ) {
        panic!("offsets section: {error:?}");
    }
    if let Err(error) = builder.add_section(
        SNAPSHOT_KIND_CSR_TARGETS_U32,
        0,
        2,
        words_to_bytes(&[1, 2, 3, 3]),
    ) {
        panic!("targets section: {error:?}");
    }
    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("test builder finish: {error:?}"),
    }
}

// ============================================================================
// Hypergraph (BCSR) coverage — the key proof that algo runs unchanged on a
// non-graph substrate.
// ============================================================================

/// Error returned while opening snapshot-backed BCSR fixtures.
#[derive(Debug)]
enum BcsrFixtureError {
    /// Snapshot validation failed.
    Snapshot(SnapshotError),
    /// BCSR snapshot adaptor failed.
    Adaptor(BcsrSnapshotError),
    /// BCSR validation failed.
    Validation(BcsrError),
}

impl From<SnapshotError> for BcsrFixtureError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<BcsrSnapshotError> for BcsrFixtureError {
    fn from(error: BcsrSnapshotError) -> Self {
        Self::Adaptor(error)
    }
}

impl From<BcsrError> for BcsrFixtureError {
    fn from(error: BcsrError) -> Self {
        Self::Validation(error)
    }
}

impl std::fmt::Display for BcsrFixtureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot validation failed: {error}"),
            Self::Adaptor(error) => write!(formatter, "BCSR adaptor failed: {error}"),
            Self::Validation(error) => write!(formatter, "BCSR validation failed: {error}"),
        }
    }
}

impl std::error::Error for BcsrFixtureError {}

/// Builds a small directed hypergraph snapshot whose vertex-level reachability
/// matches the four-element fixture used elsewhere in this file:
///
/// `0 -> {1, 2}` via hyperedge 0 (head: {0}, tail: {1, 2}),
/// `1 -> {3}`   via hyperedge 1 (head: {1}, tail: {3}),
/// `2 -> {3}`   via hyperedge 2 (head: {2}, tail: {3}).
///
/// Forward BFS from vertex 0 reaches {0, 1, 2, 3}; reverse BFS from vertex 3
/// reaches {3, 1, 2, 0}.
fn bcsr_snapshot_bytes() -> Vec<u8> {
    // Hyperedge-major: per hyperedge, list of head vertices and tail vertices.
    // Hyperedge 0: head [0],    tail [1, 2]
    // Hyperedge 1: head [1],    tail [3]
    // Hyperedge 2: head [2],    tail [3]
    let head_offsets: &[u32] = &[0, 1, 2, 3];
    let head_participants: &[u32] = &[0, 1, 2];
    let tail_offsets: &[u32] = &[0, 2, 3, 4];
    let tail_participants: &[u32] = &[1, 2, 3, 3];

    // Vertex-major: per vertex, hyperedges where vertex is a head (outgoing)
    // or a tail (incoming).
    // Vertex 0: outgoing [0],    incoming []
    // Vertex 1: outgoing [1],    incoming [0]
    // Vertex 2: outgoing [2],    incoming [0]
    // Vertex 3: outgoing [],     incoming [1, 2]
    let vertex_outgoing_offsets: &[u32] = &[0, 1, 2, 3, 3];
    let vertex_outgoing_hyperedges: &[u32] = &[0, 1, 2];
    let vertex_incoming_offsets: &[u32] = &[0, 0, 1, 2, 4];
    let vertex_incoming_hyperedges: &[u32] = &[0, 0, 1, 2];

    let mut builder = SnapshotBuilder::new();

    let add = |builder: &mut SnapshotBuilder, kind: u32, words: &[u32]| {
        if let Err(error) = builder.add_section(kind, 0, 2, words_to_bytes(words)) {
            panic!("snapshot section {kind:#06x}: {error:?}");
        }
    };

    add(&mut builder, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, head_offsets);
    add(
        &mut builder,
        SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
        head_participants,
    );
    add(&mut builder, SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, tail_offsets);
    add(
        &mut builder,
        SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
        tail_participants,
    );
    add(
        &mut builder,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
        vertex_outgoing_offsets,
    );
    add(
        &mut builder,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
        vertex_outgoing_hyperedges,
    );
    add(
        &mut builder,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
        vertex_incoming_offsets,
    );
    add(
        &mut builder,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
        vertex_incoming_hyperedges,
    );

    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("test builder finish: {error:?}"),
    }
}

/// Opens a BCSR hypergraph from bytes produced by [`bcsr_snapshot_bytes`].
fn open_bcsr(bytes: &[u8]) -> Result<BcsrHypergraph<'_, U32<LE>>, BcsrFixtureError> {
    let snapshot = Snapshot::open(bytes)?;
    let view = BcsrHypergraph::<U32<LE>>::from_snapshot(&snapshot)?;
    Ok(view)
}

/// Opens a BCSR hypergraph from `bytes` or panics with the underlying error.
fn open_bcsr_or_panic(bytes: &[u8]) -> BcsrHypergraph<'_, U32<LE>> {
    match open_bcsr(bytes) {
        Ok(view) => view,
        Err(error) => panic!("BCSR fixture open failed: {error}"),
    }
}

/// Runs forward scratch BFS on a BCSR view or panics with the underlying error.
fn scratch_order_bcsr(
    view: &BcsrHypergraph<'_, U32<LE>>,
    start: BcsrVertexId,
) -> Vec<BcsrVertexId> {
    match scratch_order(view, start) {
        Ok(order) => order,
        Err(error) => panic!("forward scratch BFS on BCSR failed: {error}"),
    }
}

/// Runs reverse scratch BFS on a BCSR view or panics with the underlying error.
fn reverse_scratch_order_bcsr(
    view: &BcsrHypergraph<'_, U32<LE>>,
    start: BcsrVertexId,
) -> Vec<BcsrVertexId> {
    match reverse_scratch_order(view, start) {
        Ok(order) => order,
        Err(error) => panic!("reverse scratch BFS on BCSR failed: {error}"),
    }
}

#[test]
fn forward_bfs_runs_over_bcsr_hypergraph() {
    let bytes = bcsr_snapshot_bytes();
    let view = open_bcsr_or_panic(&bytes);

    let order = scratch_order_bcsr(&view, BcsrVertexId(0));
    assert_eq!(
        order,
        vec![
            BcsrVertexId(0),
            BcsrVertexId(1),
            BcsrVertexId(2),
            BcsrVertexId(3)
        ]
    );
}

#[test]
fn reverse_bfs_runs_over_bcsr_hypergraph() {
    let bytes = bcsr_snapshot_bytes();
    let view = open_bcsr_or_panic(&bytes);

    let order = reverse_scratch_order_bcsr(&view, BcsrVertexId(3));
    assert_eq!(
        order,
        vec![
            BcsrVertexId(3),
            BcsrVertexId(1),
            BcsrVertexId(2),
            BcsrVertexId(0)
        ]
    );
}

#[cfg(feature = "alloc")]
#[test]
fn allocating_bfs_matches_scratch_on_bcsr_hypergraph() {
    let bytes = bcsr_snapshot_bytes();
    let view = open_bcsr_or_panic(&bytes);

    let allocating = match breadth_first_search(&view, BcsrVertexId(0)) {
        Ok(traversal) => traversal.collect::<Vec<_>>(),
        Err(error) => panic!("forward indexed BFS on BCSR failed: {error}"),
    };
    let scratch = scratch_order_bcsr(&view, BcsrVertexId(0));
    assert_eq!(allocating, scratch);

    let reverse_allocating = match reverse_breadth_first_search(&view, BcsrVertexId(3)) {
        Ok(traversal) => traversal.collect::<Vec<_>>(),
        Err(error) => panic!("reverse indexed BFS on BCSR failed: {error}"),
    };
    let reverse_scratch = reverse_scratch_order_bcsr(&view, BcsrVertexId(3));
    assert_eq!(reverse_allocating, reverse_scratch);
}

#[cfg(feature = "std")]
#[test]
fn hash_bfs_runs_over_bcsr_hypergraph() {
    use std::collections::HashSet;

    let bytes = bcsr_snapshot_bytes();
    let view = open_bcsr_or_panic(&bytes);

    // BFS yield order through `HashSet` is implementation-defined; the
    // algorithm's contract is set semantics, not order. Collect directly into
    // a `HashSet` and compare — that is the property substrate-agnostic BFS
    // actually guarantees.
    let forward_set: HashSet<_> =
        breadth_first_search_generic_hash(&view, BcsrVertexId(0)).collect();
    let expected_forward: HashSet<_> = [
        BcsrVertexId(0),
        BcsrVertexId(1),
        BcsrVertexId(2),
        BcsrVertexId(3),
    ]
    .into_iter()
    .collect();
    assert_eq!(forward_set, expected_forward);

    let reverse_set: HashSet<_> =
        reverse_breadth_first_search_generic_hash(&view, BcsrVertexId(3)).collect();
    let expected_reverse: HashSet<_> = [
        BcsrVertexId(3),
        BcsrVertexId(1),
        BcsrVertexId(2),
        BcsrVertexId(0),
    ]
    .into_iter()
    .collect();
    assert_eq!(reverse_set, expected_reverse);
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Indexed BFS matches generic BFS for generated valid CSR graphs.
    #[test]
    fn default_bfs_matches_generic_bfs_for_valid_csr(
        degrees in proptest::collection::vec(0u32..4, 1..16),
        target_seed in proptest::collection::vec(0u32..64, 0..64),
    ) {
        let node_count = match u32::try_from(degrees.len()) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("node count conversion failed: {error:?}"))),
        };
        let mut offsets = Vec::with_capacity(degrees.len() + 1);
        offsets.push(0);
        let mut total = 0u32;
        for degree in &degrees {
            total += *degree;
            offsets.push(total);
        }

        let edge_count = match usize::try_from(total) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("edge count conversion failed: {error:?}"))),
        };
        let mut targets = Vec::with_capacity(edge_count);
        for index in 0..edge_count {
            let seed = target_seed.get(index).copied().unwrap_or(0);
            targets.push(seed % node_count);
        }

        let graph = match CsrNativeGraph::<u32, u32>::validate(node_count, &offsets, &targets) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("valid CSR rejected: {error:?}"))),
        };
        let indexed = match scratch_order(&graph, CsrNodeId(0)) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("scratch BFS failed: {error:?}"))),
        };
        let epoch = match epoch_order(&graph, CsrNodeId(0)) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("epoch BFS failed: {error:?}"))),
        };
        prop_assert_eq!(&indexed, &epoch);

        #[cfg(feature = "alloc")]
        {
            let generic = breadth_first_search_generic(&graph, CsrNodeId(0)).collect::<Vec<_>>();
            let allocating = match breadth_first_search(&graph, CsrNodeId(0)) {
                Ok(value) => value.collect::<Vec<_>>(),
                Err(error) => return Err(TestCaseError::fail(format!("allocating BFS failed: {error:?}"))),
            };
            let workspace = match workspace_order(&graph, CsrNodeId(0)) {
                Ok(value) => value,
                Err(error) => return Err(TestCaseError::fail(format!("workspace BFS failed: {error:?}"))),
            };

            prop_assert_eq!(&indexed, &generic);
            prop_assert_eq!(&indexed, &allocating);
            prop_assert_eq!(&indexed, &workspace);
        }

        #[cfg(feature = "std")]
        {
            let hash = breadth_first_search_generic_hash(&graph, CsrNodeId(0)).collect::<Vec<_>>();
            prop_assert_eq!(&indexed, &hash);
        }

        let mut seen = vec![false; graph.element_bound()];
        for node in indexed {
            let index = graph.element_index(node);
            prop_assert!(index < graph.element_bound());
            prop_assert!(!seen[index]);
            seen[index] = true;
        }
    }

    /// BFS distances are non-decreasing in visit order (layer monotonicity).
    ///
    /// If the BFS yields nodes in order `[v_0, v_1, ..., v_k]`, then for every
    /// adjacent pair the distance from the source is non-decreasing:
    /// `dist(v_i) <= dist(v_{i+1})`. Distances are computed by a reference
    /// BFS that uses the same forward expansion (`element_successors`) as the
    /// algorithms under test.
    #[test]
    fn bfs_visit_order_is_layer_monotone(
        degrees in proptest::collection::vec(0u32..4, 1..16),
        target_seed in proptest::collection::vec(0u32..64, 0..64),
    ) {
        let (offsets, targets, node_count) = build_csr_arrays(&degrees, &target_seed);
        let graph = match CsrNativeGraph::<u32, u32>::validate(node_count, &offsets, &targets) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("valid CSR rejected: {error:?}"))),
        };
        let order = match scratch_order(&graph, CsrNodeId(0)) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("scratch BFS failed: {error:?}"))),
        };
        let distances = reference_bfs_distances(&graph, CsrNodeId(0));
        for window in order.windows(2) {
            let prev_index = graph.element_index(window[0]);
            let next_index = graph.element_index(window[1]);
            let prev_dist = distances[prev_index];
            let next_dist = distances[next_index];
            prop_assert!(
                prev_dist <= next_dist,
                "layer monotonicity violated: dist({:?})={} > dist({:?})={}",
                window[0], prev_dist, window[1], next_dist,
            );
        }
    }

    /// BFS visits exactly the reachability set, no more and no fewer.
    ///
    /// The set of nodes BFS yields equals the set of nodes reachable from
    /// the source under forward expansion. A reference BFS that computes
    /// the reachability set independently is used as ground truth.
    #[test]
    fn bfs_visits_exactly_the_reachable_set(
        degrees in proptest::collection::vec(0u32..4, 1..16),
        target_seed in proptest::collection::vec(0u32..64, 0..64),
    ) {
        let (offsets, targets, node_count) = build_csr_arrays(&degrees, &target_seed);
        let graph = match CsrNativeGraph::<u32, u32>::validate(node_count, &offsets, &targets) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("valid CSR rejected: {error:?}"))),
        };
        let order = match scratch_order(&graph, CsrNodeId(0)) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("scratch BFS failed: {error:?}"))),
        };
        let reachable = reference_reachable_set(&graph, CsrNodeId(0));
        let visited: std::collections::BTreeSet<usize> =
            order.iter().map(|node| graph.element_index(*node)).collect();
        prop_assert_eq!(visited, reachable);
    }
}

/// Builds CSR offset and target arrays from generated degree and target seeds,
/// mirroring the construction used by the variant-equivalence proptest above.
/// Returns `(offsets, targets, node_count)`. The caller owns the storage and
/// can pass it through `CsrNativeGraph::validate` whose result borrows from it.
fn build_csr_arrays(degrees: &[u32], target_seed: &[u32]) -> (Vec<u32>, Vec<u32>, u32) {
    let node_count = u32::try_from(degrees.len()).unwrap_or(0);
    let mut offsets = Vec::with_capacity(degrees.len() + 1);
    offsets.push(0);
    let mut total = 0u32;
    for degree in degrees {
        total += *degree;
        offsets.push(total);
    }
    let edge_count = usize::try_from(total).unwrap_or(0);
    let mut targets = Vec::with_capacity(edge_count);
    for index in 0..edge_count {
        let seed = target_seed.get(index).copied().unwrap_or(0);
        targets.push(seed % node_count.max(1));
    }
    (offsets, targets, node_count)
}

/// Reference BFS that records distance from `source` for each reachable node,
/// using the same forward expansion (`element_successors`) as the algorithms
/// under test. Unreachable nodes are recorded as `usize::MAX` so callers can
/// detect them.
fn reference_bfs_distances<G>(graph: &G, source: ElementId<G>) -> Vec<usize>
where
    G: TopologyBase + ElementIndex + ElementSuccessors,
    ElementId<G>: Copy,
{
    let mut distances = vec![usize::MAX; graph.element_bound()];
    let mut queue = std::collections::VecDeque::new();
    distances[graph.element_index(source)] = 0;
    queue.push_back(source);
    while let Some(node) = queue.pop_front() {
        let dist_here = distances[graph.element_index(node)];
        for next in graph.element_successors(node) {
            let next_index = graph.element_index(next);
            if distances[next_index] == usize::MAX {
                distances[next_index] = dist_here + 1;
                queue.push_back(next);
            }
        }
    }
    distances
}

/// Reference reachability set computed by a parent-free flood-fill using the
/// same forward expansion as the algorithms under test.
fn reference_reachable_set<G>(graph: &G, source: ElementId<G>) -> std::collections::BTreeSet<usize>
where
    G: TopologyBase + ElementIndex + ElementSuccessors,
    ElementId<G>: Copy,
{
    let mut reached = std::collections::BTreeSet::new();
    let mut stack = vec![source];
    reached.insert(graph.element_index(source));
    while let Some(node) = stack.pop() {
        for next in graph.element_successors(node) {
            let next_index = graph.element_index(next);
            if reached.insert(next_index) {
                stack.push(next);
            }
        }
    }
    reached
}
