//! Snapshot-backed constructors for [`BcsrHypergraph`].
//!
//! This module wires the eight bipartite-CSR section kinds defined in
//! [`crate::snapshot`] to the borrowed slice inputs accepted by
//! [`BcsrHypergraph::open`] and [`BcsrHypergraph::open_with`].

use oxgraph_snapshot::{Section, Snapshot};
use zerocopy::byteorder::{LE, U32};

use crate::{
    error::{BcsrSection, BcsrSnapshotError},
    internal::{validation::BcsrValidation, view::BcsrHypergraph},
    sections::BcsrSections,
    snapshot::{
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
    },
};

impl<'view> BcsrHypergraph<'view, U32<LE>> {
    /// Opens a [`BcsrHypergraph`] backed by sections from `snapshot`.
    ///
    /// Reads the eight bipartite-CSR section kinds, validates each payload as
    /// `&[U32<LE>]`, and runs [`BcsrValidation::Layout`] on the borrowed
    /// slices. The returned view borrows directly from the snapshot's byte
    /// region — no copying.
    ///
    /// # Errors
    ///
    /// Returns [`BcsrSnapshotError`] when any required section is missing,
    /// misaligned, or fails bipartite-CSR validation.
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
    /// Returns [`BcsrSnapshotError`] for missing or misaligned sections, and
    /// any [`crate::BcsrError`] surfaced through validation.
    ///
    /// # Performance
    ///
    /// As [`Self::from_snapshot`] at [`BcsrValidation::Layout`]; with
    /// [`BcsrValidation::Strict`], adds `O((P_head + P_tail) · log d)` for
    /// the cross-direction walk.
    pub fn from_snapshot_with(
        snapshot: &Snapshot<'view>,
        level: BcsrValidation,
    ) -> Result<Self, BcsrSnapshotError> {
        let sections = load_sections(snapshot)?;
        Self::open_with(sections, level).map_err(BcsrSnapshotError::Validation)
    }
}

/// Loads the eight bipartite-CSR sections from `snapshot` as `&[U32<LE>]` slices.
///
/// # Errors
///
/// Returns [`BcsrSnapshotError`] when any section is missing or has a payload
/// that cannot be borrowed as little-endian `u32`.
fn load_sections<'view>(
    snapshot: &Snapshot<'view>,
) -> Result<BcsrSections<'view, U32<LE>>, BcsrSnapshotError> {
    let head_offsets = load_section(
        snapshot,
        BcsrSection::HeadOffsets,
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS,
    )?;
    let head_participants = load_section(
        snapshot,
        BcsrSection::HeadParticipants,
        SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
    )?;
    let tail_offsets = load_section(
        snapshot,
        BcsrSection::TailOffsets,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS,
    )?;
    let tail_participants = load_section(
        snapshot,
        BcsrSection::TailParticipants,
        SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
    )?;
    let vertex_outgoing_offsets = load_section(
        snapshot,
        BcsrSection::VertexOutgoingOffsets,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
    )?;
    let vertex_outgoing_hyperedges = load_section(
        snapshot,
        BcsrSection::VertexOutgoingHyperedges,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES,
    )?;
    let vertex_incoming_offsets = load_section(
        snapshot,
        BcsrSection::VertexIncomingOffsets,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
    )?;
    let vertex_incoming_hyperedges = load_section(
        snapshot,
        BcsrSection::VertexIncomingHyperedges,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES,
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

/// Looks up `kind` in `snapshot` and borrows its payload as `&[U32<LE>]`.
fn load_section<'view>(
    snapshot: &Snapshot<'view>,
    section: BcsrSection,
    kind: u32,
) -> Result<&'view [U32<LE>], BcsrSnapshotError> {
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
