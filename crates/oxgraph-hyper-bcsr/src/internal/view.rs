//! Borrowed bipartite-CSR hypergraph view.

use crate::{
    error::BcsrError,
    internal::validation::{
        BcsrValidation, DerivedCounts, u32_to_usize_validated, validate_sections,
    },
    sections::BcsrSections,
    word::BcsrWord,
};

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
pub struct BcsrHypergraph<'view, Word: BcsrWord = u32> {
    /// Validated counts cached for `O(1)` access.
    counts: DerivedCounts,
    /// The eight borrowed sections backing this view.
    sections: BcsrSections<'view, Word>,
}

impl<'view, Word: BcsrWord> BcsrHypergraph<'view, Word> {
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
    pub fn open(sections: BcsrSections<'view, Word>) -> Result<Self, BcsrError> {
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
        sections: BcsrSections<'view, Word>,
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
    pub fn vertex_count(&self) -> usize {
        u32_to_usize_validated(self.counts.vertex_count)
    }

    /// Returns the number of hyperedges in this view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn hyperedge_count(&self) -> usize {
        u32_to_usize_validated(self.counts.hyperedge_count)
    }

    /// Returns the number of outgoing incidences (`P_head == P_outgoing`).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn outgoing_incidence_count(&self) -> usize {
        u32_to_usize_validated(self.counts.p_outgoing)
    }

    /// Returns the number of incoming incidences (`P_tail == P_incoming`).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn incoming_incidence_count(&self) -> usize {
        u32_to_usize_validated(self.counts.p_incoming)
    }

    /// Returns the validated count cache.
    pub(in crate::internal) const fn counts(&self) -> DerivedCounts {
        self.counts
    }

    /// Returns the borrowed sections.
    pub(in crate::internal) const fn sections(&self) -> &BcsrSections<'view, Word> {
        &self.sections
    }
}
