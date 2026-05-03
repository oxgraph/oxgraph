//! Benchmarks for bipartite-CSR validation and traversal.
//!
//! Each benchmark builds a deterministic regular bipartite hypergraph in
//! which every hyperedge has the same head and tail size. Sizes are scale
//! smoke tests, not final performance contracts.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxgraph_hyper::{DirectedHyperedgeParticipants, DirectedVertexSuccessors, IncidentHyperedges};
use oxgraph_hyper_bcsr::{
    BcsrHyperedgeId, BcsrHypergraph, BcsrSections, BcsrValidation, BcsrVertexId,
};

/// Fixed head and tail size used by the synthetic regular hypergraph.
const FAN: u32 = 4;

/// Vertex counts used to observe scaling behavior. Vertex count and
/// hyperedge count are kept equal for symmetry.
const VERTEX_COUNTS: &[u32] = &[1_000, 16_000, 256_000];

/// Slice payloads for one regular bipartite hypergraph.
struct RegularSlices {
    /// Hyperedge-major head offsets.
    head_offsets: Vec<u32>,
    /// Flat vertex IDs in head sets.
    head_participants: Vec<u32>,
    /// Hyperedge-major tail offsets.
    tail_offsets: Vec<u32>,
    /// Flat vertex IDs in tail sets.
    tail_participants: Vec<u32>,
    /// Vertex-major outgoing offsets.
    vertex_outgoing_offsets: Vec<u32>,
    /// Flat hyperedge IDs where the vertex is in head.
    vertex_outgoing_hyperedges: Vec<u32>,
    /// Vertex-major incoming offsets.
    vertex_incoming_offsets: Vec<u32>,
    /// Flat hyperedge IDs where the vertex is in tail.
    vertex_incoming_hyperedges: Vec<u32>,
}

impl RegularSlices {
    /// Returns a [`BcsrSections`] borrowing this fixture.
    fn sections(&self) -> BcsrSections<'_, u32> {
        BcsrSections {
            head_offsets: &self.head_offsets,
            head_participants: &self.head_participants,
            tail_offsets: &self.tail_offsets,
            tail_participants: &self.tail_participants,
            vertex_outgoing_offsets: &self.vertex_outgoing_offsets,
            vertex_outgoing_hyperedges: &self.vertex_outgoing_hyperedges,
            vertex_incoming_offsets: &self.vertex_incoming_offsets,
            vertex_incoming_hyperedges: &self.vertex_incoming_hyperedges,
        }
    }
}

/// Converts a `u32` fixture size into `usize`.
fn usize_from_u32(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark size did not fit usize: {error:?}"),
    }
}

/// Builds a deterministic regular bipartite hypergraph with `vertex_count`
/// vertices and `vertex_count` hyperedges. Each hyperedge `h` has head
/// `{(h + i) mod V : i ∈ 0..fan}` and tail `{(h + fan + j) mod V : j ∈ 0..fan}`,
/// each kept sorted-and-unique inside the bucket.
fn build_regular_hypergraph(vertex_count: u32) -> RegularSlices {
    let pairs = vertex_count.saturating_mul(FAN);
    let count_usize = usize_from_u32(vertex_count);
    let pairs_usize = usize_from_u32(pairs);

    let mut slices = RegularSlices {
        head_offsets: Vec::with_capacity(count_usize + 1),
        head_participants: Vec::with_capacity(pairs_usize),
        tail_offsets: Vec::with_capacity(count_usize + 1),
        tail_participants: Vec::with_capacity(pairs_usize),
        vertex_outgoing_offsets: Vec::with_capacity(count_usize + 1),
        vertex_outgoing_hyperedges: Vec::with_capacity(pairs_usize),
        vertex_incoming_offsets: Vec::with_capacity(count_usize + 1),
        vertex_incoming_hyperedges: Vec::with_capacity(pairs_usize),
    };

    push_hyperedge_major(vertex_count, &mut slices);
    push_vertex_major(vertex_count, &mut slices);
    slices
}

/// Fills `slices` with the hyperedge-major head and tail sections.
fn push_hyperedge_major(vertex_count: u32, slices: &mut RegularSlices) {
    slices.head_offsets.push(0);
    slices.tail_offsets.push(0);
    let mut head_total = 0_u32;
    let mut tail_total = 0_u32;
    for h in 0..vertex_count {
        for i in 0..FAN {
            slices.head_participants.push((h + i) % vertex_count);
        }
        head_total += FAN;
        slices.head_offsets.push(head_total);

        for j in 0..FAN {
            slices.tail_participants.push((h + FAN + j) % vertex_count);
        }
        tail_total += FAN;
        slices.tail_offsets.push(tail_total);
    }
}

/// Fills `slices` with the vertex-major outgoing and incoming sections.
fn push_vertex_major(vertex_count: u32, slices: &mut RegularSlices) {
    slices.vertex_outgoing_offsets.push(0);
    slices.vertex_incoming_offsets.push(0);
    let mut out_total = 0_u32;
    let mut in_total = 0_u32;
    for v in 0..vertex_count {
        let mut bucket: Vec<u32> = (0..FAN)
            .map(|i| (v + vertex_count - i) % vertex_count)
            .collect();
        bucket.sort_unstable();
        slices.vertex_outgoing_hyperedges.extend_from_slice(&bucket);
        out_total += FAN;
        slices.vertex_outgoing_offsets.push(out_total);

        let mut bucket: Vec<u32> = (0..FAN)
            .map(|j| (v + vertex_count - FAN - j) % vertex_count)
            .collect();
        bucket.sort_unstable();
        slices.vertex_incoming_hyperedges.extend_from_slice(&bucket);
        in_total += FAN;
        slices.vertex_incoming_offsets.push(in_total);
    }
}

