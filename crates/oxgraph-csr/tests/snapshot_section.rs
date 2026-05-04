//! Tests for opening a `CsrGraph` from an `oxgraph-snapshot` container.

use oxgraph_algo::breadth_first_search;
use oxgraph_csr::{
    CsrError, CsrGraph, CsrNodeId, CsrSnapshotError, SNAPSHOT_KIND_CSR_OFFSETS,
    SNAPSHOT_KIND_CSR_TARGETS,
};
use oxgraph_graph::{EdgeTargetGraph, GraphCounts, OutgoingGraph};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotError};

/// Test fixture error covering snapshot, view, and CSR failure modes.
#[derive(Debug)]
enum FixtureError {
    /// Snapshot container validation failed.
    Snapshot(SnapshotError),
    /// CSR snapshot adaptor failed.
    Adaptor(CsrSnapshotError),
}

impl From<SnapshotError> for FixtureError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<CsrSnapshotError> for FixtureError {
    fn from(error: CsrSnapshotError) -> Self {
        Self::Adaptor(error)
    }
}

impl core::fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot validation failed: {error}"),
            Self::Adaptor(error) => write!(formatter, "CSR adaptor failed: {error}"),
        }
    }
}

impl std::error::Error for FixtureError {}

/// Encodes `[u32]` words as a little-endian byte vector.
fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Builds a snapshot containing CSR offsets + targets sections.
fn build_csr_snapshot(offsets: &[u32], targets: &[u32]) -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    if let Err(error) =
        builder.add_section(SNAPSHOT_KIND_CSR_OFFSETS, 0, 2, words_to_bytes(offsets))
    {
        panic!("offsets section: {error:?}");
    }
    if let Err(error) =
        builder.add_section(SNAPSHOT_KIND_CSR_TARGETS, 0, 2, words_to_bytes(targets))
    {
        panic!("targets section: {error:?}");
    }
    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("builder finish: {error:?}"),
    }
}

#[test]
fn opens_valid_snapshot_as_csr_graph() -> Result<(), FixtureError> {
    let bytes = build_csr_snapshot(&[0, 2, 3, 4, 4], &[1, 2, 2, 3]);
    let snapshot = Snapshot::open(&bytes)?;
    let graph = CsrGraph::from_snapshot(&snapshot)?;

    assert_eq!(graph.node_count(), 4);
    assert_eq!(graph.edge_count(), 4);
    assert_eq!(
        graph
            .outgoing_edges(CsrNodeId(0))
            .map(|edge| graph.target(edge))
            .collect::<Vec<_>>(),
        [CsrNodeId(1), CsrNodeId(2)]
    );

    Ok(())
}

#[test]
fn bfs_runs_over_snapshot_csr_graph() -> Result<(), FixtureError> {
    let bytes = build_csr_snapshot(&[0, 2, 3, 4, 4], &[1, 2, 2, 3]);
    let snapshot = Snapshot::open(&bytes)?;
    let graph = CsrGraph::from_snapshot(&snapshot)?;

    let order: Vec<CsrNodeId> = match breadth_first_search(&graph, CsrNodeId(0)) {
        Ok(walk) => walk.collect(),
        Err(error) => panic!("bfs failed: {error:?}"),
    };
    assert_eq!(
        order,
        [CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)]
    );

    Ok(())
}

#[test]
fn rejects_missing_offsets_section() -> Result<(), SnapshotError> {
    let mut builder = SnapshotBuilder::new();
    if let Err(error) =
        builder.add_section(SNAPSHOT_KIND_CSR_TARGETS, 0, 2, words_to_bytes(&[0, 1]))
    {
        panic!("targets-only: {error:?}");
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    let snapshot = Snapshot::open(&bytes)?;
    match CsrGraph::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::MissingOffsets) => Ok(()),
        other => panic!("expected MissingOffsets, got {other:?}"),
    }
}

#[test]
fn rejects_empty_offsets_section() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot(&[], &[]);
    let snapshot = Snapshot::open(&bytes)?;
    match CsrGraph::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::OffsetsEmpty) => Ok(()),
        other => panic!("expected OffsetsEmpty, got {other:?}"),
    }
}

#[test]
fn rejects_target_out_of_range() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot(&[0, 1], &[42]);
    let snapshot = Snapshot::open(&bytes)?;
    match CsrGraph::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::Csr(CsrError::TargetOutOfRange { target: 42, .. })) => Ok(()),
        other => panic!("expected Csr(TargetOutOfRange), got {other:?}"),
    }
}

#[test]
fn rejects_non_monotonic_offsets() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot(&[0, 3, 1, 1], &[0, 1, 2]);
    let snapshot = Snapshot::open(&bytes)?;
    match CsrGraph::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::Csr(CsrError::NonMonotonicOffset { .. })) => Ok(()),
        other => panic!("expected Csr(NonMonotonicOffset), got {other:?}"),
    }
}
