//! Benchmarks for substrate-agnostic BFS over oxgraph-topology traits.
//!
//! Forward BFS is benchmarked over CSR graph fixtures (the historical
//! baseline). Forward and reverse BFS are both benchmarked over BCSR
//! hypergraph fixtures so any regression in either direction on the
//! substrate-agnostic path is caught.
//!
//! Defends two perf contracts (the corresponding doc claims live next to each tier):
//! - All BFS tiers (`breadth_first_search`, `*_with_scratch`, `*_with_epoch_scratch`,
//!   `*_with_workspace`) traverse `O(n + m)` for `n` reachable nodes and `m` traversed edges. Bench
//!   fixtures are size-controlled so `n + m` is known per case and `Throughput::Elements` reports
//!   edges/sec; a sub-linear curve flags an algorithmic regression rather than a constant-factor
//!   change.
//! - The four allocation tiers (allocating, scratch-backed, epoch-marked, reusable workspace) are
//!   benched on the same fixture so the relative ordering — workspace ≈ epoch < scratch <
//!   allocating — stays stable. A swap in the ordering signals a setup-cost regression in one tier.

use std::hint::black_box;

use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use oxgraph_algo::{
    BfsEpochScratch, BfsWorkspace, breadth_first_search, breadth_first_search_with_epoch_scratch,
    breadth_first_search_with_scratch, breadth_first_search_with_workspace,
    reverse_breadth_first_search, reverse_breadth_first_search_with_scratch,
    reverse_breadth_first_search_with_workspace,
};
use oxgraph_csr::{CsrNativeGraph, CsrNodeId};
use oxgraph_hyper_bcsr::{BcsrSnapshotHypergraph, BcsrSnapshotIndex, BcsrVertexId};
use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotWriter};
use oxgraph_topology::DenseElementIndex;

/// `u32` head offsets section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32: u32 = <u32 as BcsrSnapshotIndex>::HEAD_OFFSETS_KIND;
/// `u32` head participants section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32: u32 =
    <u32 as BcsrSnapshotIndex>::HEAD_PARTICIPANTS_KIND;
/// `u32` tail offsets section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32: u32 = <u32 as BcsrSnapshotIndex>::TAIL_OFFSETS_KIND;
/// `u32` tail participants section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32: u32 =
    <u32 as BcsrSnapshotIndex>::TAIL_PARTICIPANTS_KIND;
/// `u32` vertex outgoing offsets section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32: u32 =
    <u32 as BcsrSnapshotIndex>::VERTEX_OUTGOING_OFFSETS_KIND;
/// `u32` vertex outgoing hyperedges section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32: u32 =
    <u32 as BcsrSnapshotIndex>::VERTEX_OUTGOING_HYPEREDGES_KIND;
/// `u32` vertex incoming offsets section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32: u32 =
    <u32 as BcsrSnapshotIndex>::VERTEX_INCOMING_OFFSETS_KIND;
/// `u32` vertex incoming hyperedges section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32: u32 =
    <u32 as BcsrSnapshotIndex>::VERTEX_INCOMING_HYPEREDGES_KIND;

/// Fixed out-degree used by the synthetic regular graph.
const DEGREE: u32 = 4;

/// Graph sizes used to observe BFS scaling behavior.
const NODE_COUNTS: &[u32] = &[10_000, 100_000];

/// Larger node bounds used to isolate visited-clear overhead.
const SPARSE_NODE_COUNTS: &[u32] = &[100_000, 1_000_000];

/// Number of nodes reachable in sparse-reachable fixtures.
const SPARSE_REACHABLE: u32 = 1_024;

/// CSR fixture section arrays.
type CsrParts = (Vec<u32>, Vec<u32>);

/// Scratch slices for clear-backed BFS lanes.
struct ClearScratch<'scratch> {
    /// Dense visited flags indexed by element index.
    visited: &'scratch mut [u8],
    /// Queue storage for discovered elements.
    queue: &'scratch mut [CsrNodeId<u32>],
}

