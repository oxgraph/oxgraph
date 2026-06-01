//! Tests for opening a [`BcsrHypergraph`] from an `oxgraph-snapshot` container.

use oxgraph_hyper::DirectedHyperedgeParticipants;
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHyperedgeId, BcsrSection, BcsrSnapshotError, BcsrSnapshotHypergraph,
    BcsrVertexId, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64,
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

/// Encodes `[u64]` words as little-endian bytes.
fn words64_to_bytes(words: &[u64]) -> Vec<u8> {
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
        (SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32, &fixture.head_offsets),
        (
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32,
            &fixture.head_participants,
        ),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32, &fixture.tail_offsets),
        (
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32,
            &fixture.tail_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
            &fixture.vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
            &fixture.vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
            &fixture.vertex_incoming_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
            &fixture.vertex_incoming_hyperedges,
        ),
    ];
    for (kind, words) in entries {
        if let Err(error) = builder.add_section(
            kind,
            oxgraph_hyper_bcsr::SNAPSHOT_BCSR_SECTION_VERSION,
            2,
            words_to_bytes(words),
        ) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("builder finish: {error:?}"),
    }
}

#[test]
fn from_snapshot_round_trips_canonical_fixture() -> Result<(), FixtureError> {
    let fixture = Fixture::canonical();
    let bytes = build_snapshot(&fixture);
    let snapshot = Snapshot::open(&bytes)?;
    let view = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot)?;

    assert_eq!(view.vertex_count(), 3);
    assert_eq!(view.hyperedge_count(), 2);
    let h0_heads: Vec<_> = view.source_participants(BcsrHyperedgeId::new(0)).collect();
    assert_eq!(h0_heads, vec![BcsrVertexId::new(0)]);
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "fixture builds all eight width-mixed BCSR sections inline for one round-trip"
)]
fn opens_mixed_u32_vertices_relations_u64_incidences() -> Result<(), FixtureError> {
    let fixture = Fixture::canonical();
    let head_offsets: Vec<u64> = fixture
        .head_offsets
        .iter()
        .copied()
        .map(u64::from)
        .collect();
    let tail_offsets: Vec<u64> = fixture
        .tail_offsets
        .iter()
        .copied()
        .map(u64::from)
        .collect();
    let vertex_outgoing_offsets: Vec<u64> = fixture
        .vertex_outgoing_offsets
        .iter()
        .copied()
        .map(u64::from)
        .collect();
    let vertex_incoming_offsets: Vec<u64> = fixture
        .vertex_incoming_offsets
        .iter()
        .copied()
        .map(u64::from)
        .collect();

    let mut builder = SnapshotBuilder::new();
    let offset_entries: [(u32, &[u64]); 4] = [
        (SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64, &head_offsets),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64, &tail_offsets),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64,
            &vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64,
            &vertex_incoming_offsets,
        ),
    ];
    for (kind, words) in offset_entries {
        if let Err(error) = builder.add_section(
            kind,
            oxgraph_hyper_bcsr::SNAPSHOT_BCSR_SECTION_VERSION,
            3,
            words64_to_bytes(words),
        ) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    let value_entries: [(u32, &[u32]); 4] = [
        (
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32,
            &fixture.head_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32,
            &fixture.tail_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
            &fixture.vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
            &fixture.vertex_incoming_hyperedges,
        ),
    ];
    for (kind, words) in value_entries {
        if let Err(error) = builder.add_section(
            kind,
            oxgraph_hyper_bcsr::SNAPSHOT_BCSR_SECTION_VERSION,
            2,
            words_to_bytes(words),
        ) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    let snapshot = Snapshot::open(&bytes)?;
    let view = BcsrSnapshotHypergraph::<u32, u32, u64>::from_snapshot(&snapshot)?;

    assert_eq!(view.vertex_count(), 3);
    assert_eq!(view.hyperedge_count(), 2);
    assert_eq!(
        view.target_participants(BcsrHyperedgeId::new(0))
            .collect::<Vec<_>>(),
        vec![BcsrVertexId::new(1), BcsrVertexId::new(2)]
    );
    Ok(())
}

#[test]
fn rejects_missing_head_offsets_section() -> Result<(), FixtureError> {
    let fixture = Fixture::canonical();
    let mut builder = SnapshotBuilder::new();
    let entries: [(u32, &[u32]); 7] = [
        (
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32,
            &fixture.head_participants,
        ),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32, &fixture.tail_offsets),
        (
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32,
            &fixture.tail_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
            &fixture.vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
            &fixture.vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
            &fixture.vertex_incoming_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
            &fixture.vertex_incoming_hyperedges,
        ),
    ];
    for (kind, words) in entries {
        if let Err(error) = builder.add_section(
            kind,
            oxgraph_hyper_bcsr::SNAPSHOT_BCSR_SECTION_VERSION,
            2,
            words_to_bytes(words),
        ) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    let snapshot = Snapshot::open(&bytes)?;
    let result = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot);
    let Err(BcsrSnapshotError::MissingSection { section, kind }) = result else {
        panic!("expected MissingSection, got {result:?}");
    };
    assert_eq!(section, BcsrSection::HeadOffsets);
    assert_eq!(kind, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32);
    Ok(())
}

