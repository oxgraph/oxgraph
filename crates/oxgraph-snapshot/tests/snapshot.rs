//! Tests for v0 graph snapshot container validation.

use oxgraph_algo::breadth_first_search;
use oxgraph_csr::{CsrError, CsrGraph, CsrNodeId};
use oxgraph_graph::{EdgeTargetGraph, GraphCounts, NodeIndex, OutgoingGraph};
use oxgraph_snapshot::{GraphSnapshot, SnapshotError};
use zerocopy::byteorder::{LE, U32};

/// Snapshot magic bytes for test fixture construction.
const MAGIC: &[u8; 8] = b"OCTXG0\0\0";

/// CSR offsets section kind.
const SECTION_CSR_OFFSETS: u32 = 1;

/// CSR targets section kind.
const SECTION_CSR_TARGETS: u32 = 2;

/// Unknown section kind used by tests.
const SECTION_UNKNOWN: u32 = 99;

/// Section fixture used to build snapshot bytes.
struct SectionSpec<'a> {
    /// Section kind.
    kind: u32,
    /// Section words.
    words: &'a [u32],
}

/// Error returned while opening the fixture's CSR view.
#[derive(Debug)]
enum FixtureError {
    /// Snapshot container validation failed.
    Snapshot(SnapshotError),
    /// A required fixture section was missing.
    MissingSection(u32),
    /// CSR layout validation failed.
    Csr(CsrError),
}

impl From<SnapshotError> for FixtureError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<CsrError> for FixtureError {
    fn from(error: CsrError) -> Self {
        Self::Csr(error)
    }
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot validation failed: {error}"),
            Self::MissingSection(kind) => write!(formatter, "missing fixture section {kind}"),
            Self::Csr(error) => write!(formatter, "CSR validation failed: {error}"),
        }
    }
}

impl std::error::Error for FixtureError {}

/// Appends a little-endian `u32` to `bytes`.
fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Builds a minimal valid snapshot byte vector.
fn valid_snapshot_bytes() -> Vec<u8> {
    let offsets = [0u32, 2, 3, 4, 4];
    let targets = [1u32, 2, 2, 3];
    build_snapshot(4, &offsets, &targets)
}

/// Builds a snapshot byte vector with two CSR sections.
fn build_snapshot(node_count: u32, offsets: &[u32], targets: &[u32]) -> Vec<u8> {
    build_snapshot_sections(
        node_count,
        &[
            SectionSpec {
                kind: SECTION_CSR_OFFSETS,
                words: offsets,
            },
            SectionSpec {
                kind: SECTION_CSR_TARGETS,
                words: targets,
            },
        ],
    )
}

/// Builds a snapshot byte vector with arbitrary word sections.
fn build_snapshot_sections(node_count: u32, sections: &[SectionSpec<'_>]) -> Vec<u8> {
    let header_len = 24u32;
    let section_table_len = usize_to_u32_lossless(sections.len() * 12);
    let mut section_entries = Vec::with_capacity(sections.len());
    let mut next_offset = header_len + section_table_len;

    for section in sections {
        let length = usize_to_u32_lossless(section.words.len() * 4);
        section_entries.push((section.kind, next_offset, length));
        next_offset += length;
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, node_count);
    push_u32(&mut bytes, usize_to_u32_lossless(sections.len()));

    for (kind, offset, length) in &section_entries {
        push_u32(&mut bytes, *kind);
        push_u32(&mut bytes, *offset);
        push_u32(&mut bytes, *length);
    }

    for section in sections {
        for word in section.words {
            push_u32(&mut bytes, *word);
        }
    }

    bytes
}

/// Overwrites a little-endian `u32` at `offset`.
fn set_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Opens the fixture's CSR layout from a validated snapshot container.
fn csr_graph<'view>(
    snapshot: &GraphSnapshot<'view>,
) -> Result<CsrGraph<'view, U32<LE>>, FixtureError> {
    let offsets = snapshot
        .section_words(SECTION_CSR_OFFSETS)?
        .ok_or(FixtureError::MissingSection(SECTION_CSR_OFFSETS))?;
    let targets = snapshot
        .section_words(SECTION_CSR_TARGETS)?
        .ok_or(FixtureError::MissingSection(SECTION_CSR_TARGETS))?;

    CsrGraph::validate(snapshot.node_count(), offsets, targets).map_err(FixtureError::from)
}