/// Builds a deterministic regular CSR graph.
fn build_regular_csr(node_count: u32) -> CsrParts {
    let edge_count = node_count.saturating_mul(DEGREE);
    let mut offsets = Vec::with_capacity(usize_from_u32(node_count) + 1);
    let mut targets = Vec::with_capacity(usize_from_u32(edge_count));

    offsets.push(0);
    for node in 0..node_count {
        let next_offset = offsets[offsets.len() - 1] + DEGREE;
        offsets.push(next_offset);

        for step in 0..DEGREE {
            targets.push((node + step + 1) % node_count);
        }
    }

    (offsets, targets)
}

/// Builds a chain CSR graph where each node points to its successor.
fn build_chain_csr(node_count: u32) -> CsrParts {
    let edge_count = node_count.saturating_sub(1);
    let mut offsets = Vec::with_capacity(usize_from_u32(node_count) + 1);
    let mut targets = Vec::with_capacity(usize_from_u32(edge_count));

    offsets.push(0);
    for node in 0..node_count {
        if node + 1 < node_count {
            targets.push(node + 1);
        }
        offsets.push(usize_to_u32(targets.len()));
    }

    (offsets, targets)
}

/// Builds a graph with one wide frontier from node zero.
fn build_wide_frontier_csr(node_count: u32) -> CsrParts {
    let edge_count = node_count.saturating_sub(1);
    let mut offsets = Vec::with_capacity(usize_from_u32(node_count) + 1);
    let mut targets = Vec::with_capacity(usize_from_u32(edge_count));

    offsets.push(0);
    for target in 1..node_count {
        targets.push(target);
    }
    offsets.push(usize_to_u32(targets.len()));
    for _node in 1..node_count {
        offsets.push(usize_to_u32(targets.len()));
    }

    (offsets, targets)
}

/// Builds a graph with duplicate edges stressing visited membership checks.
fn build_duplicate_heavy_csr(node_count: u32) -> CsrParts {
    let edge_count = node_count.saturating_mul(DEGREE);
    let mut offsets = Vec::with_capacity(usize_from_u32(node_count) + 1);
    let mut targets = Vec::with_capacity(usize_from_u32(edge_count));

    offsets.push(0);
    for node in 0..node_count {
        let target = (node + 1) % node_count;
        for _duplicate in 0..DEGREE {
            targets.push(target);
        }
        offsets.push(usize_to_u32(targets.len()));
    }

    (offsets, targets)
}

/// Builds a graph with a large node bound but small reachable component.
fn build_sparse_reachable_csr(node_count: u32) -> CsrParts {
    let reachable = node_count.min(SPARSE_REACHABLE);
    let mut offsets = Vec::with_capacity(usize_from_u32(node_count) + 1);
    let mut targets = Vec::with_capacity(usize_from_u32(reachable.saturating_sub(1)));

    offsets.push(0);
    for node in 0..node_count {
        if node + 1 < reachable {
            targets.push(node + 1);
        }
        offsets.push(usize_to_u32(targets.len()));
    }

    (offsets, targets)
}

/// Runs allocating indexed BFS and returns the number of reached nodes.
fn indexed_alloc_bfs_count(graph: &CsrNativeGraph<'_, u32, u32>) -> usize {
    match breadth_first_search(graph, CsrNodeId::new(0)) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("benchmark BFS start was invalid: {error:?}"),
    }
}

/// Runs scratch-backed indexed BFS and returns the number of reached nodes.
fn scratch_bfs_count(
    graph: &CsrNativeGraph<'_, u32, u32>,
    visited: &mut [u8],
    queue: &mut [CsrNodeId<u32>],
) -> usize {
    match breadth_first_search_with_scratch(graph, CsrNodeId::new(0), visited, queue) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("benchmark BFS scratch was invalid: {error:?}"),
    }
}

