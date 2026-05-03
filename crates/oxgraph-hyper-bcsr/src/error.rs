//! Validation and snapshot-binding errors for bipartite-CSR hypergraph views.

use core::fmt;

use oxgraph_snapshot::SectionViewError;

/// Bipartite-CSR validation error.
///
/// Returned by [`BcsrHypergraph::open`](crate::BcsrHypergraph::open) and
/// [`BcsrHypergraph::open_with`](crate::BcsrHypergraph::open_with) when one
/// of the eight section payloads fails validation.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from validation paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BcsrError {
    /// `count + 1` overflowed `usize`, so the offset slice length cannot fit.
    OffsetLengthOverflow {
        /// The count for which `count + 1` overflowed.
        count: u32,
    },
    /// An offset slice has the wrong length.
    OffsetLength {
        /// Which offset section this error came from.
        section: BcsrSection,
        /// Expected length (`count + 1`).
        expected: usize,
        /// Actual length seen.
        actual: usize,
    },
    /// `head_offsets` and `tail_offsets` disagree on `hyperedge_count + 1`.
    HyperedgeOffsetLengthMismatch {
        /// `head_offsets.len()`.
        head_offsets_len: usize,
        /// `tail_offsets.len()`.
        tail_offsets_len: usize,
    },
    /// `vertex_outgoing_offsets` and `vertex_incoming_offsets` disagree on
    /// `vertex_count + 1`.
    VertexOffsetLengthMismatch {
        /// `vertex_outgoing_offsets.len()`.
        outgoing_offsets_len: usize,
        /// `vertex_incoming_offsets.len()`.
        incoming_offsets_len: usize,
    },
    /// The first offset in an offset slice was not zero.
    FirstOffset {
        /// Which offset section this error came from.
        section: BcsrSection,
        /// Actual first offset.
        actual: u32,
    },
    /// Offsets were not monotonically non-decreasing.
    NonMonotonicOffset {
        /// Which offset section this error came from.
        section: BcsrSection,
        /// Offset index where monotonicity failed.
        index: usize,
        /// Previous offset value.
        previous: u32,
        /// Actual offset value at `index`.
        actual: u32,
    },
    /// Final offset does not match the corresponding value slice length.
    FinalOffset {
        /// Which offset section this error came from.
        section: BcsrSection,
        /// Final offset value.
        final_offset: u32,
        /// Length of the value slice this offset references.
        value_len: usize,
    },
    /// A vertex ID was outside `0..vertex_count`.
    VertexOutOfRange {
        /// Which value section the bad ID came from.
        section: BcsrSection,
        /// Position within that section.
        index: usize,
        /// The bad vertex ID.
        vertex: u32,
        /// `vertex_count`.
        vertex_count: u32,
    },
    /// A hyperedge ID was outside `0..hyperedge_count`.
    HyperedgeOutOfRange {
        /// Which value section the bad ID came from.
        section: BcsrSection,
        /// Position within that section.
        index: usize,
        /// The bad hyperedge ID.
        hyperedge: u32,
        /// `hyperedge_count`.
        hyperedge_count: u32,
    },
    /// `head_participants.len()` and `vertex_outgoing_hyperedges.len()`
    /// disagree on the total outgoing-incidence count.
    OutgoingTotalMismatch {
        /// `head_participants.len()` (`P_head`).
        head_participants_len: usize,
        /// `vertex_outgoing_hyperedges.len()` (`P_outgoing`).
        outgoing_hyperedges_len: usize,
    },
    /// `tail_participants.len()` and `vertex_incoming_hyperedges.len()`
    /// disagree on the total incoming-incidence count.
    IncomingTotalMismatch {
        /// `tail_participants.len()` (`P_tail`).
        tail_participants_len: usize,
        /// `vertex_incoming_hyperedges.len()` (`P_incoming`).
        incoming_hyperedges_len: usize,
    },
    /// A range-local sequence (e.g. one hyperedge's head participants, or
    /// one vertex's outgoing hyperedges) was not strictly ascending.
    ///
    /// Bipartite-CSR requires set semantics within each range: vertex IDs
    /// inside a single hyperedge's head/tail and hyperedge IDs inside a
    /// single vertex's outgoing/incoming must be strictly increasing.
    NotStrictlyAscending {
        /// Which value section the bad pair came from.
        section: BcsrSection,
        /// Position of the offending value within the section.
        index: usize,
        /// Previous value at `index - 1`.
        previous: u32,
        /// Actual value at `index`.
        actual: u32,
    },
    /// A `u32` value did not fit in `usize` on this target platform.
    UsizeOverflow {
        /// Value that could not be represented as `usize`.
        value: u32,
    },
    /// `P_head + P_tail` overflowed `u32`, so the incidence ID space cannot
    /// be encoded as a single dense `u32` index.
    TotalIncidenceCountOverflow {
        /// Total head-side incidences (`P_head == P_outgoing`).
        p_head: u32,
        /// Total tail-side incidences (`P_tail == P_incoming`).
        p_tail: u32,
    },
    /// Cross-CSR consistency check (Strict-only) found a hyperedge that is
    /// recorded in one direction but missing from the other.
    CrossDirectionMismatch {
        /// Which side of the bipartite index disagreed.
        side: BcsrRoleSide,
        /// The hyperedge ID that did not match across the two indexes.
        hyperedge: u32,
        /// The vertex ID that did not match across the two indexes.
        vertex: u32,
    },
}

