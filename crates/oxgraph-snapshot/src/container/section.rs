//! Byte-level section table entry and the public [`Section`] view type.

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U32, U64},
};

use super::{error::SectionViewError, u64_to_usize_validated};

/// Byte-level section table entry.
///
/// Layout is `#[repr(C)]` with unaligned little-endian fields, mirroring
/// [`RawHeader`](super::header::RawHeader)'s alignment policy.
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub(in crate::container) struct RawSectionEntry {
    /// Byte offset of the section payload from the start of the snapshot.
    pub(in crate::container) offset: U64<LE>,
    /// Byte length of the section payload.
    pub(in crate::container) length: U64<LE>,
    /// Opaque section kind; the container assigns no semantics.
    pub(in crate::container) kind: U32<LE>,
    /// Opaque section version; consumers interpret per kind.
    pub(in crate::container) version: U32<LE>,
    /// Reserved bytes for a future per-section CRC32C; must be zero in v1.
    pub(in crate::container) reserved_checksum: [u8; 4],
    /// `log2` of the producer's chosen payload alignment; v1 cap is 12.
    pub(in crate::container) alignment_log2: u8,
    /// Reserved flag bits; must be zero in v1.
    pub(in crate::container) flags: u8,
    /// Trailing reserved bytes; must be zero in v1.
    pub(in crate::container) reserved: [u8; 2],
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
    pub(in crate::container) fn from_entry(bytes: &'view [u8], entry: &RawSectionEntry) -> Self {
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
            return Err(SectionViewError::LengthNotMultipleOfSize { length, elem_size });
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
