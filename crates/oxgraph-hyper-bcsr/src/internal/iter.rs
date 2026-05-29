//! Iterator types yielded by [`BcsrHypergraph`] traversal traits.
//!
//! [`BcsrHypergraph`]: crate::BcsrHypergraph

use core::{iter::Chain, marker::PhantomData};

use crate::{
    id::{BcsrHyperedgeId, BcsrParticipantId, BcsrVertexId},
    internal::{
        validation::{index_to_usize_validated, usize_to_index_validated},
        view::BcsrSections,
    },
    word::{BcsrIndex, BcsrWord},
};

/// Iterator over a borrowed slice of words, mapping each word's decoded
/// index to a destination newtype via `Id::from`.
///
/// Used to back both [`BcsrVertexSlice`] and [`BcsrHyperedgeSlice`] (and any
/// future `BcsrIdSlice<Word, NewtypeId>` flavor) through `pub type` aliases.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrIdSlice<'view, Word: BcsrWord, Id> {
    /// Remaining words for this slice.
    inner: core::slice::Iter<'view, Word>,
    /// Brands the iterator to the destination ID newtype without coupling
    /// `Send` / `Sync` to it (function-return `PhantomData` is covariant).
    _id: PhantomData<fn() -> Id>,
}

impl<'view, Word: BcsrWord, Id> BcsrIdSlice<'view, Word, Id> {
    /// Constructs a slice iterator from a borrowed `Word` slice.
    pub(in crate::internal) fn new(slice: &'view [Word]) -> Self {
        Self {
            inner: slice.iter(),
            _id: PhantomData,
        }
    }
}

impl<Word: BcsrWord, Id> Iterator for BcsrIdSlice<'_, Word, Id>
where
    Id: From<Word::Index>,
{
    type Item = Id;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|word| Id::from(word.get()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<Word: BcsrWord, Id> ExactSizeIterator for BcsrIdSlice<'_, Word, Id>
where
    Id: From<Word::Index>,
{
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<Word: BcsrWord, Id> DoubleEndedIterator for BcsrIdSlice<'_, Word, Id>
where
    Id: From<Word::Index>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|word| Id::from(word.get()))
    }
}

/// Iterator over a borrowed slice of vertex words.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
pub type BcsrVertexSlice<'view, Word> =
    BcsrIdSlice<'view, Word, BcsrVertexId<<Word as BcsrWord>::Index>>;

/// Iterator over a borrowed slice of hyperedge words.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
pub type BcsrHyperedgeSlice<'view, Word> =
    BcsrIdSlice<'view, Word, BcsrHyperedgeId<<Word as BcsrWord>::Index>>;

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

/// Iterator yielding incidence IDs for a contiguous incidence range.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrParticipantSlice<IncidenceIndex> {
    /// Next position to yield.
    cursor: usize,
    /// Exclusive end position.
    end: usize,
    /// Offset added to each position.
    base: usize,
    /// Logical incidence index type.
    index: PhantomData<fn() -> IncidenceIndex>,
}

impl<IncidenceIndex> BcsrParticipantSlice<IncidenceIndex> {
    /// Constructs a participant slice over `[start, end)` with the given `base`.
    pub(in crate::internal) const fn new(start: usize, end: usize, base: usize) -> Self {
        Self {
            cursor: start,
            end,
            base,
            index: PhantomData,
        }
    }
}

impl<IncidenceIndex: BcsrIndex> Iterator for BcsrParticipantSlice<IncidenceIndex> {
    type Item = BcsrParticipantId<IncidenceIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor == self.end {
            return None;
        }
        let id = self.base.checked_add(self.cursor)?;
        self.cursor += 1;
        Some(BcsrParticipantId::new(usize_to_index_validated(id)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.end - self.cursor;
        (len, Some(len))
    }
}

impl<IncidenceIndex: BcsrIndex> ExactSizeIterator for BcsrParticipantSlice<IncidenceIndex> {
    fn len(&self) -> usize {
        self.end - self.cursor
    }
}

impl<IncidenceIndex: BcsrIndex> DoubleEndedIterator for BcsrParticipantSlice<IncidenceIndex> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.cursor == self.end {
            return None;
        }
        self.end -= 1;
        let id = self.base.checked_add(self.end)?;
        Some(BcsrParticipantId::new(usize_to_index_validated(id)))
    }
}

