//! Validated [`Snapshot`] reader and section iteration.

use zerocopy::FromBytes;

use super::{
    MAX_SECTION_COUNT, SECTION_ENTRY_SIZE,
    error::SnapshotError,
    header::{parse_header, validate_magic_versions_reserved},
    section::{RawSectionEntry, Section},
    validate::{ValidationLevel, validate_section_table},
};

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