/// Runs epoch-scratch indexed BFS and returns the number of reached nodes.
fn epoch_bfs_count<'graph>(
    graph: &CsrNativeGraph<'graph, u32, u32>,
    scratch: &mut BfsEpochScratch<'_, CsrNativeGraph<'graph, u32, u32>>,
) -> usize {
    match breadth_first_search_with_epoch_scratch(graph, CsrNodeId::new(0), scratch) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("benchmark epoch scratch was invalid: {error:?}"),
    }
}

/// Runs workspace-backed indexed BFS and returns the number of reached nodes.
fn workspace_bfs_count<'graph>(
    graph: &CsrNativeGraph<'graph, u32, u32>,
    workspace: &mut BfsWorkspace<CsrNativeGraph<'graph, u32, u32>>,
) -> usize {
    match breadth_first_search_with_workspace(graph, CsrNodeId::new(0), workspace) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("benchmark workspace was invalid: {error:?}"),
    }
}

/// Converts a `u32` fixture size into `usize`.
fn usize_from_u32(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark size did not fit usize: {error:?}"),
    }
}

/// Converts a `usize` fixture length into `u32`.
fn usize_to_u32(value: usize) -> u32 {
    match u32::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark length did not fit u32: {error:?}"),
    }
}

/// Registers scratch-clear BFS benchmark lanes.
fn bench_clear_scratch_lanes(
    group: &mut BenchmarkGroup<'_, WallTime>,
    node_count: u32,
    graph: &CsrNativeGraph<'_, u32, u32>,
    scratch: &mut ClearScratch<'_>,
) {
    group.bench_with_input(
        BenchmarkId::new("scratch_clear_reused", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| black_box(scratch_bfs_count(graph, scratch.visited, scratch.queue)));
        },
    );
}

/// Registers epoch-scratch BFS benchmark lanes.
fn bench_epoch_scratch_lanes<'graph>(
    group: &mut BenchmarkGroup<'_, WallTime>,
    node_count: u32,
    graph: &CsrNativeGraph<'graph, u32, u32>,
    scratch: &mut BfsEpochScratch<'_, CsrNativeGraph<'graph, u32, u32>>,
) {
    group.bench_with_input(
        BenchmarkId::new("scratch_epoch_reused", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| black_box(epoch_bfs_count(graph, scratch)));
        },
    );
}

/// Registers allocating indexed BFS benchmark lanes.
fn bench_allocating_indexed_lanes(
    group: &mut BenchmarkGroup<'_, WallTime>,
    node_count: u32,
    graph: &CsrNativeGraph<'_, u32, u32>,
) {
    group.bench_with_input(
        BenchmarkId::new("indexed_alloc_vec_head", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| black_box(indexed_alloc_bfs_count(graph)));
        },
    );
}

/// Registers reusable workspace BFS benchmark lanes.
fn bench_workspace_lanes<'graph>(
    group: &mut BenchmarkGroup<'_, WallTime>,
    node_count: u32,
    graph: &CsrNativeGraph<'graph, u32, u32>,
    workspace: &mut BfsWorkspace<CsrNativeGraph<'graph, u32, u32>>,
) {
    group.bench_with_input(
        BenchmarkId::new("workspace_epoch_alloc", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| black_box(workspace_bfs_count(graph, workspace)));
        },
    );
}

