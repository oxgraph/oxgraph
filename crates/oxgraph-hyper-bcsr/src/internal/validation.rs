//! Layout- and Strict-tier validation for bipartite-CSR section payloads.
//!
//! Validation walks the eight section payloads at open time and rejects any
//! shape that would let a later traversal step out of bounds, observe stale
//! data, or yield an inconsistent view of the bipartite incidence relation.
//! The depth of the walk is selected by [`BcsrValidation`].

use oxgraph_layout_util::{OffsetIntegrityIssue, check_offset_section, check_value_range};

use crate::{
    error::{BcsrError, BcsrRoleSide, BcsrSection},
    internal::view::BcsrSections,
    word::{BcsrIndex, BcsrWord},
};

/// Maps an [`OffsetIntegrityIssue`] from `oxgraph-csr-util` into a typed
/// [`BcsrError`], stamping the originating [`BcsrSection`] discriminator.
fn map_offsets_issue<W: BcsrWord>(
    section: BcsrSection,
    offsets: &[W],
    issue: OffsetIntegrityIssue,
) -> BcsrError {
    match issue {
        OffsetIntegrityIssue::Length { expected, actual } => BcsrError::OffsetLength {
            section,
            expected,
            actual,
        },
        OffsetIntegrityIssue::FirstNonZero { actual } => BcsrError::FirstOffset { section, actual },
        OffsetIntegrityIssue::NonMonotonic {
            index,
            previous,
            actual,
        } => BcsrError::NonMonotonicOffset {
            section,
            index,
            previous,
            actual,
        },
        OffsetIntegrityIssue::FinalMismatch {
            final_offset,
            value_len,
        } => BcsrError::FinalOffset {
            section,
            final_offset,
            value_len,
        },
        OffsetIntegrityIssue::UsizeOverflow { index } => {
            let value = offsets
                .get(index)
                .copied()
                .and_then(|w| w.get().to_usize())
                .unwrap_or(usize::MAX);
            BcsrError::UsizeOverflow { value }
        }
        _ => BcsrError::UsizeOverflow { value: usize::MAX },
    }
}

/// Maps a value-range [`OffsetIntegrityIssue`] for vertex IDs into a
/// [`BcsrError::VertexOutOfRange`].
fn map_vertex_value_issue<W: BcsrWord>(
    section: BcsrSection,
    values: &[W],
    issue: OffsetIntegrityIssue,
) -> BcsrError {
    match issue {
        OffsetIntegrityIssue::ValueOutOfRange {
            index,
            value,
            bound,
        } => BcsrError::VertexOutOfRange {
            section,
            index,
            vertex: value,
            vertex_count: bound,
        },
        OffsetIntegrityIssue::UsizeOverflow { index } => {
            let value = values
                .get(index)
                .copied()
                .and_then(|w| w.get().to_usize())
                .unwrap_or(usize::MAX);
            BcsrError::UsizeOverflow { value }
        }
        _ => BcsrError::UsizeOverflow { value: usize::MAX },
    }
}

/// Maps a value-range [`OffsetIntegrityIssue`] for hyperedge IDs into a
/// [`BcsrError::HyperedgeOutOfRange`].
fn map_hyperedge_value_issue<W: BcsrWord>(
    section: BcsrSection,
    values: &[W],
    issue: OffsetIntegrityIssue,
) -> BcsrError {
    match issue {
        OffsetIntegrityIssue::ValueOutOfRange {
            index,
            value,
            bound,
        } => BcsrError::HyperedgeOutOfRange {
            section,
            index,
            hyperedge: value,
            hyperedge_count: bound,
        },
        OffsetIntegrityIssue::UsizeOverflow { index } => {
            let value = values
                .get(index)
                .copied()
                .and_then(|w| w.get().to_usize())
                .unwrap_or(usize::MAX);
            BcsrError::UsizeOverflow { value }
        }
        _ => BcsrError::UsizeOverflow { value: usize::MAX },
    }
}

