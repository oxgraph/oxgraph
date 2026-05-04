//! Validation levels and section-table invariant checking.

use super::{
    HEADER_SIZE, MAX_ALIGNMENT_LOG2, SECTION_ENTRY_SIZE, error::SnapshotError,
    section::RawSectionEntry,
};

/// Validation depth applied at snapshot open time.
///
/// Validation responsibilities are layered. Header-only validation is not a
/// member of this enum; callers wanting it should use
/// [`HeaderOnlySnapshot::open`](crate::HeaderOnlySnapshot::open) instead, so
/// the type system distinguishes a section-bearing handle from one whose
/// section table has not been validated.
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

/// Walks the section table once and checks all v1 invariants.
///
/// `bytes` is the entire snapshot byte slice; `entries` is the parsed
/// section table; `level` controls how deep the walk goes. Header-level
/// invariants are presumed already validated by the caller.
///
/// # Errors
///
/// Returns [`SnapshotError`] for any per-entry or layout violation.
///
/// # Performance
///
/// This function is `O(s)` for the per-entry self-consistency walk and
/// `O(s^2)` for the duplicate-kind walk at [`Layout`](ValidationLevel::Layout).
pub(in crate::container) fn validate_section_table(
    bytes: &[u8],
    entries: &[RawSectionEntry],
    level: ValidationLevel,
) -> Result<(), SnapshotError> {
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
    }

    if matches!(level, ValidationLevel::SectionTable) {
        return Ok(());
    }

    let snapshot_len = bytes.len() as u64;
    let header_plus_table = (HEADER_SIZE as u64)
        .checked_add((entries.len() as u64).saturating_mul(SECTION_ENTRY_SIZE as u64))
        .ok_or(SnapshotError::SectionRangeOverflow { kind: 0 })?;
    let mut prev_end = header_plus_table;

    for (index, entry) in entries.iter().enumerate() {
        let kind = entry.kind.get();
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
        if offset < prev_end {
            return Err(SnapshotError::UnsortedSectionTable { index });
        }
        prev_end = end;

        for prior in &entries[..index] {
            if prior.kind.get() == kind {
                return Err(SnapshotError::DuplicateKind { kind });
            }
        }
    }

    Ok(())
}
