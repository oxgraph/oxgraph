//! Tests for opening a `CsrGraph` from an `oxgraph-snapshot` container.

use oxgraph_algo::breadth_first_search;
use oxgraph_csr::{
    CsrEdgeId, CsrError, CsrNodeId, CsrSnapshotError, CsrSnapshotGraph,
    SNAPSHOT_KIND_CSR_OFFSETS_U16, SNAPSHOT_KIND_CSR_OFFSETS_U32, SNAPSHOT_KIND_CSR_OFFSETS_U64,
    SNAPSHOT_KIND_CSR_TARGETS_U16, SNAPSHOT_KIND_CSR_TARGETS_U32, SNAPSHOT_KIND_CSR_TARGETS_U64,
};
use oxgraph_graph::{EdgeTargetGraph, GraphCounts, OutgoingGraph};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotError};

/// Test fixture error covering snapshot, view, and CSR failure modes.
#[derive(Debug)]
enum FixtureError {
    /// Snapshot container validation failed.
    Snapshot(SnapshotError),
    /// CSR snapshot adaptor failed.
    Adaptor(CsrSnapshotError<u32, u32>),
}

impl From<SnapshotError> for FixtureError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<CsrSnapshotError<u32, u32>> for FixtureError {
    fn from(error: CsrSnapshotError<u32, u32>) -> Self {
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

/// Encodes `[u16]` words as a little-endian byte vector.
fn u16_words_to_bytes(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Encodes `[u64]` words as a little-endian byte vector.
fn u64_words_to_bytes(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Builds a snapshot containing encoded CSR offsets + targets sections.
fn build_csr_snapshot_from_bytes(
    offsets_kind: u32,
    targets_kind: u32,
    offsets_bytes: Vec<u8>,
    targets_bytes: Vec<u8>,
) -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    if let Err(error) = builder.add_section(offsets_kind, 0, 0, offsets_bytes) {
        panic!("offsets section: {error:?}");
    }
    if let Err(error) = builder.add_section(targets_kind, 0, 0, targets_bytes) {
        panic!("targets section: {error:?}");
    }
    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("builder finish: {error:?}"),
    }
}

/// Builds a snapshot containing u32 CSR offsets + targets sections.
fn build_csr_snapshot(offsets: &[u32], targets: &[u32]) -> Vec<u8> {
    build_csr_snapshot_from_bytes(
        SNAPSHOT_KIND_CSR_OFFSETS_U32,
        SNAPSHOT_KIND_CSR_TARGETS_U32,
        words_to_bytes(offsets),
        words_to_bytes(targets),
    )
}

#[test]
fn opens_valid_snapshot_as_csr_graph() -> Result<(), FixtureError> {
    let bytes = build_csr_snapshot(&[0, 2, 3, 4, 4], &[1, 2, 2, 3]);
    let snapshot = Snapshot::open(&bytes)?;
    let graph = CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot)?;

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
fn opens_u16_snapshot_as_csr_graph() -> Result<(), CsrSnapshotError<u16, u16>> {
    let bytes = build_csr_snapshot_from_bytes(
        SNAPSHOT_KIND_CSR_OFFSETS_U16,
        SNAPSHOT_KIND_CSR_TARGETS_U16,
        u16_words_to_bytes(&[0, 2, 2]),
        u16_words_to_bytes(&[1, 0]),
    );
    let snapshot = match Snapshot::open(&bytes) {
        Ok(value) => value,
        Err(error) => panic!("snapshot open failed: {error:?}"),
    };
    let graph = CsrSnapshotGraph::<u16, u16>::from_snapshot(&snapshot)?;

    assert_eq!(graph.node_count(), 2);
    assert_eq!(
        graph
            .outgoing_edges(CsrNodeId(0u16))
            .map(|edge| graph.target(edge))
            .collect::<Vec<_>>(),
        [CsrNodeId(1u16), CsrNodeId(0u16)]
    );

    Ok(())
}

#[test]
fn opens_u64_snapshot_as_csr_graph() -> Result<(), CsrSnapshotError<u64, u64>> {
    let bytes = build_csr_snapshot_from_bytes(
        SNAPSHOT_KIND_CSR_OFFSETS_U64,
        SNAPSHOT_KIND_CSR_TARGETS_U64,
        u64_words_to_bytes(&[0, 1, 1]),
        u64_words_to_bytes(&[1]),
    );
    let snapshot = match Snapshot::open(&bytes) {
        Ok(value) => value,
        Err(error) => panic!("snapshot open failed: {error:?}"),
    };
    let graph = CsrSnapshotGraph::<u64, u64>::from_snapshot(&snapshot)?;

    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.target(CsrEdgeId(0u64)), CsrNodeId(1u64));

    Ok(())
}

#[test]
fn opens_mixed_u32_targets_u64_offsets() -> Result<(), CsrSnapshotError<u32, u64>> {
    let bytes = build_csr_snapshot_from_bytes(
        SNAPSHOT_KIND_CSR_OFFSETS_U64,
        SNAPSHOT_KIND_CSR_TARGETS_U32,
        u64_words_to_bytes(&[0, 1, 1]),
        words_to_bytes(&[1]),
    );
    let snapshot = match Snapshot::open(&bytes) {
        Ok(value) => value,
        Err(error) => panic!("snapshot open failed: {error:?}"),
    };
    let graph = CsrSnapshotGraph::<u32, u64>::from_snapshot(&snapshot)?;

    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.target(CsrEdgeId(0u64)), CsrNodeId(1u32));

