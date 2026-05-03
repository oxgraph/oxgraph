//! Layout- and Strict-tier validation for bipartite-CSR section payloads.
//!
//! Validation walks the eight section payloads at open time and rejects any
//! shape that would let a later traversal step out of bounds, observe stale
//! data, or yield an inconsistent view of the bipartite incidence relation.
//! The depth of the walk is selected by [`BcsrValidation`].

use crate::{
    error::{BcsrError, BcsrRoleSide, BcsrSection},
    sections::BcsrSections,
    word::BcsrWord,
};

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
    pub(in crate::internal) vertex_count: u32,
    /// Number of hyperedges visible in this view.
    pub(in crate::internal) hyperedge_count: u32,
    /// Number of outgoing incidences (`P_head == P_outgoing`).
    pub(in crate::internal) p_outgoing: u32,
    /// Number of incoming incidences (`P_tail == P_incoming`).
    pub(in crate::internal) p_incoming: u32,
    /// Total incidence count (`P_outgoing + P_incoming`). Validation
    /// guarantees this fits in `u32`.
    pub(in crate::internal) total_incidences: u32,
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
pub(in crate::internal) fn validate_sections<Word: BcsrWord>(
    sections: &BcsrSections<'_, Word>,
    level: BcsrValidation,
) -> Result<DerivedCounts, BcsrError> {
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
fn derive_counts<Word: BcsrWord>(
    sections: &BcsrSections<'_, Word>,
) -> Result<DerivedCounts, BcsrError> {
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
    let vertex_count = derive_count_from_offsets(outgoing_len, BcsrSection::VertexOutgoingOffsets)?;
    let p_outgoing = u32_try_from_usize(sections.vertex_outgoing_hyperedges.len())?;
    let p_incoming = u32_try_from_usize(sections.vertex_incoming_hyperedges.len())?;
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

/// Returns `(offsets.len() - 1) as u32` after checking length is non-zero
/// and `len - 1` fits in `u32`.
fn derive_count_from_offsets(offsets_len: usize, section: BcsrSection) -> Result<u32, BcsrError> {
    if offsets_len == 0 {
        return Err(BcsrError::OffsetLength {
            section,
            expected: 1,
            actual: 0,
        });
    }
    u32_try_from_usize(offsets_len - 1)
}

/// Converts a `usize` into a `u32`, returning [`BcsrError::UsizeOverflow`]
/// when the value cannot be represented.
fn u32_try_from_usize(value: usize) -> Result<u32, BcsrError> {
    u32::try_from(value).map_err(|_error| BcsrError::UsizeOverflow {
        // The error variant carries `u32`; report the saturating value when
        // the input does not fit. This branch is only taken on platforms
        // where `usize` is wider than `u32`.
        value: u32::MAX,
    })
}

/// Validates each of the four offset arrays end-to-end.
fn validate_all_offsets<Word: BcsrWord>(
    sections: &BcsrSections<'_, Word>,
    counts: DerivedCounts,
) -> Result<(), BcsrError> {
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
/// monotonic non-decreasing, final offset matches `value_len`.
fn validate_one_offsets<Word: BcsrWord>(
    offsets: &[Word],
    section: BcsrSection,
    count: u32,
    value_len: usize,
) -> Result<(), BcsrError> {
    let expected = u32_to_usize(count)?
        .checked_add(1)
        .ok_or(BcsrError::OffsetLengthOverflow { count })?;
    if offsets.len() != expected {
        return Err(BcsrError::OffsetLength {
            section,
            expected,
            actual: offsets.len(),
        });
    }
    check_offsets_monotonic(offsets, section)?;
    let final_offset = offsets[offsets.len() - 1].get();
    if u32_to_usize(final_offset)? != value_len {
        return Err(BcsrError::FinalOffset {
            section,
            final_offset,
            value_len,
        });
    }
    Ok(())
}

/// Checks the first-zero and monotonic-non-decreasing offset invariants.
fn check_offsets_monotonic<Word: BcsrWord>(
    offsets: &[Word],
    section: BcsrSection,
) -> Result<(), BcsrError> {
    let mut previous = 0;
    for (index, offset_word) in offsets.iter().copied().enumerate() {
        let offset = offset_word.get();
        if index == 0 && offset != 0 {
            return Err(BcsrError::FirstOffset {
                section,
                actual: offset,
            });
        }
        if offset < previous {
            return Err(BcsrError::NonMonotonicOffset {
                section,
                index,
                previous,
                actual: offset,
            });
        }
        previous = offset;
    }
    Ok(())
}

/// Verifies that head/outgoing and tail/incoming totals agree, so the
/// bipartite index has a single shared incidence count per direction.
const fn validate_total_lengths<Word: BcsrWord>(
    sections: &BcsrSections<'_, Word>,
) -> Result<(), BcsrError> {
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
fn validate_value_ranges<Word: BcsrWord>(
    sections: &BcsrSections<'_, Word>,
    counts: DerivedCounts,
) -> Result<(), BcsrError> {
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

/// Returns `Err` if any vertex word is `>= vertex_count`.
fn check_vertex_values<Word: BcsrWord>(
    values: &[Word],
    section: BcsrSection,
    vertex_count: u32,
) -> Result<(), BcsrError> {
    for (index, word) in values.iter().copied().enumerate() {
        let vertex = word.get();
        if vertex >= vertex_count {
            return Err(BcsrError::VertexOutOfRange {
                section,
                index,
                vertex,
                vertex_count,
            });
        }
    }
    Ok(())
}

/// Returns `Err` if any hyperedge word is `>= hyperedge_count`.
fn check_hyperedge_values<Word: BcsrWord>(
    values: &[Word],
    section: BcsrSection,
    hyperedge_count: u32,
) -> Result<(), BcsrError> {
    for (index, word) in values.iter().copied().enumerate() {
        let hyperedge = word.get();
        if hyperedge >= hyperedge_count {
            return Err(BcsrError::HyperedgeOutOfRange {
                section,
                index,
                hyperedge,
                hyperedge_count,
            });
        }
    }
    Ok(())
}

/// Verifies that values within every per-bucket range are strictly
/// ascending. Bipartite-CSR uses set semantics inside each range.
fn validate_within_range_sorted<Word: BcsrWord>(
    sections: &BcsrSections<'_, Word>,
) -> Result<(), BcsrError> {
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
fn check_strictly_ascending_buckets<Word: BcsrWord>(
    offsets: &[Word],
    values: &[Word],
    section: BcsrSection,
) -> Result<(), BcsrError> {
    if offsets.len() < 2 {
        return Ok(());
    }
    for window in offsets.windows(2) {
        let start = u32_to_usize(window[0].get())?;
        let end = u32_to_usize(window[1].get())?;
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
    let mut previous = values[start].get();
    for relative in 1..(end - start) {
        let index = start + relative;
        let actual = values[index].get();
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
fn validate_cross_direction<Word: BcsrWord>(
    sections: &BcsrSections<'_, Word>,
) -> Result<(), BcsrError> {
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
    )
}

/// Walks one side of the bipartite index hyperedge-by-hyperedge and confirms
/// every `(h, v)` pair appears in the matching vertex-major bucket.
fn cross_direction_walk<Word: BcsrWord>(
    edge_offsets: &[Word],
    edge_values: &[Word],
    vertex_offsets: &[Word],
    vertex_values: &[Word],
    side: BcsrRoleSide,
) -> Result<(), BcsrError> {
    if edge_offsets.len() < 2 {
        return Ok(());
    }
    for hyperedge_index in 0..(edge_offsets.len() - 1) {
        let start = u32_to_usize(edge_offsets[hyperedge_index].get())?;
        let end = u32_to_usize(edge_offsets[hyperedge_index + 1].get())?;
        let hyperedge = u32_try_from_usize(hyperedge_index)?;
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
struct CrossDirectionBucket<'a, Word> {
    /// Hyperedge-major value slice (`head_participants` or `tail_participants`).
    edge_values: &'a [Word],
    /// Inclusive start of the hyperedge's range inside `edge_values`.
    start: usize,
    /// Exclusive end of the hyperedge's range inside `edge_values`.
    end: usize,
    /// Vertex-major offset slice on the matching side.
    vertex_offsets: &'a [Word],
    /// Vertex-major hyperedge ID slice on the matching side.
    vertex_values: &'a [Word],
    /// Hyperedge ID being checked.
    hyperedge: u32,
    /// Which side of the bipartite index this check covers.
    side: BcsrRoleSide,
}

/// Confirms every vertex in `edge_values[start..end]` lists `hyperedge`
/// in its own vertex-major bucket.
fn cross_direction_check_bucket<Word: BcsrWord>(
    args: CrossDirectionBucket<'_, Word>,
) -> Result<(), BcsrError> {
    for word in args.edge_values.iter().take(args.end).skip(args.start) {
        let vertex = word.get();
        let v_index = u32_to_usize(vertex)?;
        let bucket_start = u32_to_usize(args.vertex_offsets[v_index].get())?;
        let bucket_end = u32_to_usize(args.vertex_offsets[v_index + 1].get())?;
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
fn bucket_contains<Word: BcsrWord>(values: &[Word], start: usize, end: usize, needle: u32) -> bool {
    let bucket = &values[start..end];
    bucket
        .binary_search_by(|word| word.get().cmp(&needle))
        .is_ok()
}

/// Converts a validated `u32` count to `usize`, returning a typed error on
/// 16-bit `usize` targets where the conversion would truncate.
fn u32_to_usize(value: u32) -> Result<usize, BcsrError> {
    usize::try_from(value).map_err(|_error| BcsrError::UsizeOverflow { value })
}

/// Converts a previously validated `u32` to `usize`. Used inside the view's
/// hot trait paths where validation guarantees the conversion succeeds.
///
/// # Performance
///
/// This function is `O(1)`.
pub(in crate::internal) fn u32_to_usize_validated(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(_error) => unreachable!("validated bipartite-CSR u32 must fit usize"),
    }
}
