//! Tests for directed BFS over oxgraph-graph traits.

#[cfg(feature = "std")]
use oxgraph_algo::breadth_first_search_generic_hash;
use oxgraph_algo::{
    BfsEpochScratch, BfsError, breadth_first_search_with_epoch_scratch,
    breadth_first_search_with_scratch,
};
#[cfg(feature = "alloc")]
use oxgraph_algo::{
    BfsWorkspace, breadth_first_search, breadth_first_search_generic,
    breadth_first_search_with_workspace,
};
use oxgraph_csr::{
    CsrError, CsrGraph, CsrNodeId, CsrSnapshotError, SNAPSHOT_KIND_CSR_OFFSETS,
    SNAPSHOT_KIND_CSR_TARGETS,
};
use oxgraph_graph::{NodeId, NodeIndex, OutgoingNeighborsGraph};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotError};
use oxgraph_topology::{ContainsElement, ElementIndex, TopologyBase};
use proptest::prelude::*;

/// Error returned while opening snapshot-backed CSR fixtures.
#[derive(Debug, Eq, PartialEq)]
enum SnapshotFixtureError {
    /// Snapshot validation failed.
    Snapshot(SnapshotError),
    /// CSR snapshot adaptor failed.
    Adaptor(CsrSnapshotError),
    /// BFS construction failed.
    Bfs(BfsError),
}

