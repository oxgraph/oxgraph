//! Topology-agnostic snapshot container: format constants, byte-level header
//! and section table, validation, reader, no-`alloc` planner, and the
//! `alloc`-gated owning builder.
//!
//! All public types are re-exported through the crate root; consumers should
//! depend on the crate-level paths rather than reaching in here.
//!
//! When the container ever graduates to a separate `topology-snapshot` crate
//! the whole module moves wholesale, and the crate root becomes a shim of
//! `pub use topology_snapshot::*`.

#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};
use core::fmt;

use oxgraph_layout_util::SnapshotWidth;
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U32, U64},
};

use crate::container_error::{PlanError, SectionBindError, SectionViewError, SnapshotError};

/// Magic bytes identifying the topology snapshot container format.
///
/// Producers MUST write these eight bytes at offset 0; readers MUST reject
/// snapshots whose first eight bytes differ.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const FORMAT_MAGIC: [u8; 8] = *b"OXGTOPO\0";

/// Format major version this library reads and writes.
///
/// A snapshot whose `format_major` field does not equal this constant is
/// rejected at open time. Major bumps are permitted to break compatibility
/// in arbitrary ways.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const FORMAT_MAJOR: u32 = 1;

/// Format minor version written by this library's builder.
///
/// Minor bumps are reserved for backward-compatible additions (e.g. enabling
/// previously reserved bits or fields). Producers using this library will
/// emit this value unconditionally.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const FORMAT_MINOR: u32 = 0;

/// Highest format minor version this library can read.
///
/// Snapshots with `format_minor > MAX_SUPPORTED_MINOR` are rejected at open
/// time. v1 is intentionally strict; raising this value is a deliberate
/// per-minor decision once the new minor is proven safely readable here.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const MAX_SUPPORTED_MINOR: u32 = 0;

/// Size of the snapshot header in bytes.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const HEADER_SIZE: usize = 32;

/// Size of one section table entry in bytes.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SECTION_ENTRY_SIZE: usize = 32;

/// Maximum permitted `alignment_log2` value (2^12 = 4 KiB, page-friendly).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const MAX_ALIGNMENT_LOG2: u8 = 12;

/// Maximum permitted section count for v1 snapshots.
///
/// Bounds the duplicate-kind detection in `O(s^2)` validation and keeps
/// kani proofs tractable. Future minors may raise this if validation moves
/// to a sorted-by-kind side index.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const MAX_SECTION_COUNT: u32 = 1024;

/// `HEADER_SIZE` rendered as a `u32` for header-field comparisons.
const HEADER_SIZE_U32: u32 = 32;

/// Typed wrapper over a section's opaque `u32` kind tag.
///
/// The container still treats the value opaquely, but [`SectionKind`] plus the
/// [`kinds`] band registry give the wire format a single documented authority
/// over the kind namespace. Layout crates declare their kind constants inside
/// the band the registry reserves for them so that distinct subsystems cannot
/// silently collide on a value.
///
/// # Performance
///
/// All methods are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SectionKind(u32);

impl SectionKind {
    /// Wraps a raw kind tag.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw kind tag.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SectionKind {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl fmt::Display for SectionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:#06x}", self.0)
    }
}

/// Section-kind band allocation registry: the single documented authority for
/// who owns which range of the opaque `u32` kind namespace.
///
/// The container assigns no semantics to kinds, but every in-tree layer
/// declares its `SNAPSHOT_KIND_*` constants inside the band reserved here, and
/// the bands are mutually exclusive, so distinct subsystems cannot collide.
/// Each band is a half-open `[start, end)` range of raw kind values.
///
/// # Performance
///
/// `perf: unspecified`; these are compile-time constants.
pub mod kinds {
    use core::ops::Range;

    /// CSR graph layout sections (offsets/targets, all widths).
    pub const CSR_BAND: Range<u32> = 0x0001..0x0020;
    /// Bipartite-CSR hypergraph layout sections (all widths).
    pub const BCSR_BAND: Range<u32> = 0x0020..0x0100;
    /// Property and identity-map sections (all widths).
    pub const PROPERTY_BAND: Range<u32> = 0x0100..0x0200;
    /// `PostgreSQL` engine sections, including the inbound CSC layout.
    pub const POSTGRES_BAND: Range<u32> = 0x0200..0x0300;
    /// Embedded `OxGraph` database state sections.
    pub const DATABASE_BAND: Range<u32> = 0x0300..0x0400;
    /// Application/custom sections; the container reserves nothing here.
    pub const CUSTOM_BASE: u32 = 0x0400;

    /// Returns whether `kind` falls within the half-open `band`.
    ///
    /// Layout crates use this in `const`-checked tests to prove their kind
    /// constants stay inside their reserved band.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn in_band(kind: u32, band: Range<u32>) -> bool {
        kind >= band.start && kind < band.end
    }
}

/// Converts a checked `u64` into `usize`, asserting in debug mode that the
/// value already fits because validation enforced an earlier bound.
///
/// # Panics
///
/// Panics via `unreachable!()` only on a target where `usize` is narrower
/// than `u64` AND the caller has supplied a value that was not first vetted
/// by the snapshot's `Layout` validation pass (which surfaces the failure
/// as [`SnapshotError::UsizeOverflow`] before any `_validated` call).
///
/// # Performance
///
/// This function is `O(1)`.
fn u64_to_usize_validated(value: u64) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(_error) => unreachable!("validated u64 must fit usize on this target"),
    }
}

/// Byte-level snapshot header.
///
/// Layout is `#[repr(C)]` with all multi-byte fields stored as zerocopy's
/// unaligned little-endian wrappers. The struct itself has alignment 1, so
/// it can be borrowed from any byte slice that is at least `HEADER_SIZE`
/// long without an alignment check.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
struct RawHeader {
    /// Magic bytes; must equal [`FORMAT_MAGIC`].
    magic: [u8; 8],
    /// Format major version.
    format_major: U32<LE>,
    /// Format minor version.
    format_minor: U32<LE>,
    /// Header size in bytes; v1.0 mandates `HEADER_SIZE`.
    header_size: U32<LE>,
    /// Number of section table entries.
    section_count: U32<LE>,
    /// Reserved; must be zero.
    reserved: [u8; 8],
}

