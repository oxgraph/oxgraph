//! Validated byte-level graph snapshots.
//!
//! `oxgraph-snapshot` provides a minimal internal v0 snapshot container reader.
//! The container validates a header and section table, then exposes borrowed
//! section bytes for layout-specific crates or callers to interpret. It is not a
//! stable ABI.
#![no_std]

#[cfg(kani)]
extern crate kani;

use core::{fmt, mem, ops::Range};

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U32},
};

/// Snapshot magic bytes for the internal v0 graph format.
const MAGIC: [u8; 8] = *b"OCTXG0\0\0";

/// Internal v0 graph snapshot major version.
const VERSION_MAJOR: u32 = 0;

/// Internal v0 graph snapshot minor version.
const VERSION_MINOR: u32 = 1;

/// Byte-level snapshot header.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
struct SnapshotHeader {
    /// Magic bytes identifying this snapshot format.
    magic: [u8; 8],
    /// Major version.
    major: U32<LE>,
    /// Minor version.
    minor: U32<LE>,
    /// Number of graph nodes.
    node_count: U32<LE>,
    /// Number of section table entries.
    section_count: U32<LE>,
}

/// Byte-level section table entry.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
struct SectionHeader {
    /// Section kind identifier.
    kind: U32<LE>,
    /// Section byte offset from the start of the snapshot.
    offset: U32<LE>,
    /// Section byte length.
    length: U32<LE>,
}

/// Validated graph snapshot container.
///
/// A snapshot borrows the original byte slice and exposes validated section
/// ranges. Layout-specific interpretation, such as opening CSR or CSC graph
/// views, belongs outside this crate.
///
/// # Performance
///
/// Validation is `O(s^2)` for `s` section entries because duplicate section
/// kinds and overlapping section ranges are checked without allocation.
/// Accessing counts is `O(1)`. Looking up a section by kind is `O(s)`.
#[derive(Clone, Copy, Debug)]
pub struct GraphSnapshot<'view> {
    /// Original validated snapshot bytes.
    bytes: &'view [u8],
    /// Number of graph nodes recorded in the container header.
    node_count: u32,
    /// Validated section table entries.
    sections: &'view [SectionHeader],
}

impl<'view> GraphSnapshot<'view> {
    /// Validates `bytes` as an internal v0 graph snapshot container.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] when the header, version, section table, section
    /// bounds, duplicate section kinds, or section range overlap invariants are
    /// invalid.
    ///
    /// # Performance
    ///
    /// Validation is `O(s^2)` for `s` section entries because duplicate section
    /// kinds and overlapping section ranges are checked without allocation.
    pub fn validate(bytes: &'view [u8]) -> Result<Self, SnapshotError> {
        let (header, section_bytes) = parse_header(bytes)?;
        validate_version(header)?;

        let section_count = u32_to_usize(header.section_count.get())?;
        let section_table_len = section_count
            .checked_mul(mem::size_of::<SectionHeader>())
            .ok_or(SnapshotError::SectionTableLengthOverflow { section_count })?;
        if section_bytes.len() < section_table_len {
            return Err(SnapshotError::TruncatedSectionTable {
                needed: section_table_len,
                actual: section_bytes.len(),
            });
        }

        let table_bytes = &section_bytes[..section_table_len];
        let sections = <[SectionHeader]>::ref_from_bytes_with_elems(table_bytes, section_count)
            .map_err(|_error| SnapshotError::MalformedSectionTable)?;
        validate_sections(bytes, sections)?;

        Ok(Self {
            bytes,
            node_count: header.node_count.get(),
            sections,
        })
    }

    /// Returns the graph node count recorded in the snapshot header.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn node_count(&self) -> u32 {
        self.node_count
    }

    /// Returns the number of sections recorded in the snapshot header.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Returns the borrowed bytes for section `kind`, when present.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for `s` section entries.
    #[must_use]
    pub fn section_bytes(&self, kind: u32) -> Option<&'view [u8]> {
        self.find_section(kind).and_then(|section| {
            section_range(self.bytes, section)
                .ok()
                .map(|range| &self.bytes[range])
        })
    }

    /// Returns section `kind` as unaligned little-endian `u32` words.
    ///
    /// This is a convenience for word-oriented layout sections. The snapshot
    /// container itself does not assign semantics to section kinds.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] when the section exists but its length or byte
    /// layout cannot be viewed as little-endian `u32` words.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for `s` section entries.
    pub fn section_words(&self, kind: u32) -> Result<Option<&'view [U32<LE>]>, SnapshotError> {
        let Some(section) = self.find_section(kind) else {
            return Ok(None);
        };
        let range = section_range(self.bytes, section)?;
        section_words(self.bytes, section, range).map(Some)
    }

    /// Returns the first section header matching `kind`.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for `s` section entries.
    fn find_section(&self, kind: u32) -> Option<&SectionHeader> {
        self.sections
            .iter()
            .find(|section| section.kind.get() == kind)
    }
}

