//! Snapshot-backed benchmarks for [`BcsrHypergraph`].
//!
//! Builds an in-memory snapshot from the same regular bipartite-CSR fixture
//! used by `benches/bcsr.rs`, then measures `Snapshot::open` +
//! `BcsrHypergraph::from_snapshot` + traversal as one combined cost.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxgraph_hyper::DirectedVertexSuccessors;
use oxgraph_hyper_bcsr::{
    BcsrHypergraph, BcsrVertexId, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS, SNAPSHOT_KIND_BCSR_TAIL_OFFSETS,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder};

/// Fixed head and tail size used by the synthetic regular hypergraph.
const FAN: u32 = 4;

/// Vertex counts used for the snapshot-backed benchmarks.
const VERTEX_COUNTS: &[u32] = &[1_000, 16_000];

/// Convenience: cast `u32` to `usize` with a clear panic message.
fn usize_from_u32(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("benchmark size did not fit usize: {error:?}"),
    }
}

/// Encodes `[u32]` words as little-endian bytes.
fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Section payloads for one regular bipartite hypergraph (parallel to
/// `benches/bcsr.rs` but kept separate so that bench harnesses do not
/// share a build helper).
struct SectionWords {
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

/// Builds the eight section payloads for a regular bipartite hypergraph.
fn build_section_words(vertex_count: u32) -> SectionWords {
    let count_usize = usize_from_u32(vertex_count);
    let pairs_usize = usize_from_u32(vertex_count * FAN);
    let mut words = SectionWords {
        head_offsets: Vec::with_capacity(count_usize + 1),
        head_participants: Vec::with_capacity(pairs_usize),
        tail_offsets: Vec::with_capacity(count_usize + 1),
        tail_participants: Vec::with_capacity(pairs_usize),
        vertex_outgoing_offsets: Vec::with_capacity(count_usize + 1),
        vertex_outgoing_hyperedges: Vec::with_capacity(pairs_usize),
        vertex_incoming_offsets: Vec::with_capacity(count_usize + 1),
        vertex_incoming_hyperedges: Vec::with_capacity(pairs_usize),
    };
    words.head_offsets.push(0);
    words.tail_offsets.push(0);
    words.vertex_outgoing_offsets.push(0);
    words.vertex_incoming_offsets.push(0);
    let mut head_total = 0_u32;
    let mut tail_total = 0_u32;
    for h in 0..vertex_count {
        for i in 0..FAN {
            words.head_participants.push((h + i) % vertex_count);
        }
        head_total += FAN;
        words.head_offsets.push(head_total);
        for j in 0..FAN {
            words.tail_participants.push((h + FAN + j) % vertex_count);
        }
        tail_total += FAN;
        words.tail_offsets.push(tail_total);
    }
    fill_vertex_major(vertex_count, &mut words);
    words
}

/// Fills the vertex-major sections in-place with the regular pattern.
fn fill_vertex_major(vertex_count: u32, words: &mut SectionWords) {
    let mut out_total = 0_u32;
    let mut in_total = 0_u32;
    for v in 0..vertex_count {
        let mut bucket: Vec<u32> = (0..FAN)
            .map(|i| (v + vertex_count - i) % vertex_count)
            .collect();
        bucket.sort_unstable();
        words.vertex_outgoing_hyperedges.extend_from_slice(&bucket);
        out_total += FAN;
        words.vertex_outgoing_offsets.push(out_total);

        let mut bucket: Vec<u32> = (0..FAN)
            .map(|j| (v + vertex_count - FAN - j) % vertex_count)
            .collect();
        bucket.sort_unstable();
        words.vertex_incoming_hyperedges.extend_from_slice(&bucket);
        in_total += FAN;
        words.vertex_incoming_offsets.push(in_total);
    }
}

/// Encodes a [`SectionWords`] into a snapshot byte buffer.
fn encode_snapshot(words: &SectionWords) -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    let entries: [(u32, &[u32]); 8] = [
        (SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, &words.head_offsets),
        (
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
            &words.head_participants,
        ),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, &words.tail_offsets),
        (
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
            &words.tail_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
            &words.vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
            &words.vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
            &words.vertex_incoming_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
            &words.vertex_incoming_hyperedges,
        ),
    ];
    for (kind, payload) in entries {
        if let Err(error) = builder.add_section(kind, 0, 2, words_to_bytes(payload)) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    builder.finish()
}

/// Walks every vertex's successor expansion and returns a checksum.
fn walk_successors(
    view: &BcsrHypergraph<'_, zerocopy::byteorder::U32<zerocopy::byteorder::LE>>,
) -> u64 {
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

/// Benchmarks `Snapshot::open` followed by `BcsrHypergraph::from_snapshot`.
fn bench_from_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcsr_from_snapshot");
    for vertex_count in VERTEX_COUNTS {
        let words = build_section_words(*vertex_count);
        let bytes = encode_snapshot(&words);
        group.throughput(Throughput::Elements(
            u64::from(*vertex_count) * u64::from(FAN) * 2,
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(vertex_count),
            vertex_count,
            |b, _| {
                b.iter(|| {
                    let snapshot = match Snapshot::open(black_box(&bytes)) {
                        Ok(value) => value,
                        Err(error) => panic!("benchmark snapshot invalid: {error:?}"),
                    };
                    black_box(open_snapshot_view(&snapshot))
                });
            },
        );
    }
    group.finish();
}

/// Opens `snapshot` as a [`BcsrHypergraph`] or panics with a clear message.
fn open_snapshot_view<'view>(
    snapshot: &Snapshot<'view>,
) -> BcsrHypergraph<'view, zerocopy::byteorder::U32<zerocopy::byteorder::LE>> {
    match BcsrHypergraph::from_snapshot(snapshot) {
        Ok(value) => value,
        Err(error) => panic!("benchmark bcsr invalid: {error:?}"),
    }
}

/// Benchmarks open + traversal of a snapshot-backed bipartite-CSR view.
fn bench_snapshot_traverse(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcsr_snapshot_traverse");
    for vertex_count in VERTEX_COUNTS {
        let words = build_section_words(*vertex_count);
        let bytes = encode_snapshot(&words);
        group.throughput(Throughput::Elements(
            u64::from(*vertex_count) * u64::from(FAN) * u64::from(FAN),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(vertex_count),
            vertex_count,
            |b, _| {
                b.iter(|| {
                    let snapshot = match Snapshot::open(black_box(&bytes)) {
                        Ok(value) => value,
                        Err(error) => panic!("benchmark snapshot invalid: {error:?}"),
                    };
                    let view = open_snapshot_view(&snapshot);
                    black_box(walk_successors(&view))
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_from_snapshot, bench_snapshot_traverse);
criterion_main!(benches);