/// Parses the fixed header from the start of `bytes`.
///
/// # Errors
///
/// Returns [`SnapshotError::TruncatedHeader`] when fewer than [`HEADER_SIZE`]
/// bytes are provided. Header field validation is performed separately in
/// [`validate_magic_versions_reserved`].
///
/// # Performance
///
/// This function is `O(1)`.
fn parse_header(bytes: &[u8]) -> Result<(&RawHeader, &[u8]), SnapshotError> {
    if bytes.len() < HEADER_SIZE {
        return Err(SnapshotError::TruncatedHeader {
            needed: HEADER_SIZE,
            actual: bytes.len(),
        });
    }

    match RawHeader::ref_from_prefix(bytes) {
        Ok((header, rest)) => Ok((header, rest)),
        Err(_error) => Err(SnapshotError::MalformedHeader),
    }
}

/// Validates header magic, version, header size, and reserved bytes.
///
/// # Errors
///
/// Returns [`SnapshotError`] for any header-level invariant violation.
///
/// # Performance
///
/// This function is `O(1)`.
fn validate_magic_versions_reserved(header: &RawHeader) -> Result<(), SnapshotError> {
    if header.magic != FORMAT_MAGIC {
        return Err(SnapshotError::BadMagic {
            actual: header.magic,
        });
    }

    let major = header.format_major.get();
    if major != FORMAT_MAJOR {
        return Err(SnapshotError::FormatMajorMismatch {
            actual: major,
            supported: FORMAT_MAJOR,
        });
    }

    let minor = header.format_minor.get();
    if minor > MAX_SUPPORTED_MINOR {
        return Err(SnapshotError::FormatMinorTooNew {
            actual: minor,
            max_supported: MAX_SUPPORTED_MINOR,
        });
    }

    let header_size = header.header_size.get();
    if header_size != HEADER_SIZE_U32 {
        return Err(SnapshotError::HeaderSizeMismatch {
            actual: header_size,
            expected: HEADER_SIZE_U32,
        });
    }

    if header.reserved != [0; 8] {
        return Err(SnapshotError::NonZeroHeaderReserved);
    }

    Ok(())
}

/// Byte-level section table entry.
///
/// Layout is `#[repr(C)]` with unaligned little-endian fields, mirroring
/// [`RawHeader`]'s alignment policy.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
struct RawSectionEntry {
    /// Byte offset of the section payload from the start of the snapshot.
    offset: U64<LE>,
    /// Byte length of the section payload.
    length: U64<LE>,
    /// Opaque section kind; the container assigns no semantics.
    kind: U32<LE>,
    /// Opaque section version; consumers interpret per kind.
    version: U32<LE>,
    /// Reserved bytes for a future per-section CRC32C; must be zero in v1.
    reserved_checksum: [u8; 4],
    /// `log2` of the producer's chosen payload alignment; v1 cap is 12.
    alignment_log2: u8,
    /// Reserved flag bits; must be zero in v1.
    flags: u8,
    /// Trailing reserved bytes; must be zero in v1.
    reserved: [u8; 2],
}

/// Borrowed view of one validated section in a snapshot.
///
/// A `Section` carries the section's byte payload along with its declared
/// metadata. Payload bytes are bounds- and overlap-checked at snapshot open
/// time. Typed-slice access via [`Section::try_as_slice`] verifies the
/// actual borrowed pointer's alignment at the call site.
///
/// # Performance
///
/// All methods are `O(1)` or `O(payload.len())` for typed conversions.
#[derive(Clone, Copy, Debug)]
pub struct Section<'view> {
    /// Borrowed payload bytes.
    payload: &'view [u8],
    /// Section kind, as recorded in the section entry.
    kind: u32,
    /// Section version, as recorded in the section entry.
    version: u32,
    /// `log2` of the declared payload alignment.
    alignment_log2: u8,
}

impl<'view> Section<'view> {
    /// Constructs a [`Section`] from a previously validated entry.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    fn from_entry(bytes: &'view [u8], entry: &RawSectionEntry) -> Self {
        let offset = u64_to_usize_validated(entry.offset.get());
        let length = u64_to_usize_validated(entry.length.get());
        Self {
            payload: &bytes[offset..offset + length],
            kind: entry.kind.get(),
            version: entry.version.get(),
            alignment_log2: entry.alignment_log2,
        }
    }

    /// Returns the section's opaque kind identifier.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn kind(&self) -> u32 {
        self.kind
    }

    /// Returns the section's opaque version identifier.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Returns the alignment the producer declared for this payload.
    ///
    /// This is metadata recorded at build time, not a guarantee about the
    /// actual borrowed pointer. Callers that intend to interpret the payload
    /// as a typed slice should prefer [`Section::try_as_slice`], which
    /// checks the actual payload pointer.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn declared_alignment(&self) -> usize {
        1usize << self.alignment_log2
    }

    /// Returns the section's borrowed payload bytes.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn bytes(&self) -> &'view [u8] {
        self.payload
    }

    /// Borrows the payload as a typed slice of `T`.
    ///
    /// Errors if (a) `payload.len()` is not a multiple of
    /// `core::mem::size_of::<T>()` or (b) the payload's actual base address
    /// does not satisfy `core::mem::align_of::<T>()`. The producer's
    /// declared `alignment_log2` is not consulted; the actual borrowed
    /// pointer is checked directly so that mmap'd or sub-sliced inputs
    /// cannot bypass the check.
    ///
    /// # Errors
    ///
    /// Returns [`SectionViewError`] when the payload cannot be borrowed
    /// as `&[T]` without copying.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` modulo the bounds and alignment checks; it
    /// performs no allocation and no per-element work.
    pub fn try_as_slice<T>(&self) -> Result<&'view [T], SectionViewError>
    where
        T: zerocopy::FromBytes + zerocopy::Immutable + zerocopy::KnownLayout,
    {
        let elem_size = core::mem::size_of::<T>();
        let length = self.payload.len();

        if elem_size == 0 {
            return Err(SectionViewError::ZeroSizedType);
        }

        if !length.is_multiple_of(elem_size) {
            return Err(SectionViewError::LengthNotMultipleOfSize { length, elem_size });
        }

        let required = core::mem::align_of::<T>();
        let ptr_addr = self.payload.as_ptr().addr();
        if !ptr_addr.is_multiple_of(required) {
            return Err(SectionViewError::AlignmentMismatch { ptr_addr, required });
        }

        let count = length / elem_size;
        match <[T]>::ref_from_bytes_with_elems(self.payload, count) {
            Ok(slice) => Ok(slice),
            Err(_error) => Err(SectionViewError::AlignmentMismatch { ptr_addr, required }),
        }
    }
}

