//! Concrete error types for snapshot reading, viewing, and writing.
//!
//! Each error type implements [`Display`](core::fmt::Display) and
//! [`core::error::Error`] without depending on `alloc`, `std`, or external
//! error frameworks.

use core::fmt;

/// Snapshot container validation error.
///
/// Returned by [`Snapshot::open`](crate::Snapshot::open) and
/// [`Snapshot::open_with`](crate::Snapshot::open_with) for any header,
/// section table, or layout-level invariant violation.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from validation paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    /// Snapshot bytes were shorter than the fixed header.
    TruncatedHeader {
        /// Bytes required for the fixed header.
        needed: usize,
        /// Bytes actually provided.
        actual: usize,
    },
    /// Header bytes were present but could not be interpreted.
    MalformedHeader,
    /// Magic bytes did not match [`FORMAT_MAGIC`](crate::FORMAT_MAGIC).
    BadMagic {
        /// Actual magic bytes from the snapshot.
        actual: [u8; 8],
    },
    /// Format major version did not equal the supported value.
    FormatMajorMismatch {
        /// Major version recorded in the snapshot.
        actual: u32,
        /// Major version this library supports.
        supported: u32,
    },
    /// Format minor version was newer than this library can read.
    FormatMinorTooNew {
        /// Minor version recorded in the snapshot.
        actual: u32,
        /// Highest minor version this library accepts.
        max_supported: u32,
    },
    /// Header `header_size` field did not match the expected value.
    HeaderSizeMismatch {
        /// `header_size` value recorded in the snapshot.
        actual: u32,
        /// Header size this library expects.
        expected: u32,
    },
    /// Header reserved bytes were not all zero.
    NonZeroHeaderReserved,
    /// `section_count` exceeded the v1 cap.
    SectionCountTooLarge {
        /// Section count recorded in the snapshot.
        count: u32,
        /// Maximum permitted section count.
        max: u32,
    },
    /// Bytes after the header were too short for the declared section table.
    TruncatedSectionTable {
        /// Bytes required for the declared section table.
        needed: usize,
        /// Bytes available after the header.
        actual: usize,
    },
    /// Section table bytes could not be interpreted.
    MalformedSectionTable,
    /// A section entry's `reserved_checksum` bytes were not all zero.
    NonZeroEntryChecksum {
        /// Section kind whose entry violated the invariant.
        kind: u32,
    },
    /// A section entry's trailing reserved bytes were not all zero.
    NonZeroEntryReserved {
        /// Section kind whose entry violated the invariant.
        kind: u32,
    },
    /// A section entry declared an unsupported flags bit.
    UnsupportedFlags {
        /// Section kind whose entry violated the invariant.
        kind: u32,
        /// Flags byte recorded in the entry.
        flags: u8,
    },
    /// A section entry declared an `alignment_log2` larger than permitted.
    AlignmentLog2TooLarge {
        /// Section kind whose entry violated the invariant.
        kind: u32,
        /// Declared `alignment_log2` value.
        alignment_log2: u8,
    },
    /// `offset + length` overflowed `u64` for one entry.
    SectionRangeOverflow {
        /// Section kind whose entry violated the invariant.
        kind: u32,
    },
    /// A section's byte range fell outside the snapshot.
    SectionOutOfBounds {
        /// Section kind whose entry violated the invariant.
        kind: u32,
        /// Declared byte offset.
        offset: u64,
        /// Declared byte length.
        length: u64,
        /// Total snapshot byte length.
        snapshot_len: u64,
    },
    /// Section table entries were not in monotonic non-decreasing offset order
    /// or one entry overlapped its predecessor.
    UnsortedSectionTable {
        /// Index of the entry whose offset violated monotonicity.
        index: usize,
    },
    /// Two section entries shared the same kind.
    DuplicateKind {
        /// Duplicated section kind.
        kind: u32,
    },
    /// A `u64` value could not be represented as `usize` on this target.
    UsizeOverflow {
        /// Value that could not be represented as `usize`.
        value: u64,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { needed, actual } => write!(
                formatter,
                "snapshot header is truncated: needed {needed} bytes, got {actual}"
            ),
            Self::MalformedHeader => formatter.write_str("snapshot header is malformed"),
            Self::BadMagic { actual } => write!(formatter, "bad snapshot magic: {actual:?}"),
            Self::FormatMajorMismatch { actual, supported } => write!(
                formatter,
                "unsupported snapshot format major: snapshot is {actual}, this reader supports {supported}"
            ),
            Self::FormatMinorTooNew {
                actual,
                max_supported,
            } => write!(
                formatter,
                "snapshot format minor {actual} is newer than this reader's maximum {max_supported}"
            ),
            Self::HeaderSizeMismatch { actual, expected } => write!(
                formatter,
                "header_size mismatch: snapshot reports {actual}, this reader expects {expected}"
            ),
            Self::NonZeroHeaderReserved => {
                formatter.write_str("snapshot header reserved bytes are not all zero")
            }
            Self::SectionCountTooLarge { count, max } => {
                write!(formatter, "section count {count} exceeds maximum {max}")
            }
            Self::TruncatedSectionTable { needed, actual } => write!(
                formatter,
                "section table is truncated: needed {needed} bytes, got {actual}"
            ),
            Self::MalformedSectionTable => formatter.write_str("section table bytes are malformed"),
            Self::NonZeroEntryChecksum { kind } => write!(
                formatter,
                "section {kind} entry reserved checksum bytes are not all zero"
            ),
            Self::NonZeroEntryReserved { kind } => write!(
                formatter,
                "section {kind} entry trailing reserved bytes are not all zero"
            ),
            Self::UnsupportedFlags { kind, flags } => write!(
                formatter,
                "section {kind} entry has unsupported flags byte {flags:#04x}"
            ),
            Self::AlignmentLog2TooLarge {
                kind,
                alignment_log2,
            } => write!(
                formatter,
                "section {kind} alignment_log2 {alignment_log2} exceeds maximum"
            ),
            Self::SectionRangeOverflow { kind } => {
                write!(formatter, "section {kind} offset + length overflows u64")
            }
            Self::SectionOutOfBounds {
                kind,
                offset,
                length,
                snapshot_len,
            } => write!(
                formatter,
                "section {kind} is out of bounds: offset {offset}, length {length}, snapshot length {snapshot_len}"
            ),
            Self::UnsortedSectionTable { index } => write!(
                formatter,
                "section table entry at index {index} is unsorted or overlaps its predecessor"
            ),
            Self::DuplicateKind { kind } => write!(formatter, "duplicate section kind {kind}"),
            Self::UsizeOverflow { value } => {
                write!(formatter, "u64 value {value} does not fit usize")
            }
        }
    }
}