/// Validation depth applied at view open time.
///
/// `Layout` is the cheap default and catches every violation that lets a
/// downstream traversal walk out of bounds. `Strict` additionally verifies
/// cross-direction consistency: the hyperedge-major and vertex-major
/// indexes describe the same set of incidences. `Strict` is required for
/// end-to-end semantic guarantees on untrusted producers.
///
/// # Performance
///
/// `perf: unspecified`; this is a metadata enum.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BcsrValidation {
    /// Length, monotonicity, in-range IDs, sorted-and-unique within ranges.
    /// Cost is `O(P_head + P_tail + P_outgoing + P_incoming)`.
    Layout,
    /// Layout plus cross-direction multiset equality.
    /// Cost is `O((P_head + P_tail) · log d)` where `d` is the maximum
    /// vertex outgoing or incoming degree.
    Strict,
}

/// Counts derived from validated offset arrays.
///
/// Returned by [`validate_sections`] so the view can store them without
/// recomputing.
///
/// # Performance
///
/// `perf: unspecified`; copying is `O(1)`.
#[derive(Clone, Copy, Debug)]
pub(in crate::internal) struct DerivedCounts {
    /// Number of vertices visible in this view.
    pub(in crate::internal) vertex_count: usize,
    /// Number of hyperedges visible in this view.
    pub(in crate::internal) hyperedge_count: usize,
    /// Number of outgoing incidences (`P_head == P_outgoing`).
    pub(in crate::internal) p_outgoing: usize,
    /// Number of incoming incidences (`P_tail == P_incoming`).
    pub(in crate::internal) p_incoming: usize,
    /// Total incidence count (`P_outgoing + P_incoming`). Validation
    /// guarantees this fits in `usize`.
    pub(in crate::internal) total_incidences: usize,
}

/// Validates the eight bipartite-CSR section payloads.
///
/// Walks the sections in a deterministic order and returns the derived
/// vertex / hyperedge / per-direction incidence counts on success.
///
/// # Errors
///
/// Returns the first [`BcsrError`] encountered during validation.
///
/// # Performance
///
/// At [`BcsrValidation::Layout`] the cost is
/// `O(P_head + P_tail + P_outgoing + P_incoming)`. [`BcsrValidation::Strict`]
/// adds `O((P_head + P_tail) · log d)` for the cross-direction walk.
pub(in crate::internal) fn validate_sections<OffsetWord, VertexWord, RelationWord>(
    sections: &BcsrSections<'_, OffsetWord, VertexWord, RelationWord>,
    level: BcsrValidation,
) -> Result<DerivedCounts, BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    let counts = derive_counts(sections)?;
    validate_all_offsets(sections, counts)?;
    validate_total_lengths(sections)?;
    validate_value_ranges(sections, counts)?;
    validate_within_range_sorted(sections)?;
    if matches!(level, BcsrValidation::Strict) {
        validate_cross_direction(sections)?;
    }
    Ok(counts)
}