/// Validation depth applied at snapshot open time.
///
/// Validation responsibilities are layered. Header-only validation is not a
/// member of this enum; callers wanting it should use
/// [`HeaderOnlySnapshot::open`] instead, so the type system distinguishes a
/// section-bearing handle from one whose section table has not been
/// validated.
///
/// - [`SectionTable`](Self::SectionTable) parses the section table and per-entry self-consistency
///   (alignment bound, reserved bytes zero, flags zero).
/// - [`Layout`](Self::Layout) is the default; it adds payload bounds, monotonic-offset enforcement,
///   and duplicate-kind detection.
///
/// Topology-level validation (CSR offset monotonicity, hypergraph role
/// consistency, etc.) is the consumer's responsibility — the container
/// has no kind registry and cannot validate semantics it does not know.
///
/// # Performance
///
/// `perf: unspecified`; this is a metadata enum.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationLevel {
    /// Validate header and section table self-consistency.
    SectionTable,
    /// Validate header, section table, and full payload layout.
    Layout,
}

/// Returns the first `kind` value that appears more than once in `items`.
///
/// Shared by the reader's duplicate-kind walk and [`SnapshotPlan::new`] so the
/// two paths cannot drift. `kind_of` projects each item to its kind tag.
///
/// # Performance
///
/// This function is `O(items.len()^2)`.
fn first_duplicate_kind<T>(items: &[T], kind_of: impl Fn(&T) -> u32) -> Option<u32> {
    for (index, item) in items.iter().enumerate() {
        let kind = kind_of(item);
        if items[..index].iter().any(|prior| kind_of(prior) == kind) {
            return Some(kind);
        }
    }
    None
}

/// Walks the section table once and checks all v1 invariants.
///
/// `bytes` is the entire snapshot byte slice; `entries` is the parsed
/// section table; `level` controls how deep the walk goes. Header-level
/// invariants are presumed already validated by the caller.
///
/// Per-entry self-consistency **and** payload bounds (`offset + length` does
/// not overflow and stays within the snapshot) are enforced at every level, so
/// every [`Section`] a [`Snapshot`] hands out is bounds-safe regardless of the
/// requested [`ValidationLevel`]. [`ValidationLevel::Layout`] additionally
/// enforces non-overlapping monotonic ordering and unique kinds.
///
/// # Errors
///
/// Returns [`SnapshotError`] for any per-entry, bounds, or layout violation.
///
/// # Performance
///
/// This function is `O(s)` for the always-run self-consistency and bounds walk
/// and `O(s^2)` for the duplicate-kind walk at [`ValidationLevel::Layout`].
fn validate_section_table(
    bytes: &[u8],
    entries: &[RawSectionEntry],
    level: ValidationLevel,
) -> Result<(), SnapshotError> {
    let snapshot_len = bytes.len() as u64;

    // Always-run: per-entry self-consistency plus payload-bounds safety. The
    // bounds check guarantees `Section::from_entry`'s `bytes[offset..end]`
    // slice is in range, so accessors are panic-free at SectionTable level too.
    for entry in entries {
        let kind = entry.kind.get();
        if entry.reserved_checksum != [0; 4] {
            return Err(SnapshotError::NonZeroEntryChecksum { kind });
        }
        if entry.flags != 0 {
            return Err(SnapshotError::UnsupportedFlags {
                kind,
                flags: entry.flags,
            });
        }
        if entry.reserved != [0; 2] {
            return Err(SnapshotError::NonZeroEntryReserved { kind });
        }
        if entry.alignment_log2 > MAX_ALIGNMENT_LOG2 {
            return Err(SnapshotError::AlignmentLog2TooLarge {
                kind,
                alignment_log2: entry.alignment_log2,
            });
        }
        let offset = entry.offset.get();
        let length = entry.length.get();
        let end = offset
            .checked_add(length)
            .ok_or(SnapshotError::SectionRangeOverflow { kind })?;
        if end > snapshot_len {
            return Err(SnapshotError::SectionOutOfBounds {
                kind,
                offset,
                length,
                snapshot_len,
            });
        }
    }

    if matches!(level, ValidationLevel::SectionTable) {
        return Ok(());
    }

    // Layout-only: non-overlapping monotonic ordering (sections start at or
    // after the end of the header+table and never overlap a predecessor).
    let header_plus_table = (HEADER_SIZE as u64)
        .checked_add((entries.len() as u64).saturating_mul(SECTION_ENTRY_SIZE as u64))
        .ok_or(SnapshotError::SectionRangeOverflow { kind: 0 })?;
    let mut prev_end = header_plus_table;
    for (index, entry) in entries.iter().enumerate() {
        let offset = entry.offset.get();
        // `end` cannot overflow: the always-run walk above already proved it.
        let end = offset.saturating_add(entry.length.get());
        if offset < prev_end {
            return Err(SnapshotError::UnsortedSectionTable { index });
        }
        prev_end = end;
    }

    // Layout-only: duplicate-kind detection, shared with the writer.
    if let Some(kind) = first_duplicate_kind(entries, |entry| entry.kind.get()) {
        return Err(SnapshotError::DuplicateKind { kind });
    }

    Ok(())
}

/// Header-only handle to a snapshot's bytes.
///
/// `HeaderOnlySnapshot` is the typestate-distinct counterpart to
/// [`Snapshot`]: it validates only the fixed header (magic, format
/// versions, header size, reserved bytes) and exposes the format
/// versions, but it deliberately does not parse or expose the section
/// table. Callers who only need to inspect format compatibility (e.g.,
/// to decide whether the snapshot is readable at all) should use this
/// type rather than asking [`Snapshot`] to skip section validation.
///
/// # Performance
///
/// [`HeaderOnlySnapshot::open`] is `O(1)` — it does not walk the section
/// table or payload region. Subsequent accessors are `O(1)`.
#[derive(Clone, Copy, Debug)]
pub struct HeaderOnlySnapshot<'view> {
    /// Borrowed snapshot bytes.
    bytes: &'view [u8],
    /// Format major version recorded in the header.
    format_major: u32,
    /// Format minor version recorded in the header.
    format_minor: u32,
}