    Ok(())
}

#[test]
fn bfs_runs_over_snapshot_csr_graph() -> Result<(), FixtureError> {
    let bytes = build_csr_snapshot(&[0, 2, 3, 4, 4], &[1, 2, 2, 3]);
    let snapshot = Snapshot::open(&bytes)?;
    let graph = CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot)?;

    let order: Vec<CsrNodeId<u32>> = match breadth_first_search(&graph, CsrNodeId(0)) {
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
        builder.add_section(SNAPSHOT_KIND_CSR_TARGETS_U32, 0, 2, words_to_bytes(&[0, 1]))
    {
        panic!("targets-only: {error:?}");
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    let snapshot = Snapshot::open(&bytes)?;
    match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::MissingOffsets) => Ok(()),
        other => panic!("expected MissingOffsets, got {other:?}"),
    }
}

#[test]
fn rejects_empty_offsets_section() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot(&[], &[]);
    let snapshot = Snapshot::open(&bytes)?;
    match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::OffsetsEmpty) => Ok(()),
        other => panic!("expected OffsetsEmpty, got {other:?}"),
    }
}

#[test]
fn rejects_target_out_of_range() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot(&[0, 1], &[42]);
    let snapshot = Snapshot::open(&bytes)?;
    match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::Csr(CsrError::TargetOutOfRange { target: 42, .. })) => Ok(()),
        other => panic!("expected Csr(TargetOutOfRange), got {other:?}"),
    }
}

#[test]
fn rejects_non_monotonic_offsets() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot(&[0, 3, 1, 1], &[0, 1, 2]);
    let snapshot = Snapshot::open(&bytes)?;
    match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::Csr(CsrError::NonMonotonicOffset { .. })) => Ok(()),
        other => panic!("expected Csr(NonMonotonicOffset), got {other:?}"),
    }
}

#[test]
fn rejects_wrong_target_width() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot_from_bytes(
        SNAPSHOT_KIND_CSR_OFFSETS_U32,
        SNAPSHOT_KIND_CSR_TARGETS_U16,
        words_to_bytes(&[0, 1]),
        u16_words_to_bytes(&[0]),
    );
    let snapshot = Snapshot::open(&bytes)?;
    match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::MissingTargets) => Ok(()),
        other => panic!("expected MissingTargets, got {other:?}"),
    }
}

#[test]
fn rejects_wrong_offset_width() -> Result<(), SnapshotError> {
    let bytes = build_csr_snapshot_from_bytes(
        SNAPSHOT_KIND_CSR_OFFSETS_U16,
        SNAPSHOT_KIND_CSR_TARGETS_U32,
        u16_words_to_bytes(&[0, 1]),
        words_to_bytes(&[0]),
    );
    let snapshot = Snapshot::open(&bytes)?;
    match CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot) {
        Err(CsrSnapshotError::MissingOffsets) => Ok(()),
        other => panic!("expected MissingOffsets, got {other:?}"),
    }
}
