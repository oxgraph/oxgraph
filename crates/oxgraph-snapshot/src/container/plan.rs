//! No-`alloc` snapshot encoder: [`PendingSection`] and [`SnapshotPlan`].
//!
//! [`SnapshotPlan`] borrows a slice of [`PendingSection`] descriptors and a
//! caller-provided output buffer, validates the plan once, then either
//! reports the encoded length or writes the snapshot in place. This is the
//! foundation for the alloc-gated [`SnapshotBuilder`](super::SnapshotBuilder)
//! and is also usable directly by `no_std` callers that own a stack or
//! static byte buffer.

use zerocopy::{
    IntoBytes,
    byteorder::{U32, U64},
};

use super::{
    FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, HEADER_SIZE_U32, MAX_ALIGNMENT_LOG2,
    MAX_SECTION_COUNT, SECTION_ENTRY_SIZE, error::PlanError, header::RawHeader,
    section::RawSectionEntry,
};

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
    /// [`MAX_ALIGNMENT_LOG2`](crate::MAX_ALIGNMENT_LOG2).
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

        for (index, section) in sections.iter().enumerate() {
            if section.alignment_log2 > MAX_ALIGNMENT_LOG2 {
                return Err(PlanError::AlignmentTooLarge {
                    alignment_log2: section.alignment_log2,
                });
            }
            check_no_prior_kind(sections, index, section.kind)?;
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

/// Returns [`PlanError::DuplicateKind`] when `kind` appears in any earlier
/// pending section.
///
/// # Errors
///
/// See above.
///
/// # Performance
///
/// This function is `O(index)`.
fn check_no_prior_kind(
    sections: &[PendingSection<'_>],
    index: usize,
    kind: u32,
) -> Result<(), PlanError> {
    for prior in &sections[..index] {
        if prior.kind == kind {
            return Err(PlanError::DuplicateKind { kind });
        }
    }
    Ok(())
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
