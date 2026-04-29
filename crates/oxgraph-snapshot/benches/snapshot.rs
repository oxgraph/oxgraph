//! Benchmarks for validating snapshots and traversing CSR sections.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxgraph_algo::{breadth_first_search, breadth_first_search_generic};
use oxgraph_csr::{CsrGraph, CsrNodeId};
use oxgraph_graph::{EdgeTargetGraph, OutgoingGraph};
use oxgraph_snapshot::GraphSnapshot;
use zerocopy::byteorder::{LE, U32};

/// Snapshot magic bytes for benchmark fixture construction.
const MAGIC: &[u8; 8] = b"OCTXG0\0\0";

/// CSR offsets section kind.
const SECTION_CSR_OFFSETS: u32 = 1;

/// CSR targets section kind.
const SECTION_CSR_TARGETS: u32 = 2;

/// Fixed out-degree used by the synthetic regular graph.
const DEGREE: u32 = 4;

/// Graph sizes used to observe snapshot scaling behavior.
const NODE_COUNTS: &[u32] = &[10_000, 100_000, 1_000_000];

/// Section counts used to observe snapshot section-table validation scaling.
const SECTION_COUNTS: &[u32] = &[2, 8, 32, 128, 512];

/// Appends a little-endian `u32` to `bytes`.
fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Builds a deterministic regular CSR graph.
fn build_regular_csr(node_count: u32, degree: u32) -> (Vec<u32>, Vec<u32>) {
    let edge_count = node_count.saturating_mul(degree);
    let mut offsets = Vec::with_capacity(usize_from_u32(node_count) + 1);
    let mut targets = Vec::with_capacity(usize_from_u32(edge_count));

    offsets.push(0);
    for node in 0..node_count {
        let next_offset = offsets[offsets.len() - 1] + degree;
        offsets.push(next_offset);

        for step in 0..degree {
            targets.push((node + step + 1) % node_count);
        }
    }

    (offsets, targets)
}

/// Builds a v0 CSR snapshot byte vector.
fn build_snapshot(node_count: u32, offsets: &[u32], targets: &[u32]) -> Vec<u8> {
    let header_len = 24u32;
    let section_table_len = 24u32;
    let offsets_offset = header_len + section_table_len;
    let offsets_len = usize_to_u32_lossless(offsets.len() * 4);
    let targets_offset = offsets_offset + offsets_len;
    let targets_len = usize_to_u32_lossless(targets.len() * 4);

    let mut bytes = Vec::with_capacity(usize_from_u32(
        header_len + section_table_len + offsets_len + targets_len,
    ));
    bytes.extend_from_slice(MAGIC);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, node_count);
    push_u32(&mut bytes, 2);

    push_u32(&mut bytes, SECTION_CSR_OFFSETS);
    push_u32(&mut bytes, offsets_offset);
    push_u32(&mut bytes, offsets_len);
    push_u32(&mut bytes, SECTION_CSR_TARGETS);
    push_u32(&mut bytes, targets_offset);
    push_u32(&mut bytes, targets_len);

    for offset in offsets {
        push_u32(&mut bytes, *offset);
    }
    for target in targets {
        push_u32(&mut bytes, *target);
    }

    bytes
}

/// Builds a valid snapshot with many small sections.
fn build_section_table_snapshot(section_count: u32) -> Vec<u8> {
    let header_len = 24u32;
    let section_table_len = section_count * 12;
    let section_len = 4u32;
    let payload_offset = header_len + section_table_len;
    let snapshot_len = payload_offset + section_count * section_len;

    let mut bytes = Vec::with_capacity(usize_from_u32(snapshot_len));
    bytes.extend_from_slice(MAGIC);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, section_count);

    for index in 0..section_count {
        push_u32(&mut bytes, 10_000 + index);
        push_u32(&mut bytes, payload_offset + index * section_len);
        push_u32(&mut bytes, section_len);
    }

    for index in 0..section_count {
        push_u32(&mut bytes, index);
    }

    bytes
}

/// Opens the benchmark CSR graph from snapshot sections.
fn csr_graph<'view>(snapshot: &GraphSnapshot<'view>) -> CsrGraph<'view, U32<LE>> {
    let offsets = match snapshot.section_words(SECTION_CSR_OFFSETS) {
        Ok(Some(words)) => words,
        Ok(None) => panic!("benchmark snapshot missing CSR offsets"),
        Err(error) => panic!("benchmark CSR offsets were malformed: {error:?}"),
    };
    let targets = match snapshot.section_words(SECTION_CSR_TARGETS) {
        Ok(Some(words)) => words,
        Ok(None) => panic!("benchmark snapshot missing CSR targets"),
        Err(error) => panic!("benchmark CSR targets were malformed: {error:?}"),
    };

    match CsrGraph::validate(snapshot.node_count(), offsets, targets) {
        Ok(graph) => graph,
        Err(error) => panic!("benchmark CSR fixture was invalid: {error:?}"),
    }
}

