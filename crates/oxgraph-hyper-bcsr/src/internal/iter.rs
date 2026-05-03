//! Iterator types yielded by [`BcsrHypergraph`] traversal traits.
//!
//! Every iterator borrows directly from validated section payloads and never
//! allocates. Single-bucket iterators are also `ExactSizeIterator +
//! DoubleEndedIterator`. Two-bucket chains are `ExactSizeIterator` when both
//! halves are. Nested expansion iterators (successors, predecessors) are
//! `Iterator` only because their per-step inner length depends on the
//! visited hyperedge.
//!
//! [`BcsrHypergraph`]: crate::BcsrHypergraph

use core::iter::Chain;

use crate::{
    id::{BcsrHyperedgeId, BcsrParticipantId, BcsrVertexId},
    internal::validation::u32_to_usize_validated,
    sections::BcsrSections,
    word::BcsrWord,
};

/// Iterator over a borrowed slice of vertex words, yielding [`BcsrVertexId`].
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrVertexSlice<'view, Word> {
    /// Remaining vertex words for this slice.
    inner: core::slice::Iter<'view, Word>,
}

impl<'view, Word: BcsrWord> BcsrVertexSlice<'view, Word> {
    /// Constructs a slice iterator from a borrowed `Word` slice.
    pub(in crate::internal) fn new(slice: &'view [Word]) -> Self {
        Self {
            inner: slice.iter(),
        }
    }
}

impl<Word: BcsrWord> Iterator for BcsrVertexSlice<'_, Word> {
    type Item = BcsrVertexId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|word| BcsrVertexId(word.get()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<Word: BcsrWord> ExactSizeIterator for BcsrVertexSlice<'_, Word> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<Word: BcsrWord> DoubleEndedIterator for BcsrVertexSlice<'_, Word> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|word| BcsrVertexId(word.get()))
    }
}

/// Iterator over a borrowed slice of hyperedge words, yielding [`BcsrHyperedgeId`].
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrHyperedgeSlice<'view, Word> {
    /// Remaining hyperedge words for this slice.
    inner: core::slice::Iter<'view, Word>,
}

impl<'view, Word: BcsrWord> BcsrHyperedgeSlice<'view, Word> {
    /// Constructs a slice iterator from a borrowed `Word` slice.
    pub(in crate::internal) fn new(slice: &'view [Word]) -> Self {
        Self {
            inner: slice.iter(),
        }
    }
}

impl<Word: BcsrWord> Iterator for BcsrHyperedgeSlice<'_, Word> {
    type Item = BcsrHyperedgeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|word| BcsrHyperedgeId(word.get()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<Word: BcsrWord> ExactSizeIterator for BcsrHyperedgeSlice<'_, Word> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<Word: BcsrWord> DoubleEndedIterator for BcsrHyperedgeSlice<'_, Word> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner
            .next_back()
            .map(|word| BcsrHyperedgeId(word.get()))
    }
}

/// Chained vertex iterator yielding head participants then tail participants.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
pub type BcsrChainedParticipants<'view, Word> =
    Chain<BcsrVertexSlice<'view, Word>, BcsrVertexSlice<'view, Word>>;

/// Chained hyperedge iterator yielding outgoing then incoming hyperedges.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
pub type BcsrChainedHyperedges<'view, Word> =
    Chain<BcsrHyperedgeSlice<'view, Word>, BcsrHyperedgeSlice<'view, Word>>;

/// Iterator yielding [`BcsrParticipantId`] for a contiguous incidence range.
///
/// One slice spans `[start, end)` positions in either `head_participants`
/// (head incidence IDs) or `tail_participants` (tail incidence IDs offset by
/// `P_head`). The `base` field is `0` for head incidences and `P_head` for
/// tail incidences.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrParticipantSlice {
    /// Next position to yield.
    cursor: u32,
    /// Exclusive end position.
    end: u32,
    /// Offset added to each position (`0` for head, `P_head` for tail).
    base: u32,
}

impl BcsrParticipantSlice {
    /// Constructs a participant slice over `[start, end)` with the given `base`.
    pub(in crate::internal) const fn new(start: u32, end: u32, base: u32) -> Self {
        Self {
            cursor: start,
            end,
            base,
        }
    }
}

/// Side of the bipartite incidence space a participant slice walks.
///
/// Used internally to discriminate head and tail iterators when both yield
/// `BcsrParticipantId`.
///
/// # Performance
///
/// `perf: unspecified`; this is a metadata enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BcsrRelationIncidencesSide {
    /// Head incidences, IDs in `[0, P_head)`.
    Head,
    /// Tail incidences, IDs in `[P_head, P_head + P_tail)`.
    Tail,
}