impl<'view> HeaderOnlySnapshot<'view> {
    /// Opens `bytes` as a header-validated snapshot handle.
    ///
    /// Validates the magic bytes, format major and minor, header size, and
    /// reserved bytes only. The section table and payload region are not
    /// inspected and may still be malformed.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] for any header-level invariant violation.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn open(bytes: &'view [u8]) -> Result<Self, SnapshotError> {
        let (header, _after_header) = parse_header(bytes)?;
        validate_magic_versions_reserved(header)?;
        Ok(Self {
            bytes,
            format_major: header.format_major.get(),
            format_minor: header.format_minor.get(),
        })
    }

    /// Returns the borrowed snapshot bytes.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn bytes(&self) -> &'view [u8] {
        self.bytes
    }

    /// Returns the format major version recorded in the snapshot header.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn format_major(&self) -> u32 {
        self.format_major
    }

    /// Returns the format minor version recorded in the snapshot header.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn format_minor(&self) -> u32 {
        self.format_minor
    }
}

/// Validated, borrowed handle to a snapshot's bytes and section table.
///
/// A `Snapshot` is constructed via [`Snapshot::open`] (default
/// [`ValidationLevel::Layout`]) or [`Snapshot::open_with`]. The handle
/// itself is `Copy` and trivially cheap to pass; cloning it does not
/// re-validate.
///
/// For header-only inspection without parsing the section table, use
/// [`HeaderOnlySnapshot`] instead — `Snapshot` always carries a validated
/// section table.
///
/// # Performance
///
/// Open is `O(s^2)` for `s` sections at [`ValidationLevel::Layout`] due to
/// duplicate-kind detection, otherwise `O(s)`. Subsequent reads are `O(1)`
/// to `O(s)` per call. No allocation occurs.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot<'view> {
    /// Borrowed snapshot bytes.
    bytes: &'view [u8],
    /// Format major version recorded in the header.
    format_major: u32,
    /// Format minor version recorded in the header.
    format_minor: u32,
    /// Borrowed, validated section table entries.
    entries: &'view [RawSectionEntry],
}

impl<'view> Snapshot<'view> {
    /// Opens `bytes` as a validated snapshot at [`ValidationLevel::Layout`].
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] for any header, section table, or layout
    /// invariant violation.
    ///
    /// # Performance
    ///
    /// `O(s^2)` for `s` section entries.
    pub fn open(bytes: &'view [u8]) -> Result<Self, SnapshotError> {
        Self::open_with(bytes, ValidationLevel::Layout)
    }

    /// Opens `bytes` as a snapshot validated at the requested level.
    ///
    /// `level` selects between [`ValidationLevel::SectionTable`] (per-entry
    /// self-consistency only) and [`ValidationLevel::Layout`] (full
    /// payload-bounds and duplicate-kind walk). Header-only validation is
    /// deliberately not selectable here; callers wanting it should use
    /// [`HeaderOnlySnapshot::open`].
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] for any invariant violation visible at
    /// the requested level.
    ///
    /// # Performance
    ///
    /// `O(s)` at [`ValidationLevel::SectionTable`], `O(s^2)` at
    /// [`ValidationLevel::Layout`].
    pub fn open_with(bytes: &'view [u8], level: ValidationLevel) -> Result<Self, SnapshotError> {
        let (header, after_header) = parse_header(bytes)?;
        validate_magic_versions_reserved(header)?;

        let format_major = header.format_major.get();
        let format_minor = header.format_minor.get();

        let section_count = header.section_count.get();
        if section_count > MAX_SECTION_COUNT {
            return Err(SnapshotError::SectionCountTooLarge {
                count: section_count,
                max: MAX_SECTION_COUNT,
            });
        }
        let Ok(section_count_usize) = usize::try_from(section_count) else {
            return Err(SnapshotError::UsizeOverflow {
                value: u64::from(section_count),
            });
        };
        let Some(table_len) = section_count_usize.checked_mul(SECTION_ENTRY_SIZE) else {
            return Err(SnapshotError::SectionCountTooLarge {
                count: section_count,
                max: MAX_SECTION_COUNT,
            });
        };
        if after_header.len() < table_len {
            return Err(SnapshotError::TruncatedSectionTable {
                needed: table_len,
                actual: after_header.len(),
            });
        }

        let table_bytes = &after_header[..table_len];
        let entries =
            <[RawSectionEntry]>::ref_from_bytes_with_elems(table_bytes, section_count_usize)
                .map_err(|_error| SnapshotError::MalformedSectionTable)?;

        validate_section_table(bytes, entries, level)?;

        Ok(Self {
            bytes,
            format_major,
            format_minor,
            entries,
        })
    }

    /// Returns the format major version recorded in the snapshot header.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn format_major(&self) -> u32 {
        self.format_major
    }

    /// Returns the format minor version recorded in the snapshot header.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn format_minor(&self) -> u32 {
        self.format_minor
    }

    /// Returns the number of validated sections.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns an iterator over all validated sections.
    ///
    /// # Performance
    ///
    /// Constructing the iterator is `O(1)`; advancing it is `O(1)` per step.
    #[must_use]
    pub fn sections(&self) -> SectionIter<'view> {
        SectionIter {
            bytes: self.bytes,
            entries: self.entries.iter(),
        }
    }

    /// Returns the section with the given `kind`, when present.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for `s` section entries.
    #[must_use]
    pub fn section(&self, kind: u32) -> Option<Section<'view>> {
        let bytes = self.bytes;
        self.entries.iter().find_map(|entry| {
            if entry.kind.get() == kind {
                Some(Section::from_entry(bytes, entry))
            } else {
                None
            }
        })
    }

    /// Binds a width-typed section by kind and version in one step.
    ///
    /// Looks up the section, checks its version against `expected_version`, and
    /// borrows the payload as `&[W::LittleEndianWord]`. This is the single
    /// section-open primitive every layout crate reuses instead of
    /// re-implementing the lookup/version/typed-view sequence with its own error
    /// variants; callers map [`SectionBindError`] into their own typed error at
    /// the boundary.
    ///
    /// # Errors
    ///
    /// Returns [`SectionBindError::Missing`] when no section has `kind`,
    /// [`SectionBindError::VersionMismatch`] when the recorded version differs,
    /// and [`SectionBindError::View`] when the payload cannot be borrowed as the
    /// requested little-endian word.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for `s` section entries plus the typed-view checks.
    pub fn typed_section<W>(
        &self,
        kind: u32,
        expected_version: u32,
    ) -> Result<&'view [W::LittleEndianWord], SectionBindError>
    where
        W: SnapshotWidth,
    {
        let section = self
            .section(kind)
            .ok_or(SectionBindError::Missing { kind })?;
        if section.version() != expected_version {
            return Err(SectionBindError::VersionMismatch {
                kind,
                expected: expected_version,
                actual: section.version(),
            });
        }
        section
            .try_as_slice::<W::LittleEndianWord>()
            .map_err(|error| SectionBindError::View { kind, error })
    }
}