/// Names a single bipartite-CSR section for error reporting.
///
/// Carrying this in error variants avoids stringly-typed reasons while
/// keeping the failing section identifiable from the error alone.
///
/// # Performance
///
/// `perf: unspecified`; this is a metadata enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BcsrSection {
    /// `BCSR_HEAD_OFFSETS`.
    HeadOffsets,
    /// `BCSR_HEAD_PARTICIPANTS`.
    HeadParticipants,
    /// `BCSR_TAIL_OFFSETS`.
    TailOffsets,
    /// `BCSR_TAIL_PARTICIPANTS`.
    TailParticipants,
    /// `BCSR_VERTEX_OUTGOING_OFFSETS`.
    VertexOutgoingOffsets,
    /// `BCSR_VERTEX_OUTGOING_HYPEREDGES`.
    VertexOutgoingHyperedges,
    /// `BCSR_VERTEX_INCOMING_OFFSETS`.
    VertexIncomingOffsets,
    /// `BCSR_VERTEX_INCOMING_HYPEREDGES`.
    VertexIncomingHyperedges,
}

impl fmt::Display for BcsrSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::HeadOffsets => "BCSR_HEAD_OFFSETS",
            Self::HeadParticipants => "BCSR_HEAD_PARTICIPANTS",
            Self::TailOffsets => "BCSR_TAIL_OFFSETS",
            Self::TailParticipants => "BCSR_TAIL_PARTICIPANTS",
            Self::VertexOutgoingOffsets => "BCSR_VERTEX_OUTGOING_OFFSETS",
            Self::VertexOutgoingHyperedges => "BCSR_VERTEX_OUTGOING_HYPEREDGES",
            Self::VertexIncomingOffsets => "BCSR_VERTEX_INCOMING_OFFSETS",
            Self::VertexIncomingHyperedges => "BCSR_VERTEX_INCOMING_HYPEREDGES",
        })
    }
}

/// Side of the bipartite index that produced a cross-direction mismatch.
///
/// # Performance
///
/// `perf: unspecified`; this is a metadata enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BcsrRoleSide {
    /// Hyperedge-major head versus vertex-major outgoing.
    Outgoing,
    /// Hyperedge-major tail versus vertex-major incoming.
    Incoming,
}

impl fmt::Display for BcsrRoleSide {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
        })
    }
}

impl fmt::Display for BcsrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OffsetLengthOverflow { .. }
            | Self::OffsetLength { .. }
            | Self::HyperedgeOffsetLengthMismatch { .. }
            | Self::VertexOffsetLengthMismatch { .. } => fmt_length_variant(self, formatter),
            Self::FirstOffset { .. }
            | Self::NonMonotonicOffset { .. }
            | Self::FinalOffset { .. } => fmt_offset_variant(self, formatter),
            Self::VertexOutOfRange { .. }
            | Self::HyperedgeOutOfRange { .. }
            | Self::NotStrictlyAscending { .. } => fmt_value_variant(self, formatter),
            Self::OutgoingTotalMismatch { .. }
            | Self::IncomingTotalMismatch { .. }
            | Self::UsizeOverflow { .. }
            | Self::TotalIncidenceCountOverflow { .. }
            | Self::CrossDirectionMismatch { .. } => fmt_total_variant(self, formatter),
        }
    }
}

/// Formats the length-shape variants of [`BcsrError`].
fn fmt_length_variant(error: &BcsrError, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match error {
        BcsrError::OffsetLengthOverflow { count } => {
            write!(formatter, "offset length overflow for count {count}")
        }
        BcsrError::OffsetLength {
            section,
            expected,
            actual,
        } => write!(
            formatter,
            "{section} has wrong length: expected {expected}, got {actual}"
        ),
        BcsrError::HyperedgeOffsetLengthMismatch {
            head_offsets_len,
            tail_offsets_len,
        } => write!(
            formatter,
            "head_offsets length {head_offsets_len} disagrees with tail_offsets length {tail_offsets_len}"
        ),
        BcsrError::VertexOffsetLengthMismatch {
            outgoing_offsets_len,
            incoming_offsets_len,
        } => write!(
            formatter,
            "vertex_outgoing_offsets length {outgoing_offsets_len} disagrees with vertex_incoming_offsets length {incoming_offsets_len}"
        ),
        _ => Ok(()),
    }
}