/// Chained participant-ID iterator yielding head incidences then tail
/// incidences for a single hyperedge.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
pub type BcsrChainedRelationIncidences<IncidenceIndex> =
    Chain<BcsrParticipantSlice<IncidenceIndex>, BcsrParticipantSlice<IncidenceIndex>>;

/// Iterator yielding incidence IDs for one vertex's incidences.
///
/// # Performance
///
/// Advancing the iterator is `O(log d_h)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrElementIncidences<'view, OffsetWord, VertexWord, RelationWord>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    /// The vertex whose incidences we are listing.
    vertex: usize,
    /// `P_head`, used to offset tail incidence IDs.
    p_head: usize,
    /// Remaining outgoing-hyperedge words for this vertex.
    outgoing: core::slice::Iter<'view, RelationWord>,
    /// Remaining incoming-hyperedge words for this vertex.
    incoming: core::slice::Iter<'view, RelationWord>,
    /// Hyperedge-major head offsets.
    head_offsets: &'view [OffsetWord],
    /// Hyperedge-major head participant payload.
    head_participants: &'view [VertexWord],
    /// Hyperedge-major tail offsets.
    tail_offsets: &'view [OffsetWord],
    /// Hyperedge-major tail participant payload.
    tail_participants: &'view [VertexWord],
}

impl<'view, OffsetWord, VertexWord, RelationWord>
    BcsrElementIncidences<'view, OffsetWord, VertexWord, RelationWord>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    /// Constructs an element-incidences iterator for `vertex`.
    pub(in crate::internal) fn new(
        vertex: usize,
        p_head: usize,
        outgoing_slice: &'view [RelationWord],
        incoming_slice: &'view [RelationWord],
        sections: &BcsrSections<'view, OffsetWord, VertexWord, RelationWord>,
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

    /// Tries to yield the next outgoing-side incidence ID.
    fn pull_outgoing(&mut self) -> Option<BcsrParticipantId<OffsetWord::Index>> {
        for h_word in self.outgoing.by_ref() {
            let hyperedge = index_to_usize_validated(h_word.get());
            if let Some(position) = locate_in_bucket(
                self.head_offsets,
                self.head_participants,
                hyperedge,
                self.vertex,
            ) {
                return Some(BcsrParticipantId::new(usize_to_index_validated(position)));
            }
        }
        None
    }

    /// Tries to yield the next incoming-side incidence ID.
    fn pull_incoming(&mut self) -> Option<BcsrParticipantId<OffsetWord::Index>> {
        for h_word in self.incoming.by_ref() {
            let hyperedge = index_to_usize_validated(h_word.get());
            if let Some(position) = locate_in_bucket(
                self.tail_offsets,
                self.tail_participants,
                hyperedge,
                self.vertex,
            ) {
                let id = self.p_head.checked_add(position)?;
                return Some(BcsrParticipantId::new(usize_to_index_validated(id)));
            }
        }
        None
    }
}

impl<OffsetWord, VertexWord, RelationWord> Iterator
    for BcsrElementIncidences<'_, OffsetWord, VertexWord, RelationWord>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    type Item = BcsrParticipantId<OffsetWord::Index>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(id) = self.pull_outgoing() {
            return Some(id);
        }
        self.pull_incoming()
    }
}