/// Iterator over a snapshot's validated sections.
///
/// Yields each [`Section`] in section-table order. The iterator does not
/// allocate and borrows from the snapshot's underlying byte slice.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` per step.
#[derive(Clone, Debug)]
pub struct SectionIter<'view> {
    /// Borrowed snapshot bytes.
    bytes: &'view [u8],
    /// Remaining section table entries to yield.
    entries: core::slice::Iter<'view, RawSectionEntry>,
}

impl<'view> Iterator for SectionIter<'view> {
    type Item = Section<'view>;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries
            .next()
            .map(|entry| Section::from_entry(self.bytes, entry))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl ExactSizeIterator for SectionIter<'_> {
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Description of one section to include in a snapshot.
///
/// Every field is opaque to the encoder. `kind` and `version` are passed
/// through unchanged; `alignment_log2` controls payload alignment relative
/// to the snapshot's start; `payload` is the section's raw bytes.
///
/// # Performance
///
/// `perf: unspecified`; this is a metadata struct.
#[derive(Clone, Copy, Debug)]
pub struct PendingSection<'a> {
    /// Section kind to record in the entry.
    pub kind: u32,
    /// Section version to record in the entry.
    pub version: u32,
    /// `log2` of the requested payload alignment; capped at
    /// [`MAX_ALIGNMENT_LOG2`].
    pub alignment_log2: u8,
    /// Section payload bytes.
    pub payload: &'a [u8],
}

/// Validated plan that can compute its encoded length and write itself.
///
/// `SnapshotPlan` performs all duplicate-kind, alignment, and count checks
/// at construction. After construction, [`encoded_len`](Self::encoded_len)
/// and [`write_into`](Self::write_into) are guaranteed to succeed for any
/// caller-supplied buffer that is at least `encoded_len()` bytes long.
///
/// # Performance
///
/// Construction is `O(s^2)` for `s` sections (duplicate-kind check).
/// `encoded_len` and `write_into` are `O(s + total payload bytes)`.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotPlan<'a> {
    /// Borrowed pending section descriptors, in declaration order.
    sections: &'a [PendingSection<'a>],
}

impl<'a> SnapshotPlan<'a> {
    /// Validates a slice of pending sections and constructs a plan.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] when alignment is too large, too many sections
    /// are supplied, or duplicate kinds are present.
    ///
    /// # Performance
    ///
    /// This function is `O(s^2)` for `s` sections.
    pub fn new(sections: &'a [PendingSection<'a>]) -> Result<Self, PlanError> {
        if sections.len() > MAX_SECTION_COUNT as usize {
            return Err(PlanError::TooManySections {
                count: sections.len(),
            });
        }

        for section in sections {
            if section.alignment_log2 > MAX_ALIGNMENT_LOG2 {
                return Err(PlanError::AlignmentTooLarge {
                    alignment_log2: section.alignment_log2,
                });
            }
        }
        if let Some(kind) = first_duplicate_kind(sections, |section| section.kind) {
            return Err(PlanError::DuplicateKind { kind });
        }

        Ok(Self { sections })
    }

    /// Returns the number of sections in this plan.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Computes the total bytes the encoded snapshot will occupy.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::PayloadOverflow`] when offset arithmetic
    /// exceeds `usize` or `u64` representable values.
    ///
    /// # Performance
    ///
    /// This function is `O(s)` for `s` sections.
    pub fn encoded_len(&self) -> Result<usize, PlanError> {
        let table_len = self
            .sections
            .len()
            .checked_mul(SECTION_ENTRY_SIZE)
            .ok_or(PlanError::PayloadOverflow)?;
        let mut total = HEADER_SIZE
            .checked_add(table_len)
            .ok_or(PlanError::PayloadOverflow)?;

        for section in self.sections {
            total = align_up_checked(total, section.alignment_log2)?;
            total = total
                .checked_add(section.payload.len())
                .ok_or(PlanError::PayloadOverflow)?;
        }

        u64::try_from(total).map_err(|_error| PlanError::PayloadOverflow)?;
        Ok(total)
    }

    /// Writes the encoded snapshot into `out` and returns the number of
    /// bytes written.
    ///
    /// Padding bytes between the section table and each section payload
    /// are zero-filled deterministically; the resulting bytes are stable
    /// for any logical input.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::BufferTooSmall`] when `out.len()` is less than
    /// [`encoded_len`](Self::encoded_len) or [`PlanError::PayloadOverflow`]
    /// when offset arithmetic overflows during the write walk.
    ///
    /// # Performance
    ///
    /// This function is `O(s + total payload bytes)`.
    pub fn write_into(&self, out: &mut [u8]) -> Result<usize, PlanError> {
        let needed = self.encoded_len()?;
        if out.len() < needed {
            return Err(PlanError::BufferTooSmall {
                needed,
                actual: out.len(),
            });
        }

        let prefix = &mut out[..needed];
        prefix.fill(0);

        let section_count_u32 = match u32::try_from(self.sections.len()) {
            Ok(value) => value,
            Err(_error) => {
                return Err(PlanError::TooManySections {
                    count: self.sections.len(),
                });
            }
        };
        let header = RawHeader {
            magic: FORMAT_MAGIC,
            format_major: U32::new(FORMAT_MAJOR),
            format_minor: U32::new(FORMAT_MINOR),
            header_size: U32::new(HEADER_SIZE_U32),
            section_count: U32::new(section_count_u32),
            reserved: [0; 8],
        };
        prefix[..HEADER_SIZE].copy_from_slice(header.as_bytes());

        let table_start = HEADER_SIZE;
        let payload_start = table_start
            .checked_add(
                self.sections
                    .len()
                    .checked_mul(SECTION_ENTRY_SIZE)
                    .ok_or(PlanError::PayloadOverflow)?,
            )
            .ok_or(PlanError::PayloadOverflow)?;
        let mut cursor = payload_start;

        for (index, section) in self.sections.iter().enumerate() {
            cursor = align_up_checked(cursor, section.alignment_log2)?;
            let payload_end = cursor
                .checked_add(section.payload.len())
                .ok_or(PlanError::PayloadOverflow)?;

            let offset_u64 = u64::try_from(cursor).map_err(|_error| PlanError::PayloadOverflow)?;
            let length_u64 = u64::try_from(section.payload.len())
                .map_err(|_error| PlanError::PayloadOverflow)?;
            let entry = RawSectionEntry {
                offset: U64::new(offset_u64),
                length: U64::new(length_u64),
                kind: U32::new(section.kind),
                version: U32::new(section.version),
                reserved_checksum: [0; 4],
                alignment_log2: section.alignment_log2,
                flags: 0,
                reserved: [0; 2],
            };
            let entry_offset = table_start
                .checked_add(
                    index
                        .checked_mul(SECTION_ENTRY_SIZE)
                        .ok_or(PlanError::PayloadOverflow)?,
                )
                .ok_or(PlanError::PayloadOverflow)?;
            prefix[entry_offset..entry_offset + SECTION_ENTRY_SIZE]
                .copy_from_slice(entry.as_bytes());

            prefix[cursor..payload_end].copy_from_slice(section.payload);
            cursor = payload_end;
        }

        Ok(needed)
    }
}

