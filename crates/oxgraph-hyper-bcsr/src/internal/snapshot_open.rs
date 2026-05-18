//! Snapshot-backed constructors for [`BcsrHypergraph`].
//!
//! This module wires width-specific bipartite-CSR section kinds to the
//! borrowed slice inputs accepted by [`BcsrHypergraph::open`].

use oxgraph_snapshot::{Section, Snapshot};

use crate::{
    error::{BcsrSection, BcsrSnapshotError},
    internal::{
        validation::BcsrValidation,
        view::{BcsrHypergraph, BcsrSections},
    },
    word::{BcsrSnapshotIndex, BcsrSnapshotWord},
};

/// Snapshot section bundle for the requested BCSR index widths.
type SnapshotSections<'view, VertexIndex, RelationIndex, IncidenceIndex> = BcsrSections<
    'view,
    <IncidenceIndex as BcsrSnapshotIndex>::LittleEndianWord,
    <VertexIndex as BcsrSnapshotIndex>::LittleEndianWord,
    <RelationIndex as BcsrSnapshotIndex>::LittleEndianWord,
>;

impl<'view, VertexIndex, RelationIndex, IncidenceIndex>
    BcsrHypergraph<
        'view,
        VertexIndex,
        RelationIndex,
        IncidenceIndex,
        <IncidenceIndex as BcsrSnapshotIndex>::LittleEndianWord,
        <VertexIndex as BcsrSnapshotIndex>::LittleEndianWord,
        <RelationIndex as BcsrSnapshotIndex>::LittleEndianWord,
    >
where
    VertexIndex: BcsrSnapshotIndex,
    RelationIndex: BcsrSnapshotIndex,
    IncidenceIndex: BcsrSnapshotIndex,
{
    /// Opens a [`BcsrHypergraph`] backed by sections from `snapshot`.
    ///
    /// # Errors
    ///
    /// Returns [`BcsrSnapshotError`] when any required section is missing,
    /// has the wrong width for the requested typed view, or fails
    /// bipartite-CSR validation.
    ///
    /// # Performance
    ///
    /// `O(s + P_head + P_tail + P_outgoing + P_incoming)` for `s` snapshot
    /// sections and the four payload counts.
    pub fn from_snapshot(snapshot: &Snapshot<'view>) -> Result<Self, BcsrSnapshotError> {
        Self::from_snapshot_with(snapshot, BcsrValidation::Layout)
    }

    /// Opens a [`BcsrHypergraph`] from `snapshot` at the requested validation level.
    ///
    /// # Errors
    ///
    /// Returns [`BcsrSnapshotError`] for missing or wrong-width sections, and
    /// any [`crate::BcsrError`] surfaced through validation.
    ///
    /// # Performance
    ///
    /// As [`Self::from_snapshot`] at [`BcsrValidation::Layout`]; with
    /// [`BcsrValidation::Strict`], adds `O((P_head + P_tail) * log d)` for
    /// the cross-direction walk.
    pub fn from_snapshot_with(
        snapshot: &Snapshot<'view>,
        level: BcsrValidation,
    ) -> Result<Self, BcsrSnapshotError> {
        let sections = load_sections::<VertexIndex, RelationIndex, IncidenceIndex>(snapshot)?;
        Ok(Self::open_with(sections, level)?)
    }
}

/// Loads the eight bipartite-CSR sections from `snapshot`.
fn load_sections<'view, VertexIndex, RelationIndex, IncidenceIndex>(
    snapshot: &Snapshot<'view>,
) -> Result<SnapshotSections<'view, VertexIndex, RelationIndex, IncidenceIndex>, BcsrSnapshotError>
where
    VertexIndex: BcsrSnapshotIndex,
    RelationIndex: BcsrSnapshotIndex,
    IncidenceIndex: BcsrSnapshotIndex,
{
    let head_offsets = load_section(
        snapshot,
        BcsrSection::HeadOffsets,
        IncidenceIndex::HEAD_OFFSETS_KIND,
    )?;
    let head_participants = load_section(
        snapshot,
        BcsrSection::HeadParticipants,
        VertexIndex::HEAD_PARTICIPANTS_KIND,
    )?;
    let tail_offsets = load_section(
        snapshot,
        BcsrSection::TailOffsets,
        IncidenceIndex::TAIL_OFFSETS_KIND,
    )?;
    let tail_participants = load_section(
        snapshot,
        BcsrSection::TailParticipants,
        VertexIndex::TAIL_PARTICIPANTS_KIND,
    )?;
    let vertex_outgoing_offsets = load_section(
        snapshot,
        BcsrSection::VertexOutgoingOffsets,
        IncidenceIndex::VERTEX_OUTGOING_OFFSETS_KIND,
    )?;
    let vertex_outgoing_hyperedges = load_section(
        snapshot,
        BcsrSection::VertexOutgoingHyperedges,
        RelationIndex::VERTEX_OUTGOING_HYPEREDGES_KIND,
    )?;
    let vertex_incoming_offsets = load_section(
        snapshot,
        BcsrSection::VertexIncomingOffsets,
        IncidenceIndex::VERTEX_INCOMING_OFFSETS_KIND,
    )?;
    let vertex_incoming_hyperedges = load_section(
        snapshot,
        BcsrSection::VertexIncomingHyperedges,
        RelationIndex::VERTEX_INCOMING_HYPEREDGES_KIND,
    )?;
    Ok(BcsrSections {
        head_offsets,
        head_participants,
        tail_offsets,
        tail_participants,
        vertex_outgoing_offsets,
        vertex_outgoing_hyperedges,
        vertex_incoming_offsets,
        vertex_incoming_hyperedges,
    })
}

/// Looks up `kind` in `snapshot` and borrows its payload as a typed word slice.
fn load_section<'view, Word>(
    snapshot: &Snapshot<'view>,
    section: BcsrSection,
    kind: u32,
) -> Result<&'view [Word], BcsrSnapshotError>
where
    Word: BcsrSnapshotWord,
{
    let payload = lookup_section(snapshot, section, kind)?;
    payload
        .try_as_slice()
        .map_err(|error| BcsrSnapshotError::SectionView { section, error })
}

/// Returns the [`Section`] with `kind`, mapping a missing section to a typed error.
fn lookup_section<'view>(
    snapshot: &Snapshot<'view>,
    section: BcsrSection,
    kind: u32,
) -> Result<Section<'view>, BcsrSnapshotError> {
    snapshot
        .section(kind)
        .ok_or(BcsrSnapshotError::MissingSection { section, kind })
}