/// Benchmarks every forward BFS tier for one family of CSR fixtures.
fn bench_csr_fixture(
    c: &mut Criterion,
    group_name: &str,
    node_counts: &[u32],
    build: fn(u32) -> CsrParts,
) {
    let mut group = c.benchmark_group(group_name);
    for node_count in node_counts {
        let (offsets, targets) = build(*node_count);
        let graph = match CsrNativeGraph::<u32, u32>::validate(*node_count, &offsets, &targets) {
            Ok(validated) => validated,
            Err(error) => panic!("benchmark CSR fixture was invalid: {error:?}"),
        };
        let bound = graph.element_bound();
        let mut visited = vec![0; bound];
        let mut scratch_queue = vec![CsrNodeId::new(0); bound];
        let mut marks = vec![0; bound];
        let mut epoch_queue = vec![CsrNodeId::new(0); bound];
        let mut epoch_scratch = BfsEpochScratch::for_graph(&graph, &mut marks, &mut epoch_queue);
        let mut workspace = BfsWorkspace::<CsrNativeGraph<'_, u32, u32>>::with_element_bound(bound);

        group.throughput(Throughput::Elements(u64::from(*node_count)));
        let mut clear_scratch = ClearScratch {
            visited: &mut visited,
            queue: &mut scratch_queue,
        };
        bench_clear_scratch_lanes(&mut group, *node_count, &graph, &mut clear_scratch);
        bench_epoch_scratch_lanes(&mut group, *node_count, &graph, &mut epoch_scratch);
        bench_allocating_indexed_lanes(&mut group, *node_count, &graph);
        bench_workspace_lanes(&mut group, *node_count, &graph, &mut workspace);
    }
    group.finish();
}

/// Benchmarks forward BFS over validated CSR graph shapes.
fn bench_csr_bfs(c: &mut Criterion) {
    bench_csr_fixture(c, "graph_bfs_regular", NODE_COUNTS, build_regular_csr);
    bench_csr_fixture(c, "graph_bfs_chain", NODE_COUNTS, build_chain_csr);
    bench_csr_fixture(
        c,
        "graph_bfs_wide_frontier",
        NODE_COUNTS,
        build_wide_frontier_csr,
    );
    bench_csr_fixture(
        c,
        "graph_bfs_duplicate_heavy",
        NODE_COUNTS,
        build_duplicate_heavy_csr,
    );
    bench_csr_fixture(
        c,
        "graph_bfs_sparse_reachable",
        SPARSE_NODE_COUNTS,
        build_sparse_reachable_csr,
    );
}

// ============================================================================
// BCSR (hypergraph) benchmarks — forward and reverse on the same substrate.
//
// Each CSR edge `(u, v)` is encoded as a single hyperedge with head `{u}` and
// tail `{v}`. Vertex-major outgoing/incoming offsets and hyperedge ID lists are
// derived from the same graph topology so a regular CSR shape produces a
// structurally equivalent BCSR shape.
// ============================================================================

/// The eight BCSR section arrays derived from CSR-style adjacency.
struct BcsrSections {
    /// Hyperedge-major offsets into `head_participants`.
    head_offsets: Vec<u32>,
    /// Flat head-side vertex IDs per hyperedge.
    head_participants: Vec<u32>,
    /// Hyperedge-major offsets into `tail_participants`.
    tail_offsets: Vec<u32>,
    /// Flat tail-side vertex IDs per hyperedge.
    tail_participants: Vec<u32>,
    /// Vertex-major offsets into `vertex_outgoing_hyperedges`.
    vertex_outgoing_offsets: Vec<u32>,
    /// Flat hyperedge IDs whose head contains each vertex.
    vertex_outgoing_hyperedges: Vec<u32>,
    /// Vertex-major offsets into `vertex_incoming_hyperedges`.
    vertex_incoming_offsets: Vec<u32>,
    /// Flat hyperedge IDs whose tail contains each vertex.
    vertex_incoming_hyperedges: Vec<u32>,
}