/// Rounds `value` up to the next multiple of `1 << alignment_log2`.
///
/// # Errors
///
/// Returns [`PlanError::PayloadOverflow`] on `usize` overflow.
///
/// # Performance
///
/// This function is `O(1)`.
fn align_up_checked(value: usize, alignment_log2: u8) -> Result<usize, PlanError> {
    let alignment = 1usize << alignment_log2;
    let mask = alignment - 1;
    let added = value.checked_add(mask).ok_or(PlanError::PayloadOverflow)?;
    Ok(added & !mask)
}

/// One owned section pending in a [`SnapshotBuilder`].
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
struct OwnedSection {
    /// Section kind.
    kind: u32,
    /// Section version.
    version: u32,
    /// Declared payload alignment as `log2`.
    alignment_log2: u8,
    /// Owned payload bytes.
    payload: Vec<u8>,
}

/// Owning snapshot builder that produces a `Vec<u8>` on finish.
///
/// The builder rejects duplicate kinds, alignment overflows, and section
/// count overflows at `add_section*` time, so the only failure that can
/// reach [`SnapshotBuilder::finish`] is [`PlanError::PayloadOverflow`] —
/// when the cumulative encoded length would overflow `u64` or `usize`.
///
/// # Performance
///
/// `add_section*` methods are `O(s)` for the in-progress section count.
/// [`finish`](Self::finish) is `O(s + total payload bytes)`.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct SnapshotBuilder {
    /// Owned, in-order sections.
    sections: Vec<OwnedSection>,
}

#[cfg(feature = "alloc")]
impl SnapshotBuilder {
    /// Constructs an empty builder.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a section with the given metadata and owned payload.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] when `alignment_log2` is too large, when the
    /// builder already holds the maximum permitted section count, or when
    /// `kind` collides with an earlier section.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for the in-progress section count.
    pub fn add_section(
        &mut self,
        kind: u32,
        version: u32,
        alignment_log2: u8,
        payload: Vec<u8>,
    ) -> Result<&mut Self, PlanError> {
        if alignment_log2 > MAX_ALIGNMENT_LOG2 {
            return Err(PlanError::AlignmentTooLarge { alignment_log2 });
        }
        if self.sections.len() >= MAX_SECTION_COUNT as usize {
            return Err(PlanError::TooManySections {
                count: self.sections.len() + 1,
            });
        }
        for prior in &self.sections {
            if prior.kind == kind {
                return Err(PlanError::DuplicateKind { kind });
            }
        }
        self.sections.push(OwnedSection {
            kind,
            version,
            alignment_log2,
            payload,
        });
        Ok(self)
    }