/// Converts fixture sizes into `u32`.
fn usize_to_u32_lossless(value: usize) -> u32 {
    match u32::try_from(value) {
        Ok(converted) => converted,
        Err(error) => panic!("fixture value did not fit u32: {error:?}"),
    }
}

#[test]
fn opens_valid_snapshot_as_csr_graph() -> Result<(), FixtureError> {
    let bytes = valid_snapshot_bytes();
    let snapshot = GraphSnapshot::validate(&bytes)?;
    let graph = csr_graph(&snapshot)?;

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
fn opens_zero_node_snapshot() -> Result<(), SnapshotError> {
    let offsets = [0u32];
    let targets = [];
    let bytes = build_snapshot(0, &offsets, &targets);
    let snapshot = GraphSnapshot::validate(&bytes)?;

    assert_eq!(snapshot.node_count(), 0);
    assert_eq!(snapshot.section_count(), 2);

    Ok(())
}

#[test]
fn snapshot_sections_open_as_csr_view() -> Result<(), FixtureError> {
    let bytes = valid_snapshot_bytes();
    let snapshot = GraphSnapshot::validate(&bytes)?;
    let graph = csr_graph(&snapshot)?;

    assert_eq!(graph.node_count(), 4);
    assert_eq!(graph.edge_count(), 4);
    assert_eq!(graph.node_bound(), 4);
    assert_eq!(graph.node_index(CsrNodeId(2)), 2);
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
    let bytes = valid_snapshot_bytes();
    let snapshot = GraphSnapshot::validate(&bytes)?;
    let graph = csr_graph(&snapshot)?;

    assert_eq!(
        breadth_first_search(&graph, CsrNodeId(0)).map(std::iter::Iterator::collect::<Vec<_>>),
        Ok(vec![CsrNodeId(0), CsrNodeId(1), CsrNodeId(2), CsrNodeId(3)])
    );

    Ok(())
}

#[test]
fn rejects_bad_magic() {
    let mut bytes = valid_snapshot_bytes();
    bytes[0] = 0;

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::BadMagic {
            actual: [0, 67, 84, 88, 71, 48, 0, 0],
        })
    );
}

#[test]
fn rejects_unsupported_version() {
    let mut bytes = valid_snapshot_bytes();
    bytes[8] = 1;

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::UnsupportedVersion { major: 1, minor: 1 })
    );
}

#[test]
fn rejects_malformed_header() {
    assert_eq!(
        GraphSnapshot::validate(&[0; 8]).err(),
        Some(SnapshotError::MalformedHeader)
    );
}

#[test]
fn rejects_truncated_section_table() {
    let mut bytes = valid_snapshot_bytes();
    bytes.truncate(40);

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::TruncatedSectionTable {
            needed: 24,
            actual: 16,
        })
    );
}

#[test]
fn accepts_missing_layout_offsets_section() -> Result<(), SnapshotError> {
    let targets = [0u32];
    let bytes = build_snapshot_sections(
        1,
        &[SectionSpec {
            kind: SECTION_CSR_TARGETS,
            words: &targets,
        }],
    );

    let snapshot = GraphSnapshot::validate(&bytes)?;
    assert_eq!(snapshot.section_count(), 1);
    assert!(snapshot.section_words(SECTION_CSR_OFFSETS)?.is_none());

    Ok(())
}

#[test]
fn accepts_missing_layout_targets_section() -> Result<(), SnapshotError> {
    let offsets = [0u32, 0];
    let bytes = build_snapshot_sections(
        1,
        &[SectionSpec {
            kind: SECTION_CSR_OFFSETS,
            words: &offsets,
        }],
    );

    let snapshot = GraphSnapshot::validate(&bytes)?;
    assert_eq!(snapshot.section_count(), 1);
    assert!(snapshot.section_words(SECTION_CSR_TARGETS)?.is_none());

    Ok(())
}

#[test]
fn rejects_duplicate_offsets_section() {
    let offsets = [0u32, 0];
    let targets = [];
    let bytes = build_snapshot_sections(
        1,
        &[
            SectionSpec {
                kind: SECTION_CSR_OFFSETS,
                words: &offsets,
            },
            SectionSpec {
                kind: SECTION_CSR_OFFSETS,
                words: &offsets,
            },
            SectionSpec {
                kind: SECTION_CSR_TARGETS,
                words: &targets,
            },
        ],
    );

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::DuplicateSection {
            kind: SECTION_CSR_OFFSETS,
        })
    );
}

