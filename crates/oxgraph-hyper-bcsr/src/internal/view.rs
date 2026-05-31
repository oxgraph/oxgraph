//! Borrowed bipartite-CSR hypergraph view and its caller-facing borrowed
//! section parameter struct.

use crate::{
    error::BcsrError,
    internal::validation::{BcsrValidation, DerivedCounts, validate_sections},
    word::LayoutWord,
};

/// Borrowed input slices for a bipartite-CSR hypergraph view.
///
/// Offset slices use `OffsetWord` and decode to the incidence index width.
/// Participant value slices use `VertexWord` and decode to the vertex index
/// width. Vertex-major relation slices use `RelationWord` and decode to the
/// relation index width.
///
/// # Performance
///
/// `perf: unspecified`; this is a borrowed parameter struct.
#[derive(Clone, Copy, Debug)]
pub struct BcsrSections<'view, OffsetWord, VertexWord, RelationWord>
where
    OffsetWord: LayoutWord,
    VertexWord: LayoutWord,
    RelationWord: LayoutWord,
{
    /// Hyperedge-major head offsets, length `hyperedge_count + 1`.
    pub head_offsets: &'view [OffsetWord],
    /// Flat vertex IDs in head sets, length `P_head`.
    pub head_participants: &'view [VertexWord],
    /// Hyperedge-major tail offsets, length `hyperedge_count + 1`.
    pub tail_offsets: &'view [OffsetWord],
    /// Flat vertex IDs in tail sets, length `P_tail`.
    pub tail_participants: &'view [VertexWord],
    /// Vertex-major outgoing offsets, length `vertex_count + 1`.
    pub vertex_outgoing_offsets: &'view [OffsetWord],
    /// Flat hyperedge IDs where the vertex is in head, length `P_outgoing`.
    pub vertex_outgoing_hyperedges: &'view [RelationWord],
    /// Vertex-major incoming offsets, length `vertex_count + 1`.
    pub vertex_incoming_offsets: &'view [OffsetWord],
    /// Flat hyperedge IDs where the vertex is in tail, length `P_incoming`.
    pub vertex_incoming_hyperedges: &'view [RelationWord],
}

#[cfg(kani)]
impl<'view, OffsetWord, VertexWord, RelationWord>
    BcsrSections<'view, OffsetWord, VertexWord, RelationWord>
where
    OffsetWord: LayoutWord,
    VertexWord: LayoutWord,
    RelationWord: LayoutWord,
{
    /// Builds a [`BcsrSections`] from eight ordered borrowed slices.
    ///
    /// Exists only to deduplicate the kani proof harnesses: each `#[kani::proof]`
    /// otherwise repeats the eight-field struct literal verbatim. Argument
    /// order matches the field declaration order above (head, tail,
    /// vertex-outgoing, vertex-incoming; offsets paired with their value
    /// slice).
    ///
    /// # Performance
    ///
    /// `O(1)` field assignment.
    #[expect(
        clippy::too_many_arguments,
        reason = "eight slice arguments mirror the eight BcsrSections fields in the documented declaration order"
    )]
    pub(crate) fn for_kani(
        head_offsets: &'view [OffsetWord],
        head_participants: &'view [VertexWord],
        tail_offsets: &'view [OffsetWord],
        tail_participants: &'view [VertexWord],
        vertex_outgoing_offsets: &'view [OffsetWord],
        vertex_outgoing_hyperedges: &'view [RelationWord],
        vertex_incoming_offsets: &'view [OffsetWord],
        vertex_incoming_hyperedges: &'view [RelationWord],
    ) -> Self {
        Self {
            head_offsets,
            head_participants,
            tail_offsets,
            tail_participants,
            vertex_outgoing_offsets,
            vertex_outgoing_hyperedges,
            vertex_incoming_offsets,
            vertex_incoming_hyperedges,
        }
    }
}