/// Derives `vertex_count`, `hyperedge_count`, `P_outgoing`, `P_incoming`
/// after checking offset slice lengths agree pairwise.
fn derive_counts<OffsetWord, VertexWord, RelationWord>(
    sections: &BcsrSections<'_, OffsetWord, VertexWord, RelationWord>,
) -> Result<DerivedCounts, BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    let head_len = sections.head_offsets.len();
    let tail_len = sections.tail_offsets.len();
    if head_len != tail_len {
        return Err(BcsrError::HyperedgeOffsetLengthMismatch {
            head_offsets_len: head_len,
            tail_offsets_len: tail_len,
        });
    }
    let outgoing_len = sections.vertex_outgoing_offsets.len();
    let incoming_len = sections.vertex_incoming_offsets.len();
    if outgoing_len != incoming_len {
        return Err(BcsrError::VertexOffsetLengthMismatch {
            outgoing_offsets_len: outgoing_len,
            incoming_offsets_len: incoming_len,
        });
    }

    let hyperedge_count = derive_count_from_offsets(head_len, BcsrSection::HeadOffsets)?;
    // `head_len == tail_len` was enforced above and `hyperedge_count` is derived
    // from `head_len`, so both paired hyperedge offset arrays are exactly
    // `hyperedge_count + 1` long. This makes the "head offsets are canonical"
    // assumption (head incidence IDs equal head-block positions directly)
    // explicit rather than implicit. The same pairing holds for the vertex
    // outgoing/incoming offset arrays via `outgoing_len == incoming_len`.
    debug_assert_eq!(
        head_len,
        hyperedge_count + 1,
        "head offsets must be hyperedge_count + 1"
    );
    debug_assert_eq!(
        tail_len,
        hyperedge_count + 1,
        "tail offsets must be hyperedge_count + 1 (paired with head)"
    );
    let vertex_count = derive_count_from_offsets(outgoing_len, BcsrSection::VertexOutgoingOffsets)?;
    debug_assert_eq!(
        outgoing_len,
        vertex_count + 1,
        "vertex outgoing offsets must be vertex_count + 1"
    );
    debug_assert_eq!(
        incoming_len,
        vertex_count + 1,
        "vertex incoming offsets must be vertex_count + 1 (paired with outgoing)"
    );
    let p_outgoing = sections.vertex_outgoing_hyperedges.len();
    let p_incoming = sections.vertex_incoming_hyperedges.len();
    let total_incidences =
        p_outgoing
            .checked_add(p_incoming)
            .ok_or(BcsrError::TotalIncidenceCountOverflow {
                p_head: p_outgoing,
                p_tail: p_incoming,
            })?;

    Ok(DerivedCounts {
        vertex_count,
        hyperedge_count,
        p_outgoing,
        p_incoming,
        total_incidences,
    })
}

/// Returns `offsets.len() - 1` after checking length is non-zero.
const fn derive_count_from_offsets(
    offsets_len: usize,
    section: BcsrSection,
) -> Result<usize, BcsrError> {
    if offsets_len == 0 {
        return Err(BcsrError::OffsetLength {
            section,
            expected: 1,
            actual: 0,
        });
    }
    Ok(offsets_len - 1)
}

/// Validates each of the four offset arrays end-to-end.
fn validate_all_offsets<OffsetWord, VertexWord, RelationWord>(
    sections: &BcsrSections<'_, OffsetWord, VertexWord, RelationWord>,
    counts: DerivedCounts,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    validate_one_offsets(
        sections.head_offsets,
        BcsrSection::HeadOffsets,
        counts.hyperedge_count,
        sections.head_participants.len(),
    )?;
    validate_one_offsets(
        sections.tail_offsets,
        BcsrSection::TailOffsets,
        counts.hyperedge_count,
        sections.tail_participants.len(),
    )?;
    validate_one_offsets(
        sections.vertex_outgoing_offsets,
        BcsrSection::VertexOutgoingOffsets,
        counts.vertex_count,
        sections.vertex_outgoing_hyperedges.len(),
    )?;
    validate_one_offsets(
        sections.vertex_incoming_offsets,
        BcsrSection::VertexIncomingOffsets,
        counts.vertex_count,
        sections.vertex_incoming_hyperedges.len(),
    )
}

/// Validates one offset array: length is `count + 1`, first offset is 0,
/// monotonic non-decreasing, final offset matches `value_len`. Delegates to
/// [`oxgraph_layout_util::check_offset_section`] and stamps the [`BcsrSection`]
/// discriminator on any returned issue.
fn validate_one_offsets<Word: BcsrWord>(
    offsets: &[Word],
    section: BcsrSection,
    count: usize,
    value_len: usize,
) -> Result<(), BcsrError> {
    if count.checked_add(1).is_none() {
        return Err(BcsrError::OffsetLengthOverflow { count });
    }
    check_offset_section(offsets, count, value_len)
        .map_err(|issue| map_offsets_issue(section, offsets, issue))
}

/// Verifies that head/outgoing and tail/incoming totals agree, so the
/// bipartite index has a single shared incidence count per direction.
const fn validate_total_lengths<OffsetWord, VertexWord, RelationWord>(
    sections: &BcsrSections<'_, OffsetWord, VertexWord, RelationWord>,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    if sections.head_participants.len() != sections.vertex_outgoing_hyperedges.len() {
        return Err(BcsrError::OutgoingTotalMismatch {
            head_participants_len: sections.head_participants.len(),
            outgoing_hyperedges_len: sections.vertex_outgoing_hyperedges.len(),
        });
    }
    if sections.tail_participants.len() != sections.vertex_incoming_hyperedges.len() {
        return Err(BcsrError::IncomingTotalMismatch {
            tail_participants_len: sections.tail_participants.len(),
            incoming_hyperedges_len: sections.vertex_incoming_hyperedges.len(),
        });
    }
    Ok(())
}