impl core::error::Error for SnapshotError {}

/// Error returned when borrowing a section payload as a typed slice fails.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from typed-view paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectionViewError {
    /// The requested element type is zero-sized, so it cannot tile a payload.
    ZeroSizedType,
    /// Payload byte length is not an exact multiple of `size_of::<T>()`.
    LengthNotMultipleOfSize {
        /// Section payload length in bytes.
        length: usize,
        /// `core::mem::size_of::<T>()` for the requested element type.
        elem_size: usize,
    },
    /// Payload base address does not satisfy `align_of::<T>()`.
    AlignmentMismatch {
        /// Address of the payload's first byte.
        ptr_addr: usize,
        /// `core::mem::align_of::<T>()` for the requested element type.
        required: usize,
    },
}

impl fmt::Display for SectionViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSizedType => {
                formatter.write_str("cannot borrow a section payload as a zero-sized type")
            }
            Self::LengthNotMultipleOfSize { length, elem_size } => write!(
                formatter,
                "section length {length} is not a multiple of element size {elem_size}"
            ),
            Self::AlignmentMismatch { ptr_addr, required } => write!(
                formatter,
                "section payload at address {ptr_addr:#x} is not aligned to {required}"
            ),
        }
    }
}

impl core::error::Error for SectionViewError {}

/// Error returned when binding a width-typed section by kind and version.
///
/// Returned by [`Snapshot::typed_section`](crate::Snapshot::typed_section): a
/// single error covering the lookup, version check, and typed-view steps that
/// every layout crate previously open-coded with its own variants.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from section-binding paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectionBindError {
    /// No section with the requested kind was present.
    Missing {
        /// Requested section kind.
        kind: u32,
    },
    /// The section was present but its version did not match.
    VersionMismatch {
        /// Requested section kind.
        kind: u32,
        /// Version the caller required.
        expected: u32,
        /// Version recorded in the section entry.
        actual: u32,
    },
    /// The payload could not be borrowed as the requested little-endian word.
    View {
        /// Requested section kind.
        kind: u32,
        /// Underlying typed-view failure.
        error: SectionViewError,
    },
}

impl fmt::Display for SectionBindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { kind } => write!(formatter, "snapshot section {kind} is missing"),
            Self::VersionMismatch {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "snapshot section {kind} version {actual} does not match expected {expected}"
            ),
            Self::View { kind, error } => {
                write!(
                    formatter,
                    "snapshot section {kind} typed view failed: {error}"
                )
            }
        }
    }
}

impl core::error::Error for SectionBindError {}

/// Error returned by snapshot writers.
///
/// Returned by [`SnapshotPlan::new`](crate::SnapshotPlan::new),
/// [`SnapshotPlan::write_into`](crate::SnapshotPlan::write_into), and the
/// alloc-gated [`SnapshotBuilder`](crate::SnapshotBuilder) methods.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from writer paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    /// The output buffer is smaller than the snapshot's encoded length.
    BufferTooSmall {
        /// Bytes required by the encoded snapshot.
        needed: usize,
        /// Bytes provided in the output buffer.
        actual: usize,
    },
    /// A section's `alignment_log2` exceeds [`MAX_ALIGNMENT_LOG2`](crate::MAX_ALIGNMENT_LOG2).
    AlignmentTooLarge {
        /// Declared `alignment_log2` value.
        alignment_log2: u8,
    },
    /// Computed offsets or lengths overflowed `u64` or `usize`.
    PayloadOverflow,
    /// More sections were supplied than the v1 cap permits.
    TooManySections {
        /// Section count actually supplied.
        count: usize,
    },
    /// Two pending sections shared the same kind.
    DuplicateKind {
        /// Duplicated section kind.
        kind: u32,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooSmall { needed, actual } => write!(
                formatter,
                "output buffer too small: needed {needed} bytes, got {actual}"
            ),
            Self::AlignmentTooLarge { alignment_log2 } => write!(
                formatter,
                "alignment_log2 {alignment_log2} exceeds the v1 maximum"
            ),
            Self::PayloadOverflow => {
                formatter.write_str("snapshot payload arithmetic overflowed u64 or usize")
            }
            Self::TooManySections { count } => {
                write!(formatter, "section count {count} exceeds the v1 maximum")
            }
            Self::DuplicateKind { kind } => write!(formatter, "duplicate section kind {kind}"),
        }
    }
}

impl core::error::Error for PlanError {}