impl Iterator for BcsrParticipantSlice {
    type Item = BcsrParticipantId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor == self.end {
            return None;
        }
        let id = BcsrParticipantId(self.base.wrapping_add(self.cursor));
        self.cursor += 1;
        Some(id)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = u32_to_usize_validated(self.end - self.cursor);
        (len, Some(len))
    }
}

impl ExactSizeIterator for BcsrParticipantSlice {
    fn len(&self) -> usize {
        u32_to_usize_validated(self.end - self.cursor)
    }
}

impl DoubleEndedIterator for BcsrParticipantSlice {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.cursor == self.end {
            return None;
        }
        self.end -= 1;
        Some(BcsrParticipantId(self.base.wrapping_add(self.end)))
    }
}

/// Chained participant-ID iterator yielding head incidences then tail
/// incidences for a single hyperedge.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
pub type BcsrChainedRelationIncidences = Chain<BcsrParticipantSlice, BcsrParticipantSlice>;

/// Iterator yielding [`BcsrParticipantId`]s for one vertex's incidences.
///
/// Walks the vertex's outgoing-hyperedge bucket and locates each incidence's
/// position inside `head_participants`, then does the same for the incoming
/// bucket against `tail_participants`. Each step performs a binary search,
/// giving `O(log d_h)` per element where `d_h` is the maximum head/tail
/// degree of any single hyperedge.
///
/// # Performance
///
/// Advancing the iterator is `O(log d_h)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrElementIncidences<'view, Word: BcsrWord> {
    /// The vertex whose incidences we are listing.
    vertex: u32,
    /// `P_head`, used to offset tail incidence IDs.
    p_head: u32,
    /// Remaining outgoing-hyperedge words for this vertex.
    outgoing: core::slice::Iter<'view, Word>,
    /// Remaining incoming-hyperedge words for this vertex.
    incoming: core::slice::Iter<'view, Word>,
    /// Hyperedge-major head offsets.
    head_offsets: &'view [Word],
    /// Hyperedge-major head participant payload.
    head_participants: &'view [Word],
    /// Hyperedge-major tail offsets.
    tail_offsets: &'view [Word],
    /// Hyperedge-major tail participant payload.
    tail_participants: &'view [Word],
}

impl<'view, Word: BcsrWord> BcsrElementIncidences<'view, Word> {
    /// Constructs an element-incidences iterator for `vertex`.
    pub(in crate::internal) fn new(
        vertex: u32,
        p_head: u32,
        outgoing_slice: &'view [Word],
        incoming_slice: &'view [Word],
        sections: &BcsrSections<'view, Word>,
    ) -> Self {
        Self {
            vertex,
            p_head,
            outgoing: outgoing_slice.iter(),
            incoming: incoming_slice.iter(),
            head_offsets: sections.head_offsets,
            head_participants: sections.head_participants,
            tail_offsets: sections.tail_offsets,
            tail_participants: sections.tail_participants,
        }
    }

    /// Tries to yield the next outgoing-side incidence ID, skipping any
    /// hyperedges that do not list this vertex (only possible at
    /// `BcsrValidation::Layout`).
    fn pull_outgoing(&mut self) -> Option<BcsrParticipantId> {
        for h_word in self.outgoing.by_ref() {
            let hyperedge = h_word.get();
            if let Some(position) = locate_in_bucket(
                self.head_offsets,
                self.head_participants,
                hyperedge,
                self.vertex,
            ) {
                return Some(BcsrParticipantId(position));
            }
        }
        None
    }

    /// Tries to yield the next incoming-side incidence ID.
    fn pull_incoming(&mut self) -> Option<BcsrParticipantId> {
        for h_word in self.incoming.by_ref() {
            let hyperedge = h_word.get();
            if let Some(position) = locate_in_bucket(
                self.tail_offsets,
                self.tail_participants,
                hyperedge,
                self.vertex,
            ) {
                return Some(BcsrParticipantId(self.p_head.wrapping_add(position)));
            }
        }
        None
    }
}

impl<Word: BcsrWord> Iterator for BcsrElementIncidences<'_, Word> {
    type Item = BcsrParticipantId;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(id) = self.pull_outgoing() {
            return Some(id);
        }
        self.pull_incoming()
    }
}