/// Verifies vertex IDs and hyperedge IDs in value sections are in range.
fn validate_value_ranges<OffsetWord, VertexWord, RelationWord>(
    sections: &BcsrSections<'_, OffsetWord, VertexWord, RelationWord>,
    counts: DerivedCounts,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    check_vertex_values(
        sections.head_participants,
        BcsrSection::HeadParticipants,
        counts.vertex_count,
    )?;
    check_vertex_values(
        sections.tail_participants,
        BcsrSection::TailParticipants,
        counts.vertex_count,
    )?;
    check_hyperedge_values(
        sections.vertex_outgoing_hyperedges,
        BcsrSection::VertexOutgoingHyperedges,
        counts.hyperedge_count,
    )?;
    check_hyperedge_values(
        sections.vertex_incoming_hyperedges,
        BcsrSection::VertexIncomingHyperedges,
        counts.hyperedge_count,
    )
}

/// Returns `Err` if any vertex word is `>= vertex_count`. Delegates to
/// [`oxgraph_layout_util::check_value_range`] and stamps [`BcsrSection`] on the
/// returned issue.
fn check_vertex_values<Word: BcsrWord>(
    values: &[Word],
    section: BcsrSection,
    vertex_count: usize,
) -> Result<(), BcsrError> {
    check_value_range(values, vertex_count)
        .map_err(|issue| map_vertex_value_issue(section, values, issue))
}

/// Returns `Err` if any hyperedge word is `>= hyperedge_count`. Delegates to
/// [`oxgraph_layout_util::check_value_range`] and stamps [`BcsrSection`] on the
/// returned issue.
fn check_hyperedge_values<Word: BcsrWord>(
    values: &[Word],
    section: BcsrSection,
    hyperedge_count: usize,
) -> Result<(), BcsrError> {
    check_value_range(values, hyperedge_count)
        .map_err(|issue| map_hyperedge_value_issue(section, values, issue))
}

/// Verifies that values within every per-bucket range are strictly
/// ascending. Bipartite-CSR uses set semantics inside each range.
fn validate_within_range_sorted<OffsetWord, VertexWord, RelationWord>(
    sections: &BcsrSections<'_, OffsetWord, VertexWord, RelationWord>,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    check_strictly_ascending_buckets(
        sections.head_offsets,
        sections.head_participants,
        BcsrSection::HeadParticipants,
    )?;
    check_strictly_ascending_buckets(
        sections.tail_offsets,
        sections.tail_participants,
        BcsrSection::TailParticipants,
    )?;
    check_strictly_ascending_buckets(
        sections.vertex_outgoing_offsets,
        sections.vertex_outgoing_hyperedges,
        BcsrSection::VertexOutgoingHyperedges,
    )?;
    check_strictly_ascending_buckets(
        sections.vertex_incoming_offsets,
        sections.vertex_incoming_hyperedges,
        BcsrSection::VertexIncomingHyperedges,
    )
}

