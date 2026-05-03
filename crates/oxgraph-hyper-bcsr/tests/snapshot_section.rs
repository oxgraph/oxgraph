//! Tests for opening a [`BcsrHypergraph`] from an `oxgraph-snapshot` container.

use oxgraph_hyper::DirectedHyperedgeParticipants;
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHyperedgeId, BcsrHypergraph, BcsrSection, BcsrSnapshotError, BcsrVertexId,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotError};

/// Test fixture error covering snapshot, view, and bipartite-CSR failure modes.
#[derive(Debug)]
enum FixtureError {
    /// Snapshot container validation failed.
    Snapshot(SnapshotError),
    /// Bipartite-CSR adaptor failed.
    Adaptor(BcsrSnapshotError),
}

impl From<SnapshotError> for FixtureError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<BcsrSnapshotError> for FixtureError {
    fn from(error: BcsrSnapshotError) -> Self {
        Self::Adaptor(error)
    }
}

impl core::fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot validation failed: {error}"),
            Self::Adaptor(error) => write!(formatter, "bipartite-CSR adaptor failed: {error}"),
        }
    }
}

impl std::error::Error for FixtureError {}

/// Encodes `[u32]` words as little-endian bytes.
fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Hand-built canonical bipartite-CSR fixture as raw u32 word vectors.
struct Fixture {
    head_offsets: Vec<u32>,
    head_participants: Vec<u32>,
    tail_offsets: Vec<u32>,
    tail_participants: Vec<u32>,
    vertex_outgoing_offsets: Vec<u32>,
    vertex_outgoing_hyperedges: Vec<u32>,
    vertex_incoming_offsets: Vec<u32>,
    vertex_incoming_hyperedges: Vec<u32>,
}

impl Fixture {
    fn canonical() -> Self {
        Self {
            head_offsets: vec![0, 1, 2],
            head_participants: vec![0, 1],
            tail_offsets: vec![0, 2, 3],
            tail_participants: vec![1, 2, 2],
            vertex_outgoing_offsets: vec![0, 1, 2, 2],
            vertex_outgoing_hyperedges: vec![0, 1],
            vertex_incoming_offsets: vec![0, 0, 1, 3],
            vertex_incoming_hyperedges: vec![0, 0, 1],
        }
    }
}

/// Builds a snapshot from a [`Fixture`] using the eight bipartite-CSR section kinds.
fn build_snapshot(fixture: &Fixture) -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    let entries: [(u32, &[u32]); 8] = [
        (SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, &fixture.head_offsets),
        (
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
            &fixture.head_participants,
        ),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, &fixture.tail_offsets),
        (
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
            &fixture.tail_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
            &fixture.vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
            &fixture.vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
            &fixture.vertex_incoming_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
            &fixture.vertex_incoming_hyperedges,
        ),
    ];
    for (kind, words) in entries {
        if let Err(error) = builder.add_section(kind, 0, 2, words_to_bytes(words)) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    builder.finish()
}

#[test]
fn from_snapshot_round_trips_canonical_fixture() -> Result<(), FixtureError> {
    let fixture = Fixture::canonical();
    let bytes = build_snapshot(&fixture);
    let snapshot = Snapshot::open(&bytes)?;
    let view = BcsrHypergraph::from_snapshot(&snapshot)?;

    assert_eq!(view.vertex_count(), 3);
    assert_eq!(view.hyperedge_count(), 2);
    let h0_heads: Vec<_> = view.source_participants(BcsrHyperedgeId(0)).collect();
    assert_eq!(h0_heads, vec![BcsrVertexId(0)]);
    Ok(())
}

#[test]
fn rejects_missing_head_offsets_section() -> Result<(), FixtureError> {
    let fixture = Fixture::canonical();
    let mut builder = SnapshotBuilder::new();
    let entries: [(u32, &[u32]); 7] = [
        (
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
            &fixture.head_participants,
        ),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, &fixture.tail_offsets),
        (
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
            &fixture.tail_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
            &fixture.vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
            &fixture.vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
            &fixture.vertex_incoming_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
            &fixture.vertex_incoming_hyperedges,
        ),
    ];
    for (kind, words) in entries {
        if let Err(error) = builder.add_section(kind, 0, 2, words_to_bytes(words)) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    let bytes = builder.finish();
    let snapshot = Snapshot::open(&bytes)?;
    let result = BcsrHypergraph::from_snapshot(&snapshot);
    let Err(BcsrSnapshotError::MissingSection { section, kind }) = result else {
        panic!("expected MissingSection, got {result:?}");
    };
    assert_eq!(section, BcsrSection::HeadOffsets);
    assert_eq!(kind, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS);
    Ok(())
}

#[test]
fn rejects_validation_failure_through_snapshot() -> Result<(), FixtureError> {
    let mut fixture = Fixture::canonical();
    fixture.head_participants[0] = 99;
    let bytes = build_snapshot(&fixture);
    let snapshot = Snapshot::open(&bytes)?;
    let result = BcsrHypergraph::from_snapshot(&snapshot);
    let Err(BcsrSnapshotError::Validation(BcsrError::VertexOutOfRange { vertex: 99, .. })) = result
    else {
        panic!("expected Validation(VertexOutOfRange), got {result:?}");
    };
    Ok(())
}