impl From<SnapshotError> for SnapshotFixtureError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<CsrSnapshotError> for SnapshotFixtureError {
    fn from(error: CsrSnapshotError) -> Self {
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
#[derive(Debug)]
struct FixtureGraph {
    /// Direct outgoing neighbor nodes per node.
    outgoing_neighbors: &'static [&'static [Node]],
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

impl OutgoingNeighborsGraph for FixtureGraph {
    type OutNeighbors<'view>
        = core::iter::Copied<core::slice::Iter<'view, Node>>
    where
        Self: 'view;

    fn outgoing_neighbors(&self, node: Node) -> Self::OutNeighbors<'_> {
        self.outgoing_neighbors[node.0].iter().copied()
    }
}

/// Returns a graph shaped like `0 -> {1, 2}`, `1 -> {3}`, `2 -> {3}`.
fn fixture() -> FixtureGraph {
    static OUT_N_0: &[Node] = &[Node(1), Node(2)];
    static OUT_N_1: &[Node] = &[Node(3)];
    static OUT_N_2: &[Node] = &[Node(3)];
    static OUT_N_3: &[Node] = &[];
    static OUTGOING_NEIGHBORS: &[&[Node]] = &[OUT_N_0, OUT_N_1, OUT_N_2, OUT_N_3];

    FixtureGraph {
        outgoing_neighbors: OUTGOING_NEIGHBORS,
    }
}

/// Runs scratch-backed BFS and collects the order for assertions.
fn scratch_order<G>(graph: &G, start: NodeId<G>) -> Result<Vec<NodeId<G>>, BfsError>
where
    G: NodeIndex + OutgoingNeighborsGraph + oxgraph_graph::ContainsNode,
{
    let bound = graph.node_bound();
    let mut visited = vec![0; bound];
    let mut queue = vec![start; bound];
    Ok(breadth_first_search_with_scratch(graph, start, &mut visited, &mut queue)?.collect())
}

/// Runs epoch-scratch-backed BFS and collects the order for assertions.
fn epoch_order<G>(graph: &G, start: NodeId<G>) -> Result<Vec<NodeId<G>>, BfsError>
where
    G: NodeIndex + OutgoingNeighborsGraph + oxgraph_graph::ContainsNode,
{
    let bound = graph.node_bound();
    let mut marks = vec![0; bound];
    let mut queue = vec![start; bound];
    let mut scratch = BfsEpochScratch::for_graph(graph, &mut marks, &mut queue);
    Ok(breadth_first_search_with_epoch_scratch(graph, start, &mut scratch)?.collect())
}

/// Runs workspace-backed BFS and collects the order for assertions.
#[cfg(feature = "alloc")]
fn workspace_order<G>(graph: &G, start: NodeId<G>) -> Result<Vec<NodeId<G>>, BfsError>
where
    G: NodeIndex + OutgoingNeighborsGraph + oxgraph_graph::ContainsNode,
{
    let mut workspace = BfsWorkspace::new();
    Ok(breadth_first_search_with_workspace(graph, start, &mut workspace)?.collect())
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
fn epoch_bfs_matches_scratch_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(epoch_order(&graph, Node(0)), scratch_order(&graph, Node(0)));
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
fn workspace_bfs_matches_scratch_bfs_on_trait_fixture() {
    let graph = fixture();

    assert_eq!(
        workspace_order(&graph, Node(0)),
        scratch_order(&graph, Node(0))
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
fn scratch_bfs_rejects_uncontained_start_node() {
    let graph = fixture();
    let mut visited = [0; 4];
    let mut queue = [Node(0); 4];

    assert_eq!(
        breadth_first_search_with_scratch(&graph, Node(4), &mut visited, &mut queue).err(),
        Some(BfsError::StartNodeNotContained)
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

/// Misbehaving fixture that yields a neighbor index past `node_bound()` from
/// node 0, exercising the structured `NeighborIndexOutOfBounds` panic.
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

impl OutgoingNeighborsGraph for OutOfBoundFixture {
    type OutNeighbors<'view>
        = core::iter::Copied<core::slice::Iter<'view, Node>>
    where
        Self: 'view;

    fn outgoing_neighbors(&self, node: Node) -> Self::OutNeighbors<'_> {
        // Node 0 yields a neighbor whose index (7) is past node_bound (2).
        // Node 1 yields no neighbors.
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
        panic!("scratch traversal must panic on out-of-bound neighbor")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("neighbor node index 7 is outside node index bound 2"),
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
        panic!("epoch traversal must panic on out-of-bound neighbor")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("neighbor node index 7 is outside node index bound 2"),
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
        panic!("allocating traversal must panic on out-of-bound neighbor")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("neighbor node index 7 is outside node index bound 2"),
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
        panic!("workspace traversal must panic on out-of-bound neighbor")
    };
    let message = panic_message(&*payload);
    assert!(
        message.contains("neighbor node index 7 is outside node index bound 2"),
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
    assert_eq!(workspace.node_bound_capacity(), 4);

    let second = breadth_first_search_with_workspace(&graph, Node(2), &mut workspace)
        .map(std::iter::Iterator::collect::<Vec<_>>);
    assert_eq!(second, Ok(vec![Node(2), Node(3)]));
}

#[test]
fn bfs_runs_over_csr_graph() -> Result<(), CsrError> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 3, 3];

    let graph = CsrGraph::validate(4, OFFSETS, TARGETS)?;

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
fn allocating_bfs_runs_over_csr_graph() -> Result<(), CsrError> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 3, 3];

    let graph = CsrGraph::validate(4, OFFSETS, TARGETS)?;

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
fn hash_bfs_runs_over_csr_graph() -> Result<(), CsrError> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 3, 3];

    let graph = CsrGraph::validate(4, OFFSETS, TARGETS)?;

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
fn snapshot_csr_order(bytes: &[u8]) -> Result<Vec<CsrNodeId>, SnapshotFixtureError> {
    let snapshot = Snapshot::open(bytes)?;
    let graph = CsrGraph::from_snapshot(&snapshot)?;
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
        SNAPSHOT_KIND_CSR_OFFSETS,
        0,
        2,
        words_to_bytes(&[0, 2, 3, 4, 4]),
    ) {
        panic!("offsets section: {error:?}");
    }
    if let Err(error) = builder.add_section(
        SNAPSHOT_KIND_CSR_TARGETS,
        0,
        2,
        words_to_bytes(&[1, 2, 3, 3]),
    ) {
        panic!("targets section: {error:?}");
    }
    builder.finish()
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

        let graph = match CsrGraph::validate(node_count, &offsets, &targets) {
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
}