#[test]
fn rejects_wrong_offset_width() -> Result<(), FixtureError> {
    let fixture = Fixture::canonical();
    let bytes = build_snapshot(&fixture);
    let snapshot = Snapshot::open(&bytes)?;
    let result = BcsrSnapshotHypergraph::<u32, u32, u64>::from_snapshot(&snapshot);
    let Err(BcsrSnapshotError::MissingSection { section, kind }) = result else {
        panic!("expected MissingSection, got {result:?}");
    };
    assert_eq!(section, BcsrSection::HeadOffsets);
    assert_eq!(kind, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64);
    Ok(())
}

#[test]
fn rejects_wrong_participant_width() -> Result<(), FixtureError> {
    let fixture = Fixture::canonical();
    let mut builder = SnapshotBuilder::new();
    let entries: [(u32, &[u32]); 8] = [
        (SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32, &fixture.head_offsets),
        (
            SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32,
            &fixture.head_participants,
        ),
        (SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32, &fixture.tail_offsets),
        (
            SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16,
            &fixture.tail_participants,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
            &fixture.vertex_outgoing_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
            &fixture.vertex_outgoing_hyperedges,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
            &fixture.vertex_incoming_offsets,
        ),
        (
            SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
            &fixture.vertex_incoming_hyperedges,
        ),
    ];
    for (kind, words) in entries {
        if let Err(error) = builder.add_section(
            kind,
            oxgraph_hyper_bcsr::SNAPSHOT_BCSR_SECTION_VERSION,
            2,
            words_to_bytes(words),
        ) {
            panic!("section 0x{kind:04x}: {error:?}");
        }
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    let snapshot = Snapshot::open(&bytes)?;
    let result = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot);
    let Err(BcsrSnapshotError::MissingSection { section, kind }) = result else {
        panic!("expected MissingSection, got {result:?}");
    };
    assert_eq!(section, BcsrSection::TailParticipants);
    assert_eq!(kind, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32);
    Ok(())
}

#[test]
fn rejects_validation_failure_through_snapshot() -> Result<(), FixtureError> {
    let mut fixture = Fixture::canonical();
    fixture.head_participants[0] = 99;
    let bytes = build_snapshot(&fixture);
    let snapshot = Snapshot::open(&bytes)?;
    let result = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot(&snapshot);
    let Err(BcsrSnapshotError::Bcsr(BcsrError::VertexOutOfRange { vertex: 99, .. })) = result
    else {
        panic!("expected Bcsr(VertexOutOfRange), got {result:?}");
    };
    Ok(())
}