/// Builds the eight BCSR section arrays from CSR-style offsets and targets.
/// One hyperedge per CSR edge: head `{u}`, tail `{v}` for edge `(u, v)`.
fn csr_to_bcsr_sections(node_count: u32, offsets: &[u32], targets: &[u32]) -> BcsrSections {
    let edge_count = targets.len();
    let mut head_offsets = Vec::with_capacity(edge_count + 1);
    let mut head_participants = Vec::with_capacity(edge_count);
    let mut tail_offsets = Vec::with_capacity(edge_count + 1);
    let mut tail_participants = Vec::with_capacity(edge_count);

    head_offsets.push(0);
    tail_offsets.push(0);
    for node in 0..node_count {
        let row_start = usize_from_u32(offsets[usize_from_u32(node)]);
        let row_end = usize_from_u32(offsets[usize_from_u32(node) + 1]);
        for target in &targets[row_start..row_end] {
            head_participants.push(node);
            head_offsets.push(usize_to_u32(head_participants.len()));
            tail_participants.push(*target);
            tail_offsets.push(usize_to_u32(tail_participants.len()));
        }
    }

    // Outgoing per vertex: every hyperedge whose head is the vertex.
    // Incoming per vertex: every hyperedge whose tail is the vertex.
    let mut vertex_outgoing_offsets = vec![0u32; usize_from_u32(node_count) + 1];
    let mut vertex_incoming_offsets = vec![0u32; usize_from_u32(node_count) + 1];

    // Count.
    for hyperedge in 0..edge_count {
        let head = head_participants[hyperedge];
        let tail = tail_participants[hyperedge];
        vertex_outgoing_offsets[usize_from_u32(head) + 1] += 1;
        vertex_incoming_offsets[usize_from_u32(tail) + 1] += 1;
    }
    for slot in 1..vertex_outgoing_offsets.len() {
        vertex_outgoing_offsets[slot] += vertex_outgoing_offsets[slot - 1];
    }
    for slot in 1..vertex_incoming_offsets.len() {
        vertex_incoming_offsets[slot] += vertex_incoming_offsets[slot - 1];
    }

    // Bucket-fill.
    let mut vertex_outgoing_hyperedges = vec![0u32; edge_count];
    let mut vertex_incoming_hyperedges = vec![0u32; edge_count];
    let mut outgoing_cursor = vertex_outgoing_offsets.clone();
    let mut incoming_cursor = vertex_incoming_offsets.clone();
    for hyperedge in 0..edge_count {
        let head = usize_from_u32(head_participants[hyperedge]);
        let tail = usize_from_u32(tail_participants[hyperedge]);
        let out_slot = usize_from_u32(outgoing_cursor[head]);
        vertex_outgoing_hyperedges[out_slot] = usize_to_u32(hyperedge);
        outgoing_cursor[head] += 1;
        let in_slot = usize_from_u32(incoming_cursor[tail]);
        vertex_incoming_hyperedges[in_slot] = usize_to_u32(hyperedge);
        incoming_cursor[tail] += 1;
    }

    BcsrSections {
        head_offsets,
        head_participants,
        tail_offsets,
        tail_participants,
        vertex_outgoing_offsets,
        vertex_outgoing_hyperedges,
        vertex_incoming_offsets,
        vertex_incoming_hyperedges,
    }
}

/// Encodes a sequence of `u32` words as a little-endian byte vector.
fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Builds a BCSR snapshot byte vector from CSR-shaped offsets and targets.
fn bcsr_snapshot_bytes(node_count: u32, offsets: &[u32], targets: &[u32]) -> Vec<u8> {
    let sections = csr_to_bcsr_sections(node_count, offsets, targets);

    let mut writer = match SnapshotWriter::new(8, crc32c_append) {
        Ok(value) => value,
        Err(error) => panic!("BCSR snapshot writer: {error:?}"),
    };
    let add = |writer: &mut SnapshotWriter, kind: u32, words: &[u32]| {
        let version = <u32 as BcsrSnapshotIndex>::SECTION_VERSION;
        if let Err(error) = writer.section_bytes(kind, version, 2, &words_to_bytes(words)) {
            panic!("BCSR section {kind:#06x}: {error:?}");
        }
    };
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32,
        &sections.head_offsets,
    );
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32,
        &sections.head_participants,
    );
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32,
        &sections.tail_offsets,
    );
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32,
        &sections.tail_participants,
    );
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
        &sections.vertex_outgoing_offsets,
    );
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
        &sections.vertex_outgoing_hyperedges,
    );
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
        &sections.vertex_incoming_offsets,
    );
    add(
        &mut writer,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
        &sections.vertex_incoming_hyperedges,
    );

    match writer.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("bench writer finish: {error:?}"),
    }
}