/// Snapshot validation error.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from validation paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    /// Header bytes were missing or malformed.
    MalformedHeader,
    /// Magic bytes did not match the internal v0 graph snapshot format.
    BadMagic {
        /// Actual magic bytes from the snapshot.
        actual: [u8; 8],
    },
    /// Snapshot version is not supported by this reader.
    UnsupportedVersion {
        /// Major version in the snapshot.
        major: u32,
        /// Minor version in the snapshot.
        minor: u32,
    },
    /// Section table byte length overflowed `usize`.
    SectionTableLengthOverflow {
        /// Number of section table entries.
        section_count: usize,
    },
    /// Section table bytes were truncated.
    TruncatedSectionTable {
        /// Needed section table byte length.
        needed: usize,
        /// Available bytes after the header.
        actual: usize,
    },
    /// Section table bytes were malformed.
    MalformedSectionTable,
    /// The same section kind appeared more than once.
    DuplicateSection {
        /// Duplicated section kind.
        kind: u32,
    },
    /// Section offset plus length overflowed.
    SectionRangeOverflow {
        /// Section kind.
        kind: u32,
        /// Section byte offset.
        offset: u32,
        /// Section byte length.
        length: u32,
    },
    /// Section byte range was outside the snapshot.
    SectionOutOfBounds {
        /// Section kind.
        kind: u32,
        /// Section byte offset.
        offset: usize,
        /// Section byte length.
        length: usize,
        /// Total snapshot length.
        snapshot_len: usize,
    },
    /// Two sections overlap in the byte slice.
    SectionOverlap {
        /// First section kind.
        first_kind: u32,
        /// Second section kind.
        second_kind: u32,
    },
    /// A word section was not a multiple of four bytes.
    MisalignedWordLength {
        /// Section kind.
        kind: u32,
        /// Section byte length.
        length: usize,
    },
    /// A word section could not be viewed as little-endian `u32` words.
    MalformedWordSection {
        /// Section kind.
        kind: u32,
    },
    /// A `u32` value could not be represented as `usize` on this target.
    UsizeOverflow {
        /// Value that could not be represented as `usize`.
        value: u32,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedHeader => formatter.write_str("snapshot header is missing or malformed"),
            Self::BadMagic { actual } => write!(formatter, "bad snapshot magic: {actual:?}"),
            Self::UnsupportedVersion { major, minor } => {
                write!(formatter, "unsupported snapshot version {major}.{minor}")
            }
            Self::SectionTableLengthOverflow { section_count } => write!(
                formatter,
                "section table length overflow for {section_count} sections"
            ),
            Self::TruncatedSectionTable { needed, actual } => write!(
                formatter,
                "truncated section table: needed {needed} bytes, got {actual}"
            ),
            Self::MalformedSectionTable => formatter.write_str("section table is malformed"),
            Self::DuplicateSection { kind } => write!(formatter, "duplicate section {kind}"),
            Self::SectionRangeOverflow {
                kind,
                offset,
                length,
            } => write!(
                formatter,
                "section {kind} range overflow: offset {offset}, length {length}"
            ),
            Self::SectionOutOfBounds {
                kind,
                offset,
                length,
                snapshot_len,
            } => write!(
                formatter,
                "section {kind} is out of bounds: offset {offset}, length {length}, snapshot length {snapshot_len}"
            ),
            Self::SectionOverlap {
                first_kind,
                second_kind,
            } => write!(formatter, "sections {first_kind} and {second_kind} overlap"),
            Self::MisalignedWordLength { kind, length } => write!(
                formatter,
                "section {kind} length {length} is not a multiple of four bytes"
            ),
            Self::MalformedWordSection { kind } => {
                write!(formatter, "section {kind} cannot be viewed as u32 words")
            }
            Self::UsizeOverflow { value } => {
                write!(formatter, "u32 value {value} does not fit usize")
            }
        }
    }
}