/// Checks each `[offsets[i], offsets[i + 1])` bucket of `values` is
/// strictly ascending.
fn check_strictly_ascending_buckets<OffsetWord, Word>(
    offsets: &[OffsetWord],
    values: &[Word],
    section: BcsrSection,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    Word: BcsrWord,
{
    if offsets.len() < 2 {
        return Ok(());
    }
    for window in offsets.windows(2) {
        let start = index_to_usize(window[0].get())?;
        let end = index_to_usize(window[1].get())?;
        check_strictly_ascending_range(values, start, end, section)?;
    }
    Ok(())
}

/// Verifies `values[start..end]` is strictly ascending.
fn check_strictly_ascending_range<Word: BcsrWord>(
    values: &[Word],
    start: usize,
    end: usize,
    section: BcsrSection,
) -> Result<(), BcsrError> {
    if end <= start + 1 {
        return Ok(());
    }
    let mut previous = index_to_usize(values[start].get())?;
    for relative in 1..(end - start) {
        let index = start + relative;
        let actual = index_to_usize(values[index].get())?;
        if actual <= previous {
            return Err(BcsrError::NotStrictlyAscending {
                section,
                index,
                previous,
                actual,
            });
        }
        previous = actual;
    }
    Ok(())
}

/// Verifies that the hyperedge-major and vertex-major indexes describe the
/// same multiset of incidences (Strict-tier check). Set semantics let the
/// check use binary search; cost is `O((P_head + P_tail) · log d)`.
fn validate_cross_direction<OffsetWord, VertexWord, RelationWord>(
    sections: &BcsrSections<'_, OffsetWord, VertexWord, RelationWord>,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    // Forward containment: every hyperedge-major `(h, v)` incidence appears in
    // vertex `v`'s vertex-major bucket.
    cross_direction_walk(
        sections.head_offsets,
        sections.head_participants,
        sections.vertex_outgoing_offsets,
        sections.vertex_outgoing_hyperedges,
        BcsrRoleSide::Outgoing,
    )?;
    cross_direction_walk(
        sections.tail_offsets,
        sections.tail_participants,
        sections.vertex_incoming_offsets,
        sections.vertex_incoming_hyperedges,
        BcsrRoleSide::Incoming,
    )?;
    // Converse containment: every vertex-major `(v, h)` incidence appears in
    // hyperedge `h`'s hyperedge-major bucket. Forward + converse containment
    // over strictly-ascending (set-semantic) buckets is a true bidirectional
    // multiset equality, not merely one-directional containment plus a count
    // check.
    converse_direction_walk(
        sections.vertex_outgoing_offsets,
        sections.vertex_outgoing_hyperedges,
        sections.head_offsets,
        sections.head_participants,
        BcsrRoleSide::Outgoing,
    )?;
    converse_direction_walk(
        sections.vertex_incoming_offsets,
        sections.vertex_incoming_hyperedges,
        sections.tail_offsets,
        sections.tail_participants,
        BcsrRoleSide::Incoming,
    )
}

/// Walks one side of the bipartite index vertex-by-vertex and confirms every
/// `(v, h)` pair appears in the matching hyperedge-major bucket. This is the
/// converse of [`cross_direction_walk`].
fn converse_direction_walk<OffsetWord, VertexWord, RelationWord>(
    vertex_offsets: &[OffsetWord],
    vertex_values: &[RelationWord],
    edge_offsets: &[OffsetWord],
    edge_values: &[VertexWord],
    side: BcsrRoleSide,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    if vertex_offsets.len() < 2 {
        return Ok(());
    }
    for vertex_index in 0..(vertex_offsets.len() - 1) {
        let start = index_to_usize(vertex_offsets[vertex_index].get())?;
        let end = index_to_usize(vertex_offsets[vertex_index + 1].get())?;
        for word in vertex_values.iter().take(end).skip(start) {
            let hyperedge = index_to_usize(word.get())?;
            let bucket_start = index_to_usize(edge_offsets[hyperedge].get())?;
            let bucket_end = index_to_usize(edge_offsets[hyperedge + 1].get())?;
            if !bucket_contains(edge_values, bucket_start, bucket_end, vertex_index) {
                return Err(BcsrError::CrossDirectionMismatch {
                    side,
                    hyperedge,
                    vertex: vertex_index,
                });
            }
        }
    }
    Ok(())
}

/// Walks one side of the bipartite index hyperedge-by-hyperedge and confirms
/// every `(h, v)` pair appears in the matching vertex-major bucket.
fn cross_direction_walk<OffsetWord, VertexWord, RelationWord>(
    edge_offsets: &[OffsetWord],
    edge_values: &[VertexWord],
    vertex_offsets: &[OffsetWord],
    vertex_values: &[RelationWord],
    side: BcsrRoleSide,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    if edge_offsets.len() < 2 {
        return Ok(());
    }
    for hyperedge_index in 0..(edge_offsets.len() - 1) {
        let start = index_to_usize(edge_offsets[hyperedge_index].get())?;
        let end = index_to_usize(edge_offsets[hyperedge_index + 1].get())?;
        let hyperedge = hyperedge_index;
        cross_direction_check_bucket(CrossDirectionBucket {
            edge_values,
            start,
            end,
            vertex_offsets,
            vertex_values,
            hyperedge,
            side,
        })?;
    }
    Ok(())
}

/// Parameter bundle for [`cross_direction_check_bucket`].
#[derive(Clone, Copy)]
struct CrossDirectionBucket<'a, OffsetWord, VertexWord, RelationWord> {
    /// Hyperedge-major value slice (`head_participants` or `tail_participants`).
    edge_values: &'a [VertexWord],
    /// Inclusive start of the hyperedge's range inside `edge_values`.
    start: usize,
    /// Exclusive end of the hyperedge's range inside `edge_values`.
    end: usize,
    /// Vertex-major offset slice on the matching side.
    vertex_offsets: &'a [OffsetWord],
    /// Vertex-major hyperedge ID slice on the matching side.
    vertex_values: &'a [RelationWord],
    /// Hyperedge ID being checked.
    hyperedge: usize,
    /// Which side of the bipartite index this check covers.
    side: BcsrRoleSide,
}