    /// Appends a section whose alignment is derived from `T`.
    ///
    /// The payload is copied via [`zerocopy::IntoBytes`].
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] for the same reasons as
    /// [`add_section`](Self::add_section), plus
    /// [`PlanError::AlignmentTooLarge`] when `align_of::<T>()` exceeds
    /// the v1 cap.
    ///
    /// # Performance
    ///
    /// This method is `O(s + payload.len() * size_of::<T>())`.
    pub fn add_section_typed<T>(
        &mut self,
        kind: u32,
        version: u32,
        payload: &[T],
    ) -> Result<&mut Self, PlanError>
    where
        T: zerocopy::IntoBytes + zerocopy::Immutable,
    {
        let alignment = core::mem::align_of::<T>();
        let alignment_log2 = match u8::try_from(alignment.trailing_zeros()) {
            Ok(value) => value,
            Err(_error) => {
                return Err(PlanError::AlignmentTooLarge {
                    alignment_log2: u8::MAX,
                });
            }
        };
        if alignment_log2 > MAX_ALIGNMENT_LOG2 {
            return Err(PlanError::AlignmentTooLarge { alignment_log2 });
        }
        let bytes = payload.as_bytes().to_vec();
        self.add_section(kind, version, alignment_log2, bytes)
    }

    /// Appends a section containing explicit little-endian typed words.
    ///
    /// Prefer [`add_section_widths`](Self::add_section_widths), which takes a
    /// native index slice and lowers it through `slice_to_le`, enforcing the
    /// little-endian guarantee in the type system. This method exists for
    /// callers that already hold portable byteorder words such as
    /// `zerocopy::byteorder::U32<LE>`; the payload is copied via
    /// [`zerocopy::IntoBytes`] and the alignment is derived from `T`.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] for the same reasons as
    /// [`add_section_typed`](Self::add_section_typed).
    ///
    /// # Performance
    ///
    /// This method is `O(s + payload.len() * size_of::<T>())`.
    pub fn add_section_little_endian<T>(
        &mut self,
        kind: u32,
        version: u32,
        payload: &[T],
    ) -> Result<&mut Self, PlanError>
    where
        T: zerocopy::IntoBytes + zerocopy::Immutable,
    {
        self.add_section_typed(kind, version, payload)
    }

    /// Appends a section from a native-width index slice, lowering it to its
    /// explicit little-endian storage words first.
    ///
    /// Convenience wrapper that calls `oxgraph_layout_util::slice_to_le` and
    /// then [`add_section_little_endian`](Self::add_section_little_endian), so
    /// exporters can pass native `&[u32]`-style slices without converting by
    /// hand. Requires the `alloc` feature (it allocates the converted words).
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] for the same reasons as
    /// [`add_section_typed`](Self::add_section_typed).
    ///
    /// # Performance
    ///
    /// This method is `O(s + payload.len())` plus one allocation for the
    /// converted words.
    pub fn add_section_widths<W>(
        &mut self,
        kind: u32,
        version: u32,
        values: &[W],
    ) -> Result<&mut Self, PlanError>
    where
        W: SnapshotWidth,
    {
        let words = oxgraph_layout_util::slice_to_le(values);
        self.add_section_little_endian(kind, version, &words)
    }

    /// Returns the number of pending sections.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Encodes the pending sections into an owned snapshot byte vector.
    ///
    /// The builder enforces per-insert invariants
    /// ([`PlanError::AlignmentTooLarge`], [`PlanError::TooManySections`],
    /// [`PlanError::DuplicateKind`]) on `add_section*`, so this method
    /// only fails when the cumulative payload arithmetic overflows
    /// (`u64`/`usize`), surfaced as [`PlanError::PayloadOverflow`].
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::PayloadOverflow`] when the total encoded length
    /// would overflow `u64` or `usize`. All other [`PlanError`] variants
    /// are caught at `add_section*` time and cannot reach this method.
    ///
    /// # Performance
    ///
    /// This method is `O(s + total payload bytes)`.
    pub fn finish(self) -> Result<Vec<u8>, PlanError> {
        let pending: Vec<PendingSection<'_>> = self
            .sections
            .iter()
            .map(|section| PendingSection {
                kind: section.kind,
                version: section.version,
                alignment_log2: section.alignment_log2,
                payload: section.payload.as_slice(),
            })
            .collect();

        let plan = SnapshotPlan::new(&pending)?;
        let needed = plan.encoded_len()?;
        let mut out = vec![0u8; needed];
        plan.write_into(&mut out)?;
        Ok(out)
    }
}

/// Write-through snapshot encoder that lays payload bytes out at their final
/// offsets in a single buffer.
///
/// [`SnapshotBuilder`] owns every payload and copies the whole snapshot again
/// at finish, holding ~2x the encoded bytes at peak; this writer streams each
/// payload directly into the final buffer instead.
///
/// The table region is reserved up-front for `max_sections` entries; sections
/// written beyond the reservation are rejected. When fewer sections are
/// written, the unused table slots remain zero between the table and the first
/// payload — the same class of never-dereferenced bytes as alignment padding
/// (every entry offset stays in bounds and monotonic, so validation accepts
/// the layout). Writing exactly `max_sections` sections produces bytes
/// identical to [`SnapshotBuilder::finish`] for the same logical input.
///
/// # Performance
///
/// Each write appends `O(written bytes)`; [`finish`](Self::finish) is `O(s)`
/// for `s` sections (header + table patch, no payload copy).
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
#[must_use]
pub struct SnapshotWriter {
    /// Final snapshot bytes, laid out in place from byte zero (header and
    /// table are zero until [`Self::finish`] patches them).
    buf: Vec<u8>,
    /// Staged section entries, patched into the reserved table at finish.
    entries: Vec<RawSectionEntry>,
    /// Reserved table capacity in entries.
    max_sections: usize,
}

#[cfg(feature = "alloc")]
impl SnapshotWriter {
    /// Constructs a writer whose table region reserves `max_sections` entries.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::TooManySections`] when `max_sections` exceeds
    /// [`MAX_SECTION_COUNT`].
    ///
    /// # Performance
    ///
    /// This function is `O(reserved table bytes)` (one zero-filled
    /// allocation).
    pub fn new(max_sections: usize) -> Result<Self, PlanError> {
        Self::with_payload_capacity(max_sections, 0)
    }

    /// Constructs a writer reserving `max_sections` table entries and
    /// pre-allocating `payload_capacity` additional buffer bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::TooManySections`] when `max_sections` exceeds
    /// [`MAX_SECTION_COUNT`].
    ///
    /// # Performance
    ///
    /// This function is `O(reserved table bytes)` (one zero-filled
    /// allocation).
    pub fn with_payload_capacity(
        max_sections: usize,
        payload_capacity: usize,
    ) -> Result<Self, PlanError> {
        if max_sections > MAX_SECTION_COUNT as usize {
            return Err(PlanError::TooManySections {
                count: max_sections,
            });
        }
        let table_len = max_sections
            .checked_mul(SECTION_ENTRY_SIZE)
            .ok_or(PlanError::PayloadOverflow)?;
        let reserved = HEADER_SIZE
            .checked_add(table_len)
            .ok_or(PlanError::PayloadOverflow)?;
        let mut buf = Vec::with_capacity(reserved.saturating_add(payload_capacity));
        buf.resize(reserved, 0);
        Ok(Self {
            buf,
            entries: Vec::with_capacity(max_sections),
            max_sections,
        })
    }

    /// Starts a section, zero-padding the buffer to the requested alignment,
    /// and returns the sink that streams its payload bytes.
    ///
    /// The section's entry is recorded when the sink's [`SectionSink::end`]
    /// is called; a sink dropped without `end` leaves its bytes as
    /// never-referenced slack and records no entry.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::AlignmentTooLarge`] when `alignment_log2` exceeds
    /// the format cap, [`PlanError::TooManySections`] when the reservation is
    /// exhausted, or [`PlanError::DuplicateKind`] when `kind` collides with an
    /// earlier section.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for the in-progress section count (duplicate-kind
    /// scan) plus `O(padding)` zero fill.
    pub fn begin_section(
        &mut self,
        kind: u32,
        version: u32,
        alignment_log2: u8,
    ) -> Result<SectionSink<'_>, PlanError> {
        if alignment_log2 > MAX_ALIGNMENT_LOG2 {
            return Err(PlanError::AlignmentTooLarge { alignment_log2 });
        }
        if self.entries.len() >= self.max_sections {
            return Err(PlanError::TooManySections {
                count: self.entries.len() + 1,
            });
        }
        for prior in &self.entries {
            if prior.kind.get() == kind {
                return Err(PlanError::DuplicateKind { kind });
            }
        }
        let aligned = align_up_checked(self.buf.len(), alignment_log2)?;
        self.buf.resize(aligned, 0);
        Ok(SectionSink {
            start: aligned,
            kind,
            version,
            alignment_log2,
            writer: self,
        })
    }

    /// Writes one whole section whose alignment is derived from `T`, copying
    /// the records via [`zerocopy::IntoBytes`] directly into the final buffer.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] for the same reasons as
    /// [`begin_section`](Self::begin_section), plus
    /// [`PlanError::AlignmentTooLarge`] when `align_of::<T>()` exceeds the
    /// format cap.
    ///
    /// # Performance
    ///
    /// This method is `O(s + records.len() * size_of::<T>())`.
    pub fn section_typed<T>(
        &mut self,
        kind: u32,
        version: u32,
        records: &[T],
    ) -> Result<(), PlanError>
    where
        T: zerocopy::IntoBytes + zerocopy::Immutable,
    {
        let alignment = core::mem::align_of::<T>();
        let alignment_log2 = match u8::try_from(alignment.trailing_zeros()) {
            Ok(value) => value,
            Err(_error) => {
                return Err(PlanError::AlignmentTooLarge {
                    alignment_log2: u8::MAX,
                });
            }
        };
        let mut sink = self.begin_section(kind, version, alignment_log2)?;
        sink.write_typed(records);
        sink.end()
    }

    /// Returns the number of sections recorded so far.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.entries.len()
    }

    /// Patches the header and the reserved table with the recorded entries and
    /// returns the encoded snapshot bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::PayloadOverflow`] when the total encoded length is
    /// not representable as `u64`.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for `s` sections; payload bytes are not copied.
    pub fn finish(mut self) -> Result<Vec<u8>, PlanError> {
        u64::try_from(self.buf.len()).map_err(|_error| PlanError::PayloadOverflow)?;
        let section_count_u32 = match u32::try_from(self.entries.len()) {
            Ok(value) => value,
            Err(_error) => {
                return Err(PlanError::TooManySections {
                    count: self.entries.len(),
                });
            }
        };
        let header = RawHeader {
            magic: FORMAT_MAGIC,
            format_major: U32::new(FORMAT_MAJOR),
            format_minor: U32::new(FORMAT_MINOR),
            header_size: U32::new(HEADER_SIZE_U32),
            section_count: U32::new(section_count_u32),
            reserved: [0; 8],
        };
        self.buf[..HEADER_SIZE].copy_from_slice(header.as_bytes());
        for (index, entry) in self.entries.iter().enumerate() {
            let entry_offset = HEADER_SIZE + index * SECTION_ENTRY_SIZE;
            self.buf[entry_offset..entry_offset + SECTION_ENTRY_SIZE]
                .copy_from_slice(entry.as_bytes());
        }
        Ok(self.buf)
    }
}

