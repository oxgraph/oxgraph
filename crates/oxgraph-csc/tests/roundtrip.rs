//! Integration tests: inbound CSC sections opened from a synthetic OXGTOPO
//! snapshot built with the CSR builder + transpose, without depending on any
//! storage engine. The inbound (reverse) adjacency is the transpose of the
//! forward graph, exported under the default CSR section kinds.

use oxgraph_csc::{CscNodeId, CscSnapshotGraph};
use oxgraph_csr::{
    SNAPSHOT_CSR_SECTION_VERSION,
    build::{GraphBuilder, GraphNodeId, export_csr_snapshot},
};
use oxgraph_snapshot::Snapshot;

/// Section kinds the synthetic inbound snapshot is written under. The transpose
/// is exported with the default CSR offsets/targets kinds; the CSC view reads
/// them via the storage-agnostic `from_snapshot_with_kinds`.
const OFFSETS_KIND: u32 = oxgraph_csr::SNAPSHOT_KIND_CSR_OFFSETS_U32;
const TARGETS_KIND: u32 = oxgraph_csr::SNAPSHOT_KIND_CSR_TARGETS_U32;

/// Builds an inbound (transposed) CSR snapshot for `edges` over `node_count`
/// nodes and returns the encoded bytes.
fn inbound_snapshot(
    node_count: u32,
    edges: &[(u32, u32)],
) -> Result<Vec<u8>, Box<dyn core::error::Error>> {
    let mut builder = GraphBuilder::<u32, u32>::new();
    for _ in 0..node_count {
        builder.add_node()?;
    }
    for &(source, target) in edges {
        builder.add_edge(GraphNodeId::new(source), GraphNodeId::new(target))?;
    }
    let forward = builder.freeze()?;
    let inbound = forward.transpose()?;
    Ok(export_csr_snapshot(&inbound)?)
}

#[test]
fn inbound_view_reads_transposed_predecessors() -> Result<(), Box<dyn core::error::Error>> {
    let bytes = inbound_snapshot(4, &[(0, 1), (1, 2)])?;
    let snapshot = Snapshot::open(&bytes)?;
    let csc = CscSnapshotGraph::<u32, u32>::from_snapshot_with_kinds(
        &snapshot,
        OFFSETS_KIND,
        TARGETS_KIND,
        SNAPSHOT_CSR_SECTION_VERSION,
    )?;
    let preds: Vec<u32> = csc
        .predecessors(CscNodeId::new(2))
        .map(CscNodeId::get)
        .collect();
    assert_eq!(preds, vec![1]);
    Ok(())
}

#[test]
fn for_each_predecessor_matches_iterator() -> Result<(), Box<dyn core::error::Error>> {
    let bytes = inbound_snapshot(4, &[(0, 1), (1, 2), (0, 2)])?;
    let snapshot = Snapshot::open(&bytes)?;
    let csc = CscSnapshotGraph::<u32, u32>::from_snapshot_with_kinds(
        &snapshot,
        OFFSETS_KIND,
        TARGETS_KIND,
        SNAPSHOT_CSR_SECTION_VERSION,
    )?;
    let mut collected = Vec::new();
    csc.for_each_predecessor(CscNodeId::new(2), |pred| {
        collected.push(pred.get());
        false
    });
    collected.sort_unstable();
    let mut expected: Vec<u32> = csc
        .predecessors(CscNodeId::new(2))
        .map(CscNodeId::get)
        .collect();
    expected.sort_unstable();
    assert_eq!(collected, expected);
    Ok(())
}

#[test]
fn for_each_predecessor_stops_early() -> Result<(), Box<dyn core::error::Error>> {
    let bytes = inbound_snapshot(4, &[(0, 2), (1, 2)])?;
    let snapshot = Snapshot::open(&bytes)?;
    let csc = CscSnapshotGraph::<u32, u32>::from_snapshot_with_kinds(
        &snapshot,
        OFFSETS_KIND,
        TARGETS_KIND,
        SNAPSHOT_CSR_SECTION_VERSION,
    )?;
    let mut seen = 0_u32;
    let stopped = csc.for_each_predecessor(CscNodeId::new(2), |pred| {
        seen = pred.get();
        true
    });
    assert!(stopped);
    assert!(seen == 0 || seen == 1);
    Ok(())
}
