//! Integration tests: inbound CSC sections opened from an OXGTOPO artifact.

use oxgraph_csc::CscSnapshotGraph;
use oxgraph_postgres::DualTopologySnapshot;
use oxgraph_snapshot::Snapshot;

#[test]
fn inbound_view_reads_transposed_predecessors() -> Result<(), Box<dyn core::error::Error>> {
    let bytes =
        DualTopologySnapshot::from_dense_u32_edges_with_node_count(4, &[(0, 1), (1, 2)], 0)?;

    let snapshot = Snapshot::open(&bytes)?;
    let csc = CscSnapshotGraph::from_snapshot(&snapshot)?;
    let preds: Vec<u32> = csc.predecessors(2).collect();
    assert_eq!(preds, vec![1]);
    Ok(())
}

#[test]
fn for_each_predecessor_matches_iterator() -> Result<(), Box<dyn core::error::Error>> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges_with_node_count(
        4,
        &[(0, 1), (1, 2), (0, 2)],
        0,
    )?;
    let snapshot = Snapshot::open(&bytes)?;
    let csc = CscSnapshotGraph::from_snapshot(&snapshot)?;
    let mut collected = Vec::new();
    csc.for_each_predecessor(2, |pred| {
        collected.push(pred);
        false
    });
    collected.sort_unstable();
    let mut expected: Vec<u32> = csc.predecessors(2).collect();
    expected.sort_unstable();
    assert_eq!(collected, expected);
    Ok(())
}

#[test]
fn for_each_predecessor_stops_early() -> Result<(), Box<dyn core::error::Error>> {
    let bytes =
        DualTopologySnapshot::from_dense_u32_edges_with_node_count(4, &[(0, 2), (1, 2)], 0)?;
    let snapshot = Snapshot::open(&bytes)?;
    let csc = CscSnapshotGraph::from_snapshot(&snapshot)?;
    let mut seen = 0_u32;
    let stopped = csc.for_each_predecessor(2, |pred| {
        seen = pred;
        true
    });
    assert!(stopped);
    assert!(seen == 0 || seen == 1);
    Ok(())
}

#[test]
fn from_snapshot_with_kinds_matches_default() -> Result<(), Box<dyn core::error::Error>> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges(&[(0, 1)], 0)?;
    let snapshot = Snapshot::open(&bytes)?;
    let default = CscSnapshotGraph::from_snapshot(&snapshot)?;
    let explicit = CscSnapshotGraph::from_snapshot_with_kinds(
        &snapshot,
        oxgraph_csc::SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
        oxgraph_csc::SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
    )?;
    assert_eq!(default.node_count(), explicit.node_count());
    assert_eq!(default.relation_count(), explicit.relation_count());
    Ok(())
}
