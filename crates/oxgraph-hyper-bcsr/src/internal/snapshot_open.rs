//! Snapshot-backed constructors for [`BcsrHypergraph`].
//!
//! This module wires width-specific bipartite-CSR section kinds to the
//! borrowed slice inputs accepted by [`BcsrHypergraph::open`].

use oxgraph_layout_util::SnapshotWidth;
use oxgraph_snapshot::{SectionBindError, Snapshot};

use crate::{
    error::{BcsrSection, BcsrSnapshotError},
    internal::{
        validation::BcsrValidation,
        view::{BcsrHypergraph, BcsrSections},
    },
    word::{BcsrSnapshotIndex, LeWords},
};

/// Snapshot section bundle for the requested BCSR index widths.
type SnapshotSections<'view, VertexIndex, RelationIndex, IncidenceIndex> = BcsrSections<
    'view,
    <IncidenceIndex as SnapshotWidth>::LittleEndianWord,
    <VertexIndex as SnapshotWidth>::LittleEndianWord,
    <RelationIndex as SnapshotWidth>::LittleEndianWord,
>;

impl<'view, VertexIndex, RelationIndex, IncidenceIndex>
    BcsrHypergraph<'view, LeWords<VertexIndex, RelationIndex, IncidenceIndex>>
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
    let head_offsets = load_section::<IncidenceIndex>(
        snapshot,
        BcsrSection::HeadOffsets,
        IncidenceIndex::HEAD_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
    )?;
    let head_participants = load_section::<VertexIndex>(
        snapshot,
        BcsrSection::HeadParticipants,
        VertexIndex::HEAD_PARTICIPANTS_KIND,
        VertexIndex::SECTION_VERSION,
    )?;
    let tail_offsets = load_section::<IncidenceIndex>(
        snapshot,
        BcsrSection::TailOffsets,
        IncidenceIndex::TAIL_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
    )?;
    let tail_participants = load_section::<VertexIndex>(
        snapshot,
        BcsrSection::TailParticipants,
        VertexIndex::TAIL_PARTICIPANTS_KIND,
        VertexIndex::SECTION_VERSION,
    )?;
    let vertex_outgoing_offsets = load_section::<IncidenceIndex>(
        snapshot,
        BcsrSection::VertexOutgoingOffsets,
        IncidenceIndex::VERTEX_OUTGOING_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
    )?;
    let vertex_outgoing_hyperedges = load_section::<RelationIndex>(
        snapshot,
        BcsrSection::VertexOutgoingHyperedges,
        RelationIndex::VERTEX_OUTGOING_HYPEREDGES_KIND,
        RelationIndex::SECTION_VERSION,
    )?;
    let vertex_incoming_offsets = load_section::<IncidenceIndex>(
        snapshot,
        BcsrSection::VertexIncomingOffsets,
        IncidenceIndex::VERTEX_INCOMING_OFFSETS_KIND,
        IncidenceIndex::SECTION_VERSION,
    )?;
    let vertex_incoming_hyperedges = load_section::<RelationIndex>(
        snapshot,
        BcsrSection::VertexIncomingHyperedges,
        RelationIndex::VERTEX_INCOMING_HYPEREDGES_KIND,
        RelationIndex::SECTION_VERSION,
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

/// Binds `kind` in `snapshot` as a little-endian word slice for width `W`.
///
/// Wraps [`Snapshot::typed_section`] and maps the shared
/// [`SectionBindError`] into the BCSR-specific [`BcsrSnapshotError`] variants.
fn load_section<'view, W>(
    snapshot: &Snapshot<'view>,
    section: BcsrSection,
    kind: u32,
    version: u32,
) -> Result<&'view [W::LittleEndianWord], BcsrSnapshotError>
where
    W: SnapshotWidth,
{
    snapshot
        .typed_section::<W>(kind, version)
        .map_err(|error| map_section_bind(section, error))
}

/// Maps a [`SectionBindError`] into the BCSR-specific snapshot error.
const fn map_section_bind(section: BcsrSection, error: SectionBindError) -> BcsrSnapshotError {
    match error {
        SectionBindError::Missing { kind } => BcsrSnapshotError::MissingSection { section, kind },
        SectionBindError::VersionMismatch {
            kind,
            expected,
            actual,
        } => BcsrSnapshotError::VersionMismatch {
            section,
            kind,
            expected,
            actual,
        },
        SectionBindError::View { error, .. } => BcsrSnapshotError::SectionView { section, error },
    }
}