/// Streaming payload sink for one in-progress [`SnapshotWriter`] section.
///
/// Bytes written here land directly at their final offsets in the snapshot
/// buffer. [`Self::end`] records the section's table entry; dropping the sink
/// without `end` records nothing and leaves the written bytes as
/// never-referenced slack.
///
/// # Performance
///
/// Each write is `O(written bytes)` (one `Vec` append).
#[cfg(feature = "alloc")]
#[must_use]
pub struct SectionSink<'writer> {
    /// Writer whose buffer receives the payload bytes.
    writer: &'writer mut SnapshotWriter,
    /// Buffer offset where this section's payload starts.
    start: usize,
    /// Section kind recorded at [`Self::end`].
    kind: u32,
    /// Section version recorded at [`Self::end`].
    version: u32,
    /// Declared payload alignment recorded at [`Self::end`].
    alignment_log2: u8,
}

#[cfg(feature = "alloc")]
impl SectionSink<'_> {
    /// Appends raw payload bytes to this section.
    ///
    /// # Performance
    ///
    /// This method is `O(bytes.len())`.
    pub fn write(&mut self, bytes: &[u8]) {
        self.writer.buf.extend_from_slice(bytes);
    }

    /// Appends typed records to this section via [`zerocopy::IntoBytes`].
    ///
    /// # Performance
    ///
    /// This method is `O(records.len() * size_of::<T>())`.
    pub fn write_typed<T>(&mut self, records: &[T])
    where
        T: zerocopy::IntoBytes + zerocopy::Immutable,
    {
        self.write(records.as_bytes());
    }

    /// Finishes this section, recording its table entry.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::PayloadOverflow`] when the section offset or
    /// length is not representable as `u64`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub fn end(self) -> Result<(), PlanError> {
        let offset = u64::try_from(self.start).map_err(|_error| PlanError::PayloadOverflow)?;
        let length = u64::try_from(self.writer.buf.len() - self.start)
            .map_err(|_error| PlanError::PayloadOverflow)?;
        self.writer.entries.push(RawSectionEntry {
            offset: U64::new(offset),
            length: U64::new(length),
            kind: U32::new(self.kind),
            version: U32::new(self.version),
            reserved_checksum: [0; 4],
            alignment_log2: self.alignment_log2,
            flags: 0,
            reserved: [0; 2],
        });
        Ok(())
    }
}