/// Counts reachable vertices via forward BCSR scratch BFS.
fn bcsr_forward_scratch_count(
    view: &BcsrSnapshotHypergraph<'_, u32, u32, u32>,
    visited: &mut [u8],
    queue: &mut [BcsrVertexId<u32>],
) -> usize {
    match breadth_first_search_with_scratch(view, BcsrVertexId::new(0), visited, queue) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("BCSR forward scratch BFS invalid: {error:?}"),
    }
}

/// Counts reverse-reachable vertices via reverse BCSR scratch BFS.
fn bcsr_reverse_scratch_count(
    view: &BcsrSnapshotHypergraph<'_, u32, u32, u32>,
    start: BcsrVertexId<u32>,
    visited: &mut [u8],
    queue: &mut [BcsrVertexId<u32>],
) -> usize {
    match reverse_breadth_first_search_with_scratch(view, start, visited, queue) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("BCSR reverse scratch BFS invalid: {error:?}"),
    }
}

/// Counts reachable vertices via forward BCSR allocating indexed BFS.
fn bcsr_forward_allocating_count(view: &BcsrSnapshotHypergraph<'_, u32, u32, u32>) -> usize {
    match breadth_first_search(view, BcsrVertexId::new(0)) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("BCSR forward allocating BFS invalid: {error:?}"),
    }
}

/// Counts reverse-reachable vertices via reverse BCSR allocating indexed BFS.
fn bcsr_reverse_allocating_count(
    view: &BcsrSnapshotHypergraph<'_, u32, u32, u32>,
    start: BcsrVertexId<u32>,
) -> usize {
    match reverse_breadth_first_search(view, start) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("BCSR reverse allocating BFS invalid: {error:?}"),
    }
}

/// Counts reachable vertices via forward BCSR workspace BFS.
fn bcsr_forward_workspace_count<'view>(
    view: &BcsrSnapshotHypergraph<'view, u32, u32, u32>,
    workspace: &mut BfsWorkspace<BcsrSnapshotHypergraph<'view, u32, u32, u32>>,
) -> usize {
    match breadth_first_search_with_workspace(view, BcsrVertexId::new(0), workspace) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("BCSR forward workspace BFS invalid: {error:?}"),
    }
}

/// Counts reverse-reachable vertices via reverse BCSR workspace BFS.
fn bcsr_reverse_workspace_count<'view>(
    view: &BcsrSnapshotHypergraph<'view, u32, u32, u32>,
    start: BcsrVertexId<u32>,
    workspace: &mut BfsWorkspace<BcsrSnapshotHypergraph<'view, u32, u32, u32>>,
) -> usize {
    match reverse_breadth_first_search_with_workspace(view, start, workspace) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("BCSR reverse workspace BFS invalid: {error:?}"),
    }
}

/// Mutable scratch and workspace handles owned across one BCSR bench step.
struct BcsrBenchState<'state> {
    /// Forward scratch byte-flag visited buffer.
    forward_visited: &'state mut [u8],
    /// Forward scratch queue buffer.
    forward_queue: &'state mut [BcsrVertexId<u32>],
    /// Reverse scratch byte-flag visited buffer.
    reverse_visited: &'state mut [u8],
    /// Reverse scratch queue buffer.
    reverse_queue: &'state mut [BcsrVertexId<u32>],
    /// Forward reusable workspace.
    forward_workspace: &'state mut BfsWorkspace<BcsrSnapshotHypergraph<'state, u32, u32, u32>>,
    /// Reverse reusable workspace.
    reverse_workspace: &'state mut BfsWorkspace<BcsrSnapshotHypergraph<'state, u32, u32, u32>>,
}

