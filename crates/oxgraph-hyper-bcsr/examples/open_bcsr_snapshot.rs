//! Walkthrough: build a bipartite-CSR snapshot, open it, and walk it.
//!
//! The snapshot crate is topology-agnostic; this example shows how the
//! hypergraph layout crate stacks on top of it by registering its eight
//! section kinds and reading them back without copying.
//!
//! Run with:
//! `cargo run -p oxgraph-hyper-bcsr --example open_bcsr_snapshot`

use oxgraph_hyper::{DirectedHyperedgeParticipants, DirectedVertexSuccessors};
use oxgraph_hyper_bcsr::{
    BcsrHyperedgeId, BcsrSnapshotError, BcsrSnapshotHypergraph, BcsrSnapshotIndex, BcsrVertexId,
};
use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotError};

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

/// Local error type covering both snapshot and bipartite-CSR adaptor failures.
#[derive(Debug)]
enum DemoError {
    /// Snapshot opening failed.
    Snapshot(SnapshotError),
    /// Bipartite-CSR adaptor failed.
    Adaptor(BcsrSnapshotError),
}

impl From<SnapshotError> for DemoError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<BcsrSnapshotError> for DemoError {
    fn from(error: BcsrSnapshotError) -> Self {
        Self::Adaptor(error)
    }
}

impl core::fmt::Display for DemoError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot: {error}"),
            Self::Adaptor(error) => write!(formatter, "adaptor: {error}"),
        }
    }
}

impl std::error::Error for DemoError {}

/// Encodes `[u32]` words as little-endian bytes.
fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn main() -> Result<(), DemoError> {
    // Same fixture as `bcsr_directed`: three vertices, two directed hyperedges.
    let head_offsets: [u32; 3] = [0, 1, 2];
    let head_participants: [u32; 2] = [0, 1];
    let tail_offsets: [u32; 3] = [0, 2, 3];
    let tail_participants: [u32; 3] = [1, 2, 2];
    let vertex_outgoing_offsets: [u32; 4] = [0, 1, 2, 2];
    let vertex_outgoing_hyperedges: [u32; 2] = [0, 1];
    let vertex_incoming_offsets: [u32; 4] = [0, 0, 1, 3];
    let vertex_incoming_hyperedges: [u32; 3] = [0, 0, 1];

    let mut builder = SnapshotBuilder::new(crc32c_append);
    let entries: [(u32, &[u32]); 8] = [
        (SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32, &head_offsets),
        (SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32, &head_participants),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32, &tail_offsets),
        (SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32, &tail_participants),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
            &vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
            &vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
            &vertex_incoming_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
            &vertex_incoming_hyperedges,
        ),
    ];
    for (kind, words) in entries {
        if let Err(error) = builder.add_section(kind, 0, 2, words_to_bytes(words)) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    println!("encoded snapshot: {} bytes", bytes.len());

    let snapshot = Snapshot::open(&bytes)?;
    let view = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot)?;
    println!(
        "hypergraph: {} vertices, {} hyperedges",
        view.vertex_count(),
        view.hyperedge_count()
    );

    let h0 = BcsrHyperedgeId::new(0);
    let heads: Vec<u32> = view
        .source_participants(h0)
        .map(BcsrVertexId::get)
        .collect();
    let tails: Vec<u32> = view
        .target_participants(h0)
        .map(BcsrVertexId::get)
        .collect();
    println!("h0 head={heads:?} tail={tails:?}");

    let successors: Vec<u32> = view
        .successor_vertices(BcsrVertexId::new(0))
        .map(BcsrVertexId::get)
        .collect();
    println!("successors of v0 (through h0's tail)={successors:?}");

    Ok(())
}
