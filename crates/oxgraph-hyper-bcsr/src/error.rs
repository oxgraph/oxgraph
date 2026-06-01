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
        count: usize,
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
        actual: usize,
    },
    /// Offsets were not monotonically non-decreasing.
    NonMonotonicOffset {
        /// Which offset section this error came from.
        section: BcsrSection,
        /// Offset index where monotonicity failed.
        index: usize,
        /// Previous offset value.
        previous: usize,
        /// Actual offset value at `index`.
        actual: usize,
    },
    /// Final offset does not match the corresponding value slice length.
    FinalOffset {
        /// Which offset section this error came from.
        section: BcsrSection,
        /// Final offset value.
        final_offset: usize,
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
        vertex: usize,
        /// `vertex_count`.
        vertex_count: usize,
    },
    /// A hyperedge ID was outside `0..hyperedge_count`.
    HyperedgeOutOfRange {
        /// Which value section the bad ID came from.
        section: BcsrSection,
        /// Position within that section.
        index: usize,
        /// The bad hyperedge ID.
        hyperedge: usize,
        /// `hyperedge_count`.
        hyperedge_count: usize,
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
        previous: usize,
        /// Actual value at `index`.
        actual: usize,
    },
    /// A stored index value did not fit in `usize` on this target platform.
    UsizeOverflow {
        /// Value that could not be represented as `usize`.
        value: usize,
    },
    /// `P_head + P_tail` overflowed `usize`, so the incidence ID space cannot
    /// be indexed on this target.
    TotalIncidenceCountOverflow {
        /// Total head-side incidences (`P_head == P_outgoing`).
        p_head: usize,
        /// Total tail-side incidences (`P_tail == P_incoming`).
        p_tail: usize,
    },
    /// Cross-CSR consistency check (Strict-only) found a hyperedge that is
    /// recorded in one direction but missing from the other.
    CrossDirectionMismatch {
        /// Which side of the bipartite index disagreed.
        side: BcsrRoleSide,
        /// The hyperedge ID that did not match across the two indexes.
        hyperedge: usize,
        /// The vertex ID that did not match across the two indexes.
        vertex: usize,
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
    #[expect(
        clippy::too_many_lines,
        reason = "one flat exhaustive Display match over every BcsrError variant; splitting it reintroduces the silent _ => Ok(()) wildcards this consolidation removed"
    )]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // One flat, exhaustive match: a new variant forces a Display arm at
        // compile time, and there is no `_ => Ok(())` wildcard that could
        // silently format nothing for a mis-routed variant.
        match self {
            Self::OffsetLengthOverflow { count } => {
                write!(formatter, "offset length overflow for count {count}")
            }
            Self::OffsetLength {
                section,
                expected,
                actual,
            } => write!(
                formatter,
                "{section} has wrong length: expected {expected}, got {actual}"
            ),
            Self::HyperedgeOffsetLengthMismatch {
                head_offsets_len,
                tail_offsets_len,
            } => write!(
                formatter,
                "head_offsets length {head_offsets_len} disagrees with tail_offsets length {tail_offsets_len}"
            ),
            Self::VertexOffsetLengthMismatch {
                outgoing_offsets_len,
                incoming_offsets_len,
            } => write!(
                formatter,
                "vertex_outgoing_offsets length {outgoing_offsets_len} disagrees with vertex_incoming_offsets length {incoming_offsets_len}"
            ),
            Self::FirstOffset { section, actual } => {
                write!(formatter, "{section} first offset must be 0, got {actual}")
            }
            Self::NonMonotonicOffset {
                section,
                index,
                previous,
                actual,
            } => write!(
                formatter,
                "{section} offset at index {index} is not monotonic: previous {previous}, got {actual}"
            ),
            Self::FinalOffset {
                section,
                final_offset,
                value_len,
            } => write!(
                formatter,
                "{section} final offset {final_offset} does not match value length {value_len}"
            ),
            Self::VertexOutOfRange {
                section,
                index,
                vertex,
                vertex_count,
            } => write!(
                formatter,
                "{section} vertex {vertex} at index {index} is out of range (vertex count {vertex_count})"
            ),
            Self::HyperedgeOutOfRange {
                section,
                index,
                hyperedge,
                hyperedge_count,
            } => write!(
                formatter,
                "{section} hyperedge {hyperedge} at index {index} is out of range (hyperedge count {hyperedge_count})"
            ),
            Self::NotStrictlyAscending {
                section,
                index,
                previous,
                actual,
            } => write!(
                formatter,
                "{section} value at index {index} is not strictly ascending: previous {previous}, got {actual}"
            ),
            Self::OutgoingTotalMismatch {
                head_participants_len,
                outgoing_hyperedges_len,
            } => write!(
                formatter,
                "head_participants length {head_participants_len} disagrees with vertex_outgoing_hyperedges length {outgoing_hyperedges_len}"
            ),
            Self::IncomingTotalMismatch {
                tail_participants_len,
                incoming_hyperedges_len,
            } => write!(
                formatter,
                "tail_participants length {tail_participants_len} disagrees with vertex_incoming_hyperedges length {incoming_hyperedges_len}"
            ),
            Self::UsizeOverflow { value } => {
                write!(formatter, "BCSR index value {value} does not fit usize")
            }
            Self::TotalIncidenceCountOverflow { p_head, p_tail } => write!(
                formatter,
                "incidence ID space P_head ({p_head}) + P_tail ({p_tail}) overflows usize"
            ),
            Self::CrossDirectionMismatch {
                side,
                hyperedge,
                vertex,
            } => write!(
                formatter,
                "cross-direction mismatch on {side}: hyperedge {hyperedge} and vertex {vertex} disagree"
            ),
        }
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
    /// A required section was present but its version did not match.
    VersionMismatch {
        /// Which section had the wrong version.
        section: BcsrSection,
        /// The kind constant the lookup used.
        kind: u32,
        /// Version the reader required.
        expected: u32,
        /// Version recorded in the snapshot.
        actual: u32,
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
    /// Bipartite-CSR layout-shape error surfaced through the borrowed sections.
    Bcsr(BcsrError),
}

impl fmt::Display for BcsrSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSection { section, kind } => write!(
                formatter,
                "snapshot has no {section} section (kind 0x{kind:04x})"
            ),
            Self::VersionMismatch {
                section,
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "{section} section (kind 0x{kind:04x}) version {actual} does not match expected {expected}"
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
            Self::Bcsr(error) => {
                write!(formatter, "bipartite-CSR validation failed: {error}")
            }
        }
    }
}

impl core::error::Error for BcsrSnapshotError {}

impl From<BcsrError> for BcsrSnapshotError {
    fn from(error: BcsrError) -> Self {
        Self::Bcsr(error)
    }
}