/// Returns the absolute position of `vertex` inside one hyperedge bucket.
fn locate_in_bucket<OffsetWord, VertexWord>(
    offsets: &[OffsetWord],
    participants: &[VertexWord],
    hyperedge: usize,
    vertex: usize,
) -> Option<usize>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
{
    let bucket_start = index_to_usize_validated(offsets[hyperedge].get());
    let bucket_end = index_to_usize_validated(offsets[hyperedge + 1].get());
    let bucket = &participants[bucket_start..bucket_end];
    let local = bucket
        .binary_search_by(|word| index_to_usize_validated(word.get()).cmp(&vertex))
        .ok()?;
    bucket_start.checked_add(local)
}

/// Iterator over neighbor vertices reached through a sequence of hyperedges
/// resolved against an (offsets, participants) pair.
///
/// Backing type for both [`BcsrSuccessorVertices`] (outgoing relations →
/// tail buckets) and [`BcsrPredecessorVertices`] (incoming relations →
/// head buckets). Direction is encoded by which sections the constructor
/// receives; the struct itself is direction-neutral.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` amortised and performs no allocation.
#[derive(Clone, Debug)]
pub struct BcsrNeighborVertices<'view, RelationWord, OffsetWord, VertexWord>
where
    RelationWord: BcsrWord,
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
{
    /// Remaining hyperedges along this traversal direction.
    relations: core::slice::Iter<'view, RelationWord>,
    /// Remaining vertices in the current hyperedge's resolved bucket.
    current_bucket: core::slice::Iter<'view, VertexWord>,
    /// Hyperedge-major bucket offsets for the chosen direction (tail for
    /// successors, head for predecessors).
    bucket_offsets: &'view [OffsetWord],
    /// Hyperedge-major participants for the chosen direction.
    bucket_participants: &'view [VertexWord],
}

impl<'view, RelationWord, OffsetWord, VertexWord>
    BcsrNeighborVertices<'view, RelationWord, OffsetWord, VertexWord>
where
    RelationWord: BcsrWord,
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
{
    /// Constructs a neighbor iterator from a relation slice and the
    /// (offsets, participants) pair for the desired direction. Use the
    /// `tail_offsets` / `tail_participants` for successor traversal and
    /// `head_offsets` / `head_participants` for predecessor traversal.
    pub(in crate::internal) fn new(
        relations: &'view [RelationWord],
        bucket_offsets: &'view [OffsetWord],
        bucket_participants: &'view [VertexWord],
    ) -> Self {
        Self {
            relations: relations.iter(),
            current_bucket: [].iter(),
            bucket_offsets,
            bucket_participants,
        }
    }

    /// Reloads `current_bucket` from the next relation if any remain.
    fn advance_outer(&mut self) -> bool {
        match self.relations.next() {
            Some(h_word) => {
                let h_index = index_to_usize_validated(h_word.get());
                let start = index_to_usize_validated(self.bucket_offsets[h_index].get());
                let end = index_to_usize_validated(self.bucket_offsets[h_index + 1].get());
                self.current_bucket = self.bucket_participants[start..end].iter();
                true
            }
            None => false,
        }
    }
}

impl<RelationWord, OffsetWord, VertexWord> Iterator
    for BcsrNeighborVertices<'_, RelationWord, OffsetWord, VertexWord>
where
    RelationWord: BcsrWord,
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
{
    type Item = BcsrVertexId<VertexWord::Index>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(word) = self.current_bucket.next() {
                return Some(BcsrVertexId::new(word.get()));
            }
            if !self.advance_outer() {
                return None;
            }
        }
    }
}

/// Iterator over successor vertices of a directed-hypergraph vertex.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` amortised and performs no allocation.
pub type BcsrSuccessorVertices<'view, RelationWord, OffsetWord, VertexWord> =
    BcsrNeighborVertices<'view, RelationWord, OffsetWord, VertexWord>;

/// Iterator over predecessor vertices of a directed-hypergraph vertex.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` amortised and performs no allocation.
pub type BcsrPredecessorVertices<'view, RelationWord, OffsetWord, VertexWord> =
    BcsrNeighborVertices<'view, RelationWord, OffsetWord, VertexWord>;
