//! Byte-level header definition and parsing for the snapshot container.
//!
//! The header is exactly [`HEADER_SIZE`](super::HEADER_SIZE) bytes and uses
//! unaligned little-endian integer fields so that snapshots can be borrowed
//! from arbitrarily-aligned input slices (e.g. mmap'd files, sub-slices, or
//! `Vec<u8>` allocations).

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U32},
};

use super::{
    FORMAT_MAGIC, FORMAT_MAJOR, HEADER_SIZE, HEADER_SIZE_U32, MAX_SUPPORTED_MINOR,
    error::SnapshotError,
};

/// Byte-level snapshot header.
///
/// Layout is `#[repr(C)]` with all multi-byte fields stored as zerocopy's
/// unaligned little-endian wrappers. The struct itself has alignment 1, so
/// it can be borrowed from any byte slice that is at least `HEADER_SIZE`
/// long without an alignment check.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(in crate::container) struct RawHeader {
    /// Magic bytes; must equal [`FORMAT_MAGIC`](super::FORMAT_MAGIC).
    pub(in crate::container) magic: [u8; 8],
    /// Format major version.
    pub(in crate::container) format_major: U32<LE>,
    /// Format minor version.
    pub(in crate::container) format_minor: U32<LE>,
    /// Header size in bytes; v1.0 mandates `HEADER_SIZE`.
    pub(in crate::container) header_size: U32<LE>,
    /// Number of section table entries.
    pub(in crate::container) section_count: U32<LE>,
    /// Reserved; must be zero.
    pub(in crate::container) reserved: [u8; 8],
}

/// Parses the fixed header from the start of `bytes`.
///
/// # Errors
///
/// Returns [`SnapshotError::TruncatedHeader`] when fewer than
/// [`HEADER_SIZE`](super::HEADER_SIZE) bytes are provided. Header field
/// validation is performed separately in [`validate_magic_versions_reserved`].
///
/// # Performance
///
/// This function is `O(1)`.
pub(in crate::container) fn parse_header(
    bytes: &[u8],
) -> Result<(&RawHeader, &[u8]), SnapshotError> {
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
pub(in crate::container) fn validate_magic_versions_reserved(
    header: &RawHeader,
) -> Result<(), SnapshotError> {
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