/// Formats the offset-shape variants of [`BcsrError`].
fn fmt_offset_variant(error: &BcsrError, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match error {
        BcsrError::FirstOffset { section, actual } => {
            write!(formatter, "{section} first offset must be 0, got {actual}")
        }
        BcsrError::NonMonotonicOffset {
            section,
            index,
            previous,
            actual,
        } => write!(
            formatter,
            "{section} offset at index {index} is not monotonic: previous {previous}, got {actual}"
        ),
        BcsrError::FinalOffset {
            section,
            final_offset,
            value_len,
        } => write!(
            formatter,
            "{section} final offset {final_offset} does not match value length {value_len}"
        ),
        _ => Ok(()),
    }
}

/// Formats the in-range-value and ascending-order variants of [`BcsrError`].
fn fmt_value_variant(error: &BcsrError, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match error {
        BcsrError::VertexOutOfRange {
            section,
            index,
            vertex,
            vertex_count,
        } => write!(
            formatter,
            "{section} vertex {vertex} at index {index} is out of range (vertex count {vertex_count})"
        ),
        BcsrError::HyperedgeOutOfRange {
            section,
            index,
            hyperedge,
            hyperedge_count,
        } => write!(
            formatter,
            "{section} hyperedge {hyperedge} at index {index} is out of range (hyperedge count {hyperedge_count})"
        ),
        BcsrError::NotStrictlyAscending {
            section,
            index,
            previous,
            actual,
        } => write!(
            formatter,
            "{section} value at index {index} is not strictly ascending: previous {previous}, got {actual}"
        ),
        _ => Ok(()),
    }
}

/// Formats the cross-section total / overflow / consistency variants.
fn fmt_total_variant(error: &BcsrError, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match error {
        BcsrError::OutgoingTotalMismatch {
            head_participants_len,
            outgoing_hyperedges_len,
        } => write!(
            formatter,
            "head_participants length {head_participants_len} disagrees with vertex_outgoing_hyperedges length {outgoing_hyperedges_len}"
        ),
        BcsrError::IncomingTotalMismatch {
            tail_participants_len,
            incoming_hyperedges_len,
        } => write!(
            formatter,
            "tail_participants length {tail_participants_len} disagrees with vertex_incoming_hyperedges length {incoming_hyperedges_len}"
        ),
        BcsrError::UsizeOverflow { value } => {
            write!(formatter, "u32 value {value} does not fit usize")
        }
        BcsrError::TotalIncidenceCountOverflow { p_head, p_tail } => write!(
            formatter,
            "incidence ID space P_head ({p_head}) + P_tail ({p_tail}) overflows u32"
        ),
        BcsrError::CrossDirectionMismatch {
            side,
            hyperedge,
            vertex,
        } => write!(
            formatter,
            "cross-direction mismatch on {side}: hyperedge {hyperedge} and vertex {vertex} disagree"
        ),
        _ => Ok(()),
    }
}

impl core::error::Error for BcsrError {}

/// Error returned when a snapshot cannot be opened as a bipartite-CSR
/// hypergraph view.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from snapshot-bound paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BcsrSnapshotError {
    /// The snapshot is missing one of the eight required sections.
    MissingSection {
        /// Which section was missing.
        section: BcsrSection,
        /// The kind constant the lookup used.
        kind: u32,
    },
    /// A required section payload could not be borrowed as `[U32<LE>]`.
    SectionView {
        /// Which section failed the typed-slice cast.
        section: BcsrSection,
        /// The underlying snapshot error.
        error: SectionViewError,
    },
    /// One of the offset sections was empty; bipartite-CSR requires at least
    /// one entry for the n-plus-one layout.
    OffsetsEmpty {
        /// Which offset section was empty.
        section: BcsrSection,
    },
    /// A derived count would not fit in `u32`.
    CountOverflow {
        /// Which section's length triggered the overflow.
        section: BcsrSection,
        /// Length of the offsets section.
        offsets_len: usize,
    },
    /// Bipartite-CSR validation failed on the borrowed sections.
    Validation(BcsrError),
}

impl fmt::Display for BcsrSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSection { section, kind } => write!(
                formatter,
                "snapshot has no {section} section (kind 0x{kind:04x})"
            ),
            Self::SectionView { section, error } => write!(
                formatter,
                "{section} cannot be borrowed as little-endian u32: {error}"
            ),
            Self::OffsetsEmpty { section } => write!(formatter, "{section} section is empty"),
            Self::CountOverflow {
                section,
                offsets_len,
            } => write!(
                formatter,
                "derived count from {section} length {offsets_len} does not fit u32"
            ),
            Self::Validation(error) => {
                write!(formatter, "bipartite-CSR validation failed: {error}")
            }
        }
    }
}

impl core::error::Error for BcsrSnapshotError {}