/// Traverses every outgoing target and returns a checksum to defeat dead-code elimination.
fn traverse_graph(graph: &CsrGraph<'_, U32<LE>>, node_count: u32) -> u64 {
    let mut checksum = 0u64;
    for node in 0..node_count {
        for edge in graph.outgoing_edges(CsrNodeId(node)) {
            checksum ^= u64::from(graph.target(edge).0);
        }
    }
    checksum
}

/// Runs BFS over a CSR graph opened from snapshot sections.
fn generic_bfs_graph(graph: &CsrGraph<'_, U32<LE>>) -> usize {
    breadth_first_search_generic(graph, CsrNodeId(0)).count()
}

/// Runs default indexed BFS and returns the number of reached nodes.
fn default_bfs_graph(graph: &CsrGraph<'_, U32<LE>>) -> usize {
    match breadth_first_search(graph, CsrNodeId(0)) {
        Ok(traversal) => traversal.count(),
        Err(error) => panic!("benchmark BFS start was invalid: {error:?}"),
    }
}

/// Converts a `u32` fixture size into `usize`.
fn usize_from_u32(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark size did not fit usize: {error:?}"),
    }
}

/// Converts fixture byte lengths into `u32`.
fn usize_to_u32_lossless(value: usize) -> u32 {
    match u32::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark byte length did not fit u32: {error:?}"),
    }
}

/// Benchmarks snapshot section-table validation.
///
/// Defends `GraphSnapshot::validate` `O(s^2)` duplicate and overlap checks for
/// `s` section entries.
fn bench_validate(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_validate");
    for section_count in SECTION_COUNTS {
        let bytes = build_section_table_snapshot(*section_count);

        group.throughput(Throughput::Elements(u64::from(*section_count)));
        group.bench_with_input(
            BenchmarkId::from_parameter(section_count),
            section_count,
            |b, _section_count| {
                b.iter(|| black_box(GraphSnapshot::validate(black_box(&bytes))));
            },
        );
    }
    group.finish();
}

/// Benchmarks opening a CSR graph view from an already-validated snapshot.
///
/// Defends snapshot section lookup plus `CsrGraph::validate` over borrowed CSR
/// offset and target sections.
fn bench_open_csr(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_open_csr");
    for node_count in NODE_COUNTS {
        let (offsets, targets) = build_regular_csr(*node_count, DEGREE);
        let bytes = build_snapshot(*node_count, &offsets, &targets);
        let snapshot = match GraphSnapshot::validate(&bytes) {
            Ok(validated) => validated,
            Err(error) => panic!("benchmark snapshot fixture was invalid: {error:?}"),
        };

        group.throughput(Throughput::Elements(u64_from_usize(
            offsets.len() + targets.len(),
        )));
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, _size| {
                b.iter(|| black_box(csr_graph(black_box(&snapshot))));
            },
        );
    }
    group.finish();
}

/// Benchmarks traversal over an already-opened snapshot-backed CSR graph.
///
/// Defends CSR outgoing traversal over borrowed little-endian snapshot words,
/// excluding snapshot section lookup and CSR validation.
fn bench_traverse(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_csr_traverse");
    for node_count in NODE_COUNTS {
        let (offsets, targets) = build_regular_csr(*node_count, DEGREE);
        let bytes = build_snapshot(*node_count, &offsets, &targets);
        let snapshot = match GraphSnapshot::validate(&bytes) {
            Ok(validated) => validated,
            Err(error) => panic!("benchmark snapshot fixture was invalid: {error:?}"),
        };
        let graph = csr_graph(&snapshot);

        group.throughput(Throughput::Elements(
            u64::from(*node_count) * u64::from(DEGREE),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, size| {
                b.iter(|| black_box(traverse_graph(&graph, *size)));
            },
        );
    }
    group.finish();
}

/// Benchmarks directed BFS over an already-opened snapshot-backed CSR graph.
///
/// Defends indexed and generic BFS traversal costs without snapshot section
/// lookup or CSR validation in the measured loop.
fn bench_bfs(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_bfs");
    for node_count in NODE_COUNTS {
        let (offsets, targets) = build_regular_csr(*node_count, DEGREE);
        let bytes = build_snapshot(*node_count, &offsets, &targets);
        let snapshot = match GraphSnapshot::validate(&bytes) {
            Ok(validated) => validated,
            Err(error) => panic!("benchmark snapshot fixture was invalid: {error:?}"),
        };
        let graph = csr_graph(&snapshot);

        group.throughput(Throughput::Elements(u64::from(*node_count)));
        group.bench_with_input(
            BenchmarkId::new("generic", node_count),
            node_count,
            |b, _size| {
                b.iter(|| black_box(generic_bfs_graph(&graph)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("default_indexed", node_count),
            node_count,
            |b, _size| {
                b.iter(|| black_box(default_bfs_graph(&graph)));
            },
        );
    }
    group.finish();
}

/// Converts `usize` byte lengths into `u64` for Criterion throughput reporting.
fn u64_from_usize(value: usize) -> u64 {
    match u64::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark byte length did not fit u64: {error:?}"),
    }
}

criterion_group!(
    benches,
    bench_validate,
    bench_open_csr,
    bench_traverse,
    bench_bfs
);
criterion_main!(benches);