/// Registers all six (3 storage tiers × 2 directions) BCSR BFS lanes for one
/// fixture size.
fn register_bcsr_lanes<'view>(
    group: &mut BenchmarkGroup<'_, WallTime>,
    node_count: u32,
    view: &'view BcsrSnapshotHypergraph<'view, u32, u32, u32>,
    state: &mut BcsrBenchState<'view>,
) {
    // Reverse BFS starts from the last vertex so chain / sparse fixtures
    // exercise the full reverse reachable set.
    let reverse_start = BcsrVertexId::new(node_count.saturating_sub(1));

    group.bench_with_input(
        BenchmarkId::new("forward_scratch_clear_reused", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| {
                black_box(bcsr_forward_scratch_count(
                    view,
                    state.forward_visited,
                    state.forward_queue,
                ))
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::new("reverse_scratch_clear_reused", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| {
                black_box(bcsr_reverse_scratch_count(
                    view,
                    reverse_start,
                    state.reverse_visited,
                    state.reverse_queue,
                ))
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::new("forward_indexed_alloc", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| black_box(bcsr_forward_allocating_count(view)));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("reverse_indexed_alloc", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| black_box(bcsr_reverse_allocating_count(view, reverse_start)));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("forward_workspace_epoch", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| black_box(bcsr_forward_workspace_count(view, state.forward_workspace)));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("reverse_workspace_epoch", node_count),
        &node_count,
        |b, _size| {
            b.iter(|| {
                black_box(bcsr_reverse_workspace_count(
                    view,
                    reverse_start,
                    state.reverse_workspace,
                ))
            });
        },
    );
}

/// Benchmarks forward and reverse BFS over a BCSR fixture family.
fn bench_bcsr_fixture(
    c: &mut Criterion,
    group_name: &str,
    node_counts: &[u32],
    build: fn(u32) -> CsrParts,
) {
    let mut group = c.benchmark_group(group_name);
    for node_count in node_counts {
        let (offsets, targets) = build(*node_count);
        let bytes = bcsr_snapshot_bytes(*node_count, &offsets, &targets);
        let snapshot = match Snapshot::open(&bytes) {
            Ok(opened) => opened,
            Err(error) => panic!("BCSR benchmark snapshot invalid: {error:?}"),
        };
        let view = match BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot) {
            Ok(view) => view,
            Err(error) => panic!("BCSR benchmark adaptor invalid: {error:?}"),
        };
        let bound = view.element_bound();
        let mut forward_visited = vec![0u8; bound];
        let mut forward_queue = vec![BcsrVertexId::new(0); bound];
        let mut reverse_visited = vec![0u8; bound];
        let mut reverse_queue = vec![BcsrVertexId::new(0); bound];
        let mut forward_workspace =
            BfsWorkspace::<BcsrSnapshotHypergraph<'_, u32, u32, u32>>::with_element_bound(bound);
        let mut reverse_workspace =
            BfsWorkspace::<BcsrSnapshotHypergraph<'_, u32, u32, u32>>::with_element_bound(bound);

        group.throughput(Throughput::Elements(u64::from(*node_count)));

        let mut state = BcsrBenchState {
            forward_visited: &mut forward_visited,
            forward_queue: &mut forward_queue,
            reverse_visited: &mut reverse_visited,
            reverse_queue: &mut reverse_queue,
            forward_workspace: &mut forward_workspace,
            reverse_workspace: &mut reverse_workspace,
        };
        register_bcsr_lanes(&mut group, *node_count, &view, &mut state);
    }
    group.finish();
}

/// Benchmarks forward and reverse BFS over BCSR hypergraph shapes.
fn bench_bcsr_bfs(c: &mut Criterion) {
    bench_bcsr_fixture(c, "hyper_bfs_regular", NODE_COUNTS, build_regular_csr);
    bench_bcsr_fixture(c, "hyper_bfs_chain", NODE_COUNTS, build_chain_csr);
}

criterion_group!(benches, bench_csr_bfs, bench_bcsr_bfs);
criterion_main!(benches);