impl core::error::Error for SnapshotError {}

/// Parses the fixed header and returns remaining bytes.
///
/// # Performance
///
/// This function is `O(1)`.
fn parse_header(bytes: &[u8]) -> Result<(&SnapshotHeader, &[u8]), SnapshotError> {
    SnapshotHeader::ref_from_prefix(bytes).map_err(|_error| SnapshotError::MalformedHeader)
}

/// Validates header magic and version.
///
/// # Performance
///
/// This function is `O(1)`.
fn validate_version(header: &SnapshotHeader) -> Result<(), SnapshotError> {
    if header.magic != MAGIC {
        return Err(SnapshotError::BadMagic {
            actual: header.magic,
        });
    }

    let major = header.major.get();
    let minor = header.minor.get();
    if major != VERSION_MAJOR || minor != VERSION_MINOR {
        return Err(SnapshotError::UnsupportedVersion { major, minor });
    }

    Ok(())
}

/// Validates all section-table invariants.
///
/// # Performance
///
/// This function is `O(s^2)` for `s` section entries.
fn validate_sections(bytes: &[u8], sections: &[SectionHeader]) -> Result<(), SnapshotError> {
    for (index, section) in sections.iter().enumerate() {
        let range = section_range(bytes, section)?;
        for other in &sections[..index] {
            if section.kind.get() == other.kind.get() {
                return Err(SnapshotError::DuplicateSection {
                    kind: section.kind.get(),
                });
            }
            let other_range = section_range(bytes, other)?;
            reject_overlap(other.kind.get(), &other_range, section.kind.get(), &range)?;
        }
    }

    Ok(())
}

/// Returns one section as unaligned little-endian `u32` words.
///
/// # Performance
///
/// This function is `O(1)` after bounds checks.
fn section_words<'view>(
    bytes: &'view [u8],
    section: &SectionHeader,
    range: Range<usize>,
) -> Result<&'view [U32<LE>], SnapshotError> {
    let section_bytes = &bytes[range];
    if !section_bytes
        .len()
        .is_multiple_of(mem::size_of::<U32<LE>>())
    {
        return Err(SnapshotError::MisalignedWordLength {
            kind: section.kind.get(),
            length: section_bytes.len(),
        });
    }

    let word_count = section_bytes.len() / mem::size_of::<U32<LE>>();
    <[U32<LE>]>::ref_from_bytes_with_elems(section_bytes, word_count).map_err(|_error| {
        SnapshotError::MalformedWordSection {
            kind: section.kind.get(),
        }
    })
}

/// Returns the checked byte range for one section.
///
/// # Performance
///
/// This function is `O(1)`.
fn section_range(bytes: &[u8], section: &SectionHeader) -> Result<Range<usize>, SnapshotError> {
    let offset_u32 = section.offset.get();
    let length_u32 = section.length.get();
    let offset = u32_to_usize(offset_u32)?;
    let length = u32_to_usize(length_u32)?;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| SnapshotError::SectionRangeOverflow {
            kind: section.kind.get(),
            offset: offset_u32,
            length: length_u32,
        })?;

    if end > bytes.len() {
        return Err(SnapshotError::SectionOutOfBounds {
            kind: section.kind.get(),
            offset,
            length,
            snapshot_len: bytes.len(),
        });
    }

    Ok(offset..end)
}

/// Converts a `u32` to `usize` and reports overflow on narrow targets.
///
/// # Performance
///
/// This function is `O(1)`.
fn u32_to_usize(value: u32) -> Result<usize, SnapshotError> {
    usize::try_from(value).map_err(|_| SnapshotError::UsizeOverflow { value })
}

/// Rejects overlapping data sections.
///
/// # Performance
///
/// This function is `O(1)`.
const fn reject_overlap(
    first_kind: u32,
    first: &Range<usize>,
    second_kind: u32,
    second: &Range<usize>,
) -> Result<(), SnapshotError> {
    if first.start < second.end && second.start < first.end {
        return Err(SnapshotError::SectionOverlap {
            first_kind,
            second_kind,
        });
    }

    Ok(())
}