/// Returns the absolute position of `vertex` inside the bucket
/// `participants[offsets[hyperedge as usize]..offsets[hyperedge as usize + 1]]`,
/// or `None` if not present. The bucket is required to be strictly ascending
/// (validated at open time), so binary search is correct.
fn locate_in_bucket<Word: BcsrWord>(
    offsets: &[Word],
    participants: &[Word],
    hyperedge: u32,
    vertex: u32,
) -> Option<u32> {
    let h_index = u32_to_usize_validated(hyperedge);
    let bucket_start = u32_to_usize_validated(offsets[h_index].get());
    let bucket_end = u32_to_usize_validated(offsets[h_index + 1].get());
    let bucket = &participants[bucket_start..bucket_end];
    let local = bucket
        .binary_search_by(|word| word.get().cmp(&vertex))
        .ok()?;
    let absolute = bucket_start.checked_add(local)?;
    u32::try_from(absolute).ok()
}

/// Iterator over successor vertices of a directed-hypergraph vertex.
///
/// For each hyperedge in the vertex's outgoing-hyperedge bucket, yields the
/// vertices in that hyperedge's tail set in order. Successors may include
/// duplicates if the same vertex appears as a tail in multiple hyperedges.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` amortised and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrSuccessorVertices<'view, Word: BcsrWord> {
    /// Remaining outgoing hyperedges for the source vertex.
    outgoing: core::slice::Iter<'view, Word>,
    /// Remaining vertices in the current hyperedge's tail bucket.
    current_tail: core::slice::Iter<'view, Word>,
    /// Hyperedge-major tail offsets.
    tail_offsets: &'view [Word],
    /// Hyperedge-major tail participants.
    tail_participants: &'view [Word],
}

impl<'view, Word: BcsrWord> BcsrSuccessorVertices<'view, Word> {
    /// Constructs a successor iterator for the given outgoing-hyperedge slice.
    pub(in crate::internal) fn new(
        outgoing_slice: &'view [Word],
        tail_offsets: &'view [Word],
        tail_participants: &'view [Word],
    ) -> Self {
        Self {
            outgoing: outgoing_slice.iter(),
            current_tail: [].iter(),
            tail_offsets,
            tail_participants,
        }
    }

    /// Reloads `current_tail` from the next outgoing hyperedge if any remain.
    /// Returns `false` when both the outer cursor and the current tail are
    /// exhausted.
    fn advance_outer(&mut self) -> bool {
        match self.outgoing.next() {
            Some(h_word) => {
                let h_index = u32_to_usize_validated(h_word.get());
                let start = u32_to_usize_validated(self.tail_offsets[h_index].get());
                let end = u32_to_usize_validated(self.tail_offsets[h_index + 1].get());
                self.current_tail = self.tail_participants[start..end].iter();
                true
            }
            None => false,
        }
    }
}

impl<Word: BcsrWord> Iterator for BcsrSuccessorVertices<'_, Word> {
    type Item = BcsrVertexId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(word) = self.current_tail.next() {
                return Some(BcsrVertexId(word.get()));
            }
            if !self.advance_outer() {
                return None;
            }
        }
    }
}

/// Iterator over predecessor vertices of a directed-hypergraph vertex.
///
/// Symmetric to [`BcsrSuccessorVertices`]: walks the vertex's
/// incoming-hyperedge bucket and yields the head participants of each visited
/// hyperedge.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` amortised and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrPredecessorVertices<'view, Word: BcsrWord> {
    /// Remaining incoming hyperedges for the target vertex.
    incoming: core::slice::Iter<'view, Word>,
    /// Remaining vertices in the current hyperedge's head bucket.
    current_head: core::slice::Iter<'view, Word>,
    /// Hyperedge-major head offsets.
    head_offsets: &'view [Word],
    /// Hyperedge-major head participants.
    head_participants: &'view [Word],
}

impl<'view, Word: BcsrWord> BcsrPredecessorVertices<'view, Word> {
    /// Constructs a predecessor iterator for the given incoming-hyperedge slice.
    pub(in crate::internal) fn new(
        incoming_slice: &'view [Word],
        head_offsets: &'view [Word],
        head_participants: &'view [Word],
    ) -> Self {
        Self {
            incoming: incoming_slice.iter(),
            current_head: [].iter(),
            head_offsets,
            head_participants,
        }
    }

    /// Reloads `current_head` from the next incoming hyperedge if any remain.
    fn advance_outer(&mut self) -> bool {
        match self.incoming.next() {
            Some(h_word) => {
                let h_index = u32_to_usize_validated(h_word.get());
                let start = u32_to_usize_validated(self.head_offsets[h_index].get());
                let end = u32_to_usize_validated(self.head_offsets[h_index + 1].get());
                self.current_head = self.head_participants[start..end].iter();
                true
            }
            None => false,
        }
    }
}

impl<Word: BcsrWord> Iterator for BcsrPredecessorVertices<'_, Word> {
    type Item = BcsrVertexId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(word) = self.current_head.next() {
                return Some(BcsrVertexId(word.get()));
            }
            if !self.advance_outer() {
                return None;
            }
        }
    }
}
