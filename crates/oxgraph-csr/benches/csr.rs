//! Benchmarks for CSR validation and outgoing traversal.
//!
//! Defends two perf contracts (the corresponding doc claims live next to each item):
//! - [`CsrGraph::validate`](oxgraph_csr::CsrGraph::validate) is `O(n + m)` for `n` nodes and `m`
//!   edges — benched at `n ∈ {10k, 100k, 1M}` with fixed degree (so `m = 8 * n`) using
//!   `Throughput::Elements(n + m)` so a contract regression shows up as a sub-linear elements/sec
//!   curve rather than a hidden constant-factor blowup.
//! - Outgoing traversal via `OutgoingGraph::outgoing_edges` + `EdgeTargetGraph::target` is `O(k)`
//!   per source for `k` outgoing edges — benched on the same regular fixture; total work per bench
//!   iteration is `O(n * k) = O(m)`.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxgraph_csr::{CsrGraph, CsrNodeId};
use oxgraph_graph::{EdgeTargetGraph, OutgoingGraph};

/// Fixed out-degree used by the synthetic regular graph.
const DEGREE: u32 = 8;

/// Graph sizes used to observe scaling behavior.
const NODE_COUNTS: &[u32] = &[10_000, 100_000, 1_000_000];

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

/// Traverses every outgoing target and returns a checksum to defeat dead-code elimination.
fn traverse_targets(graph: &CsrGraph<'_>, node_count: u32) -> u64 {
    let mut checksum = 0u64;
    for node in 0..node_count {
        for edge in graph.outgoing_edges(CsrNodeId(node)) {
            checksum ^= u64::from(graph.target(edge).0);
        }
    }
    checksum
}

/// Converts a `u32` fixture size into `usize`.
fn usize_from_u32(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark size did not fit usize: {error:?}"),
    }
}

/// Benchmarks CSR validation over already-built slices.
fn bench_validate(c: &mut Criterion) {
    let mut group = c.benchmark_group("csr_validate");
    for node_count in NODE_COUNTS {
        let (offsets, targets) = build_regular_csr(*node_count, DEGREE);
        // Throughput unit is `n + m` (nodes + edges) so the elements/sec
        // curve tracks the stated `O(n + m)` validation contract directly.
        group.throughput(Throughput::Elements(
            u64::from(*node_count) + u64::from(*node_count) * u64::from(DEGREE),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, size| {
                b.iter(|| {
                    let graph = CsrGraph::validate(*size, black_box(&offsets), black_box(&targets));
                    black_box(graph)
                });
            },
        );
    }
    group.finish();
}

/// Benchmarks full outgoing traversal over a validated CSR graph.
fn bench_traverse(c: &mut Criterion) {
    let mut group = c.benchmark_group("csr_outgoing_traverse");
    for node_count in NODE_COUNTS {
        let (offsets, targets) = build_regular_csr(*node_count, DEGREE);
        let graph = match CsrGraph::validate(*node_count, &offsets, &targets) {
            Ok(validated) => validated,
            Err(error) => panic!("benchmark CSR fixture was invalid: {error:?}"),
        };

        group.throughput(Throughput::Elements(
            u64::from(*node_count) * u64::from(DEGREE),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(node_count),
            node_count,
            |b, size| {
                b.iter(|| black_box(traverse_targets(&graph, *size)));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_validate, bench_traverse);
criterion_main!(benches);