#[test]
fn rejects_duplicate_targets_section() {
    let offsets = [0u32, 0];
    let targets = [];
    let bytes = build_snapshot_sections(
        1,
        &[
            SectionSpec {
                kind: SECTION_CSR_OFFSETS,
                words: &offsets,
            },
            SectionSpec {
                kind: SECTION_CSR_TARGETS,
                words: &targets,
            },
            SectionSpec {
                kind: SECTION_CSR_TARGETS,
                words: &targets,
            },
        ],
    );

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::DuplicateSection {
            kind: SECTION_CSR_TARGETS,
        })
    );
}

#[test]
fn accepts_unknown_section_with_valid_range() -> Result<(), SnapshotError> {
    let unknown = [42u32];
    let offsets = [0u32, 0];
    let targets = [];
    let bytes = build_snapshot_sections(
        1,
        &[
            SectionSpec {
                kind: SECTION_UNKNOWN,
                words: &unknown,
            },
            SectionSpec {
                kind: SECTION_CSR_OFFSETS,
                words: &offsets,
            },
            SectionSpec {
                kind: SECTION_CSR_TARGETS,
                words: &targets,
            },
        ],
    );

    let snapshot = GraphSnapshot::validate(&bytes)?;
    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.section_count(), 3);
    assert_eq!(
        snapshot.section_bytes(SECTION_UNKNOWN),
        Some(&[42, 0, 0, 0][..])
    );

    Ok(())
}

#[test]
fn rejects_unknown_section_with_invalid_range() {
    let unknown = [42u32];
    let offsets = [0u32, 0];
    let targets = [];
    let mut bytes = build_snapshot_sections(
        1,
        &[
            SectionSpec {
                kind: SECTION_UNKNOWN,
                words: &unknown,
            },
            SectionSpec {
                kind: SECTION_CSR_OFFSETS,
                words: &offsets,
            },
            SectionSpec {
                kind: SECTION_CSR_TARGETS,
                words: &targets,
            },
        ],
    );
    set_u32(&mut bytes, 28, 1_000);

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::SectionOutOfBounds {
            kind: SECTION_UNKNOWN,
            offset: 1_000,
            length: 4,
            snapshot_len: bytes.len(),
        })
    );
}

#[test]
fn rejects_required_section_out_of_bounds() {
    let mut bytes = valid_snapshot_bytes();
    set_u32(&mut bytes, 28, 1_000);

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::SectionOutOfBounds {
            kind: SECTION_CSR_OFFSETS,
            offset: 1_000,
            length: 20,
            snapshot_len: bytes.len(),
        })
    );
}

#[test]
fn rejects_overlapping_sections() {
    let mut bytes = valid_snapshot_bytes();
    set_u32(&mut bytes, 40, 48);

    assert_eq!(
        GraphSnapshot::validate(&bytes).err(),
        Some(SnapshotError::SectionOverlap {
            first_kind: SECTION_CSR_OFFSETS,
            second_kind: SECTION_CSR_TARGETS,
        })
    );
}

#[test]
fn csr_layout_validation_remains_outside_snapshot() -> Result<(), FixtureError> {
    let offsets = [0u32, 1];
    let targets = [1u32];
    let bytes = build_snapshot(1, &offsets, &targets);
    let snapshot = GraphSnapshot::validate(&bytes)?;
    let offsets = snapshot
        .section_words(SECTION_CSR_OFFSETS)?
        .ok_or(FixtureError::MissingSection(SECTION_CSR_OFFSETS))?;
    let targets = snapshot
        .section_words(SECTION_CSR_TARGETS)?
        .ok_or(FixtureError::MissingSection(SECTION_CSR_TARGETS))?;

    assert_eq!(
        CsrGraph::validate(snapshot.node_count(), offsets, targets).err(),
        Some(CsrError::TargetOutOfRange {
            index: 0,
            target: 1,
            node_count: 1,
        })
    );

    Ok(())
}

#[test]
fn rejects_word_view_with_non_multiple_of_four_length() -> Result<(), SnapshotError> {
    let mut bytes = valid_snapshot_bytes();
    bytes[32] = 19;
    let snapshot = GraphSnapshot::validate(&bytes)?;

    assert_eq!(
        snapshot.section_words(SECTION_CSR_OFFSETS).err(),
        Some(SnapshotError::MisalignedWordLength {
            kind: SECTION_CSR_OFFSETS,
            length: 19,
        })
    );

    Ok(())
}