/// Borrowed bipartite compressed-sparse-row hypergraph view.
///
/// `BcsrHypergraph` borrows the eight section payloads supplied through
/// [`BcsrSections`] without copying or allocating. Construction validates the
/// borrowed slices according to the chosen [`BcsrValidation`] level. Once
/// constructed, every traversal is `O(degree)` in either direction.
///
/// # Performance
///
/// Construction is `O(P_head + P_tail + P_outgoing + P_incoming)` at
/// [`BcsrValidation::Layout`]. [`BcsrValidation::Strict`] adds an
/// `O((P_head + P_tail) · log d)` cross-direction walk where `d` is the
/// maximum vertex outgoing or incoming degree.
#[derive(Clone, Copy, Debug)]
pub struct BcsrHypergraph<
    'view,
    VertexIndex,
    RelationIndex,
    IncidenceIndex,
    OffsetWord,
    VertexWord,
    RelationWord,
> where
    OffsetWord: LayoutWord<Index = IncidenceIndex>,
    VertexWord: LayoutWord<Index = VertexIndex>,
    RelationWord: LayoutWord<Index = RelationIndex>,
    VertexIndex: crate::word::LayoutIndex,
    RelationIndex: crate::word::LayoutIndex,
    IncidenceIndex: crate::word::LayoutIndex,
{
    /// Validated counts cached for `O(1)` access.
    counts: DerivedCounts,
    /// The eight borrowed sections backing this view.
    sections: BcsrSections<'view, OffsetWord, VertexWord, RelationWord>,
}

impl<'view, VertexIndex, RelationIndex, IncidenceIndex, OffsetWord, VertexWord, RelationWord>
    BcsrHypergraph<
        'view,
        VertexIndex,
        RelationIndex,
        IncidenceIndex,
        OffsetWord,
        VertexWord,
        RelationWord,
    >
where
    OffsetWord: LayoutWord<Index = IncidenceIndex>,
    VertexWord: LayoutWord<Index = VertexIndex>,
    RelationWord: LayoutWord<Index = RelationIndex>,
    VertexIndex: crate::word::LayoutIndex,
    RelationIndex: crate::word::LayoutIndex,
    IncidenceIndex: crate::word::LayoutIndex,
{
    /// Validates `sections` at [`BcsrValidation::Layout`] and returns a view.
    ///
    /// # Errors
    ///
    /// Returns [`BcsrError`] when any layout invariant fails. See
    /// [`BcsrValidation::Layout`] for the full list.
    ///
    /// # Performance
    ///
    /// `O(P_head + P_tail + P_outgoing + P_incoming)`.
    pub fn open(
        sections: BcsrSections<'view, OffsetWord, VertexWord, RelationWord>,
    ) -> Result<Self, BcsrError> {
        Self::open_with(sections, BcsrValidation::Layout)
    }

    /// Validates `sections` at the requested level and returns a view.
    ///
    /// # Errors
    ///
    /// Returns [`BcsrError`] when any invariant visible at `level` fails.
    ///
    /// # Performance
    ///
    /// `O(P_head + P_tail + P_outgoing + P_incoming)` at
    /// [`BcsrValidation::Layout`]; adds `O((P_head + P_tail) · log d)` at
    /// [`BcsrValidation::Strict`].
    pub fn open_with(
        sections: BcsrSections<'view, OffsetWord, VertexWord, RelationWord>,
        level: BcsrValidation,
    ) -> Result<Self, BcsrError> {
        let counts = validate_sections(&sections, level)?;
        Ok(Self { counts, sections })
    }

    /// Returns the number of vertices in this view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn vertex_count(&self) -> usize {
        self.counts.vertex_count
    }

    /// Returns the number of hyperedges in this view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn hyperedge_count(&self) -> usize {
        self.counts.hyperedge_count
    }

    /// Returns the number of outgoing incidences (`P_head == P_outgoing`).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn outgoing_incidence_count(&self) -> usize {
        self.counts.p_outgoing
    }

    /// Returns the number of incoming incidences (`P_tail == P_incoming`).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn incoming_incidence_count(&self) -> usize {
        self.counts.p_incoming
    }

    /// Returns the validated count cache.
    pub(in crate::internal) const fn counts(&self) -> DerivedCounts {
        self.counts
    }

    /// Returns the borrowed sections.
    pub(in crate::internal) const fn sections(
        &self,
    ) -> &BcsrSections<'view, OffsetWord, VertexWord, RelationWord> {
        &self.sections
    }
}