/// Confirms every vertex in `edge_values[start..end]` lists `hyperedge`
/// in its own vertex-major bucket.
fn cross_direction_check_bucket<OffsetWord, VertexWord, RelationWord>(
    args: CrossDirectionBucket<'_, OffsetWord, VertexWord, RelationWord>,
) -> Result<(), BcsrError>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    for word in args.edge_values.iter().take(args.end).skip(args.start) {
        let vertex = index_to_usize(word.get())?;
        let v_index = vertex;
        let bucket_start = index_to_usize(args.vertex_offsets[v_index].get())?;
        let bucket_end = index_to_usize(args.vertex_offsets[v_index + 1].get())?;
        if !bucket_contains(args.vertex_values, bucket_start, bucket_end, args.hyperedge) {
            return Err(BcsrError::CrossDirectionMismatch {
                side: args.side,
                hyperedge: args.hyperedge,
                vertex,
            });
        }
    }
    Ok(())
}

/// Returns whether `values[start..end]` contains `needle`. Values within the
/// range are required to be strictly ascending, so binary search is correct.
fn bucket_contains<Word: BcsrWord>(
    values: &[Word],
    start: usize,
    end: usize,
    needle: usize,
) -> bool {
    let bucket = &values[start..end];
    bucket
        .binary_search_by(|word| index_to_usize_validated(word.get()).cmp(&needle))
        .is_ok()
}

/// Converts a BCSR index to `usize`, returning a typed error on truncation.
pub(in crate::internal) fn index_to_usize<Index: BcsrIndex>(
    value: Index,
) -> Result<usize, BcsrError> {
    value
        .to_usize()
        .ok_or(BcsrError::UsizeOverflow { value: usize::MAX })
}

/// Converts a previously validated BCSR index to `usize`.
///
/// # Panics
///
/// Panics via `unreachable!()` only if validation has been bypassed. All
/// in-tree callers run after [`validate_sections`], which surfaces truncation
/// as [`BcsrError::UsizeOverflow`] before any `_validated` call.
///
/// # Performance
///
/// This function is `O(1)`.
pub(in crate::internal) fn index_to_usize_validated<Index: BcsrIndex>(value: Index) -> usize {
    oxgraph_layout_util::index_to_usize_validated(value)
        .unwrap_or_else(|| unreachable!("validated bipartite-CSR index must fit usize"))
}

/// Converts a previously validated `usize` slot to a BCSR index.
///
/// # Panics
///
/// Panics via `unreachable!()` only if validation failed to enforce the target
/// index width.
///
/// # Performance
///
/// This function is `O(1)`.
pub(in crate::internal) fn usize_to_index_validated<Index: BcsrIndex>(value: usize) -> Index {
    oxgraph_layout_util::usize_to_index_validated(value)
        .unwrap_or_else(|| unreachable!("validated BCSR slot must fit index"))
}
