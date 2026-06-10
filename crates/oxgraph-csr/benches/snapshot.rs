//! Benches for opening a CSR view from an `oxgraph-snapshot` and running
//! BFS over the borrowed sections.
//!
//! Defends one perf contract:
//! - Snapshot open plus `CsrSnapshotGraph::<u32, u32>::from_snapshot` plus forward BFS is `O(n +
//!   m)` for the ring fixture (each `n`-node ring carries `n` edges, so `n + m = 2n`). Benched at
//!   multiple node counts so the bytes-to-traversal pipeline cost tracks linearly; a regression in
//!   any sub-step (snapshot validation, CSR validation, or BFS) shows up as a sub-linear curve.

use std::hint::black_box;

use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use oxgraph_algo::breadth_first_search;
use oxgraph_csr::{
    CsrNodeId, CsrSnapshotGraph, SNAPSHOT_KIND_CSR_OFFSETS_U32, SNAPSHOT_KIND_CSR_TARGETS_U32,
};
use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotBuilder};

/// Builds a deterministic CSR ring graph of `node_count` nodes (each node
/// points to the next) and emits it as snapshot bytes.
fn ring_snapshot_bytes(node_count: u32) -> Vec<u8> {
    let mut offsets: Vec<u32> = Vec::with_capacity((node_count + 1) as usize);
    let mut targets: Vec<u32> = Vec::with_capacity(node_count as usize);
    for index in 0..node_count {
        offsets.push(index);
        targets.push((index + 1) % node_count.max(1));
    }
    offsets.push(node_count);

    let offsets_bytes: Vec<u8> = offsets.iter().flat_map(|word| word.to_le_bytes()).collect();
    let targets_bytes: Vec<u8> = targets.iter().flat_map(|word| word.to_le_bytes()).collect();

    let mut builder = SnapshotBuilder::new(crc32c_append);
    match builder.add_section(
        SNAPSHOT_KIND_CSR_OFFSETS_U32,
        oxgraph_csr::SNAPSHOT_CSR_SECTION_VERSION,
        2,
        offsets_bytes,
    ) {
        Ok(_) => {}
        Err(error) => panic!("bench offsets: {error:?}"),
    }
    match builder.add_section(
        SNAPSHOT_KIND_CSR_TARGETS_U32,
        oxgraph_csr::SNAPSHOT_CSR_SECTION_VERSION,
        2,
        targets_bytes,
    ) {
        Ok(_) => {}
        Err(error) => panic!("bench targets: {error:?}"),
    }
    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("bench builder finish: {error:?}"),
    }
}

/// Benchmarks `Snapshot::open` + `CsrSnapshotGraph::<u32, u32>::from_snapshot` together
/// (the cost of going from bytes to a traversable view).
fn bench_open(group: &mut BenchmarkGroup<'_, WallTime>) {
    for &node_count in &[64u32, 1024, 16_384] {
        let bytes = ring_snapshot_bytes(node_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            &bytes,
            |bencher, snapshot_bytes| {
                bencher.iter(|| {
                    let snapshot = match Snapshot::open(black_box(snapshot_bytes)) {
                        Ok(value) => value,
                        Err(error) => panic!("bench open: {error:?}"),
                    };
                    let graph = match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
                        Ok(value) => value,
                        Err(error) => panic!("bench from_snapshot: {error:?}"),
                    };
                    black_box(graph);
                });
            },
        );
    }
}

/// Benchmarks BFS over a snapshot-backed CSR ring graph.
fn bench_bfs(group: &mut BenchmarkGroup<'_, WallTime>) {
    for &node_count in &[64u32, 1024, 16_384] {
        let bytes = ring_snapshot_bytes(node_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            &bytes,
            |bencher, snapshot_bytes| {
                let snapshot = match Snapshot::open(snapshot_bytes) {
                    Ok(value) => value,
                    Err(error) => panic!("bench open: {error:?}"),
                };
                let graph = match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
                    Ok(value) => value,
                    Err(error) => panic!("bench from_snapshot: {error:?}"),
                };
                bencher.iter(|| {
                    let walk = match breadth_first_search(&graph, CsrNodeId::new(0)) {
                        Ok(value) => value,
                        Err(error) => panic!("bench bfs: {error:?}"),
                    };
                    let count = walk.count();
                    black_box(count);
                });
            },
        );
    }
}

/// Top-level criterion entry stitching together the open and BFS groups.
fn bench_snapshot(criterion: &mut Criterion) {
    {
        let mut open_group = criterion.benchmark_group("snapshot_csr_open");
        bench_open(&mut open_group);
        open_group.finish();
    }
    {
        let mut bfs_group = criterion.benchmark_group("snapshot_csr_bfs");
        bench_bfs(&mut bfs_group);
        bfs_group.finish();
    }
}

criterion_group!(benches, bench_snapshot);
criterion_main!(benches);