/// Walks every hyperedge's head and tail and returns a checksum.
fn walk_hyperedges(view: &BcsrHypergraph<'_>) -> u64 {
    let mut checksum = 0u64;
    let h_count = match u32::try_from(view.hyperedge_count()) {
        Ok(value) => value,
        Err(error) => panic!("benchmark hyperedge count overflow: {error:?}"),
    };
    for h in 0..h_count {
        for vertex in view.source_participants(BcsrHyperedgeId(h)) {
            checksum ^= u64::from(vertex.0);
        }
        for vertex in view.target_participants(BcsrHyperedgeId(h)) {
            checksum ^= u64::from(vertex.0).rotate_left(1);
        }
    }
    checksum
}

/// Walks every vertex's incident hyperedges and returns a checksum.
fn walk_incident(view: &BcsrHypergraph<'_>) -> u64 {
    let mut checksum = 0u64;
    let v_count = match u32::try_from(view.vertex_count()) {
        Ok(value) => value,
        Err(error) => panic!("benchmark vertex count overflow: {error:?}"),
    };
    for v in 0..v_count {
        for hyperedge in view.incident_hyperedges(BcsrVertexId(v)) {
            checksum ^= u64::from(hyperedge.0);
        }
    }
    checksum
}

/// Walks every vertex's directed successors and returns a checksum.
fn walk_successors(view: &BcsrHypergraph<'_>) -> u64 {
    let mut checksum = 0u64;
    let v_count = match u32::try_from(view.vertex_count()) {
        Ok(value) => value,
        Err(error) => panic!("benchmark vertex count overflow: {error:?}"),
    };
    for v in 0..v_count {
        for vertex in view.successor_vertices(BcsrVertexId(v)) {
            checksum ^= u64::from(vertex.0);
        }
    }
    checksum
}

/// Throughput unit per benchmark sample (number of incidence pairs visited).
fn pairs_throughput(vertex_count: u32) -> u64 {
    u64::from(vertex_count) * u64::from(FAN) * 2
}

/// Opens a borrowed view over `slices` or panics with a clear message.
fn open_view(slices: &RegularSlices) -> BcsrHypergraph<'_> {
    match BcsrHypergraph::open(slices.sections()) {
        Ok(value) => value,
        Err(error) => panic!("regular fixture invalid: {error:?}"),
    }
}

/// Benchmarks bipartite-CSR validation at `Layout`.
fn bench_open_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcsr_open_layout");
    for vertex_count in VERTEX_COUNTS {
        let slices = build_regular_hypergraph(*vertex_count);
        group.throughput(Throughput::Elements(pairs_throughput(*vertex_count)));
        group.bench_with_input(
            BenchmarkId::from_parameter(vertex_count),
            vertex_count,
            |b, _| {
                b.iter(|| {
                    let view = BcsrHypergraph::open(black_box(slices.sections()));
                    black_box(view)
                });
            },
        );
    }
    group.finish();
}

/// Benchmarks bipartite-CSR validation at `Strict`.
fn bench_open_strict(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcsr_open_strict");
    for vertex_count in VERTEX_COUNTS {
        let slices = build_regular_hypergraph(*vertex_count);
        group.throughput(Throughput::Elements(pairs_throughput(*vertex_count)));
        group.bench_with_input(
            BenchmarkId::from_parameter(vertex_count),
            vertex_count,
            |b, _| {
                b.iter(|| {
                    let view = BcsrHypergraph::open_with(
                        black_box(slices.sections()),
                        BcsrValidation::Strict,
                    );
                    black_box(view)
                });
            },
        );
    }
    group.finish();
}

/// Benchmarks hyperedge-major head/tail iteration.
fn bench_walk_hyperedges(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcsr_walk_hyperedges");
    for vertex_count in VERTEX_COUNTS {
        let slices = build_regular_hypergraph(*vertex_count);
        let view = open_view(&slices);
        group.throughput(Throughput::Elements(pairs_throughput(*vertex_count)));
        group.bench_with_input(
            BenchmarkId::from_parameter(vertex_count),
            vertex_count,
            |b, _| {
                b.iter(|| black_box(walk_hyperedges(&view)));
            },
        );
    }
    group.finish();
}

/// Benchmarks vertex-major incident-hyperedge iteration.
fn bench_walk_incident(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcsr_walk_incident");
    for vertex_count in VERTEX_COUNTS {
        let slices = build_regular_hypergraph(*vertex_count);
        let view = open_view(&slices);
        group.throughput(Throughput::Elements(pairs_throughput(*vertex_count)));
        group.bench_with_input(
            BenchmarkId::from_parameter(vertex_count),
            vertex_count,
            |b, _| {
                b.iter(|| black_box(walk_incident(&view)));
            },
        );
    }
    group.finish();
}

/// Benchmarks directed vertex-to-vertex successor expansion.
fn bench_walk_successors(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcsr_walk_successors");
    for vertex_count in VERTEX_COUNTS {
        let slices = build_regular_hypergraph(*vertex_count);
        let view = open_view(&slices);
        group.throughput(Throughput::Elements(
            u64::from(*vertex_count) * u64::from(FAN) * u64::from(FAN),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(vertex_count),
            vertex_count,
            |b, _| {
                b.iter(|| black_box(walk_successors(&view)));
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_open_layout,
    bench_open_strict,
    bench_walk_hyperedges,
    bench_walk_incident,
    bench_walk_successors
);
criterion_main!(benches);
