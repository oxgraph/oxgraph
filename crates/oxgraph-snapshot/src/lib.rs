//! Topology-agnostic byte-level snapshot container.
//!
//! `oxgraph-snapshot` defines a sectioned, zero-copy checkpoint format that
//! can validate, package, and read immutable topology snapshots without
//! knowing whether their contents are graphs, hypergraphs, or anything
//! else. Section semantics belong to upper layers (`oxgraph-csr`,
//! `oxgraph-graph`, `oxgraph-hyper`); the container only validates and
//! exposes byte-aligned section payloads.
//!
//! # Format overview
//!
//! Every snapshot consists of a fixed [`HEADER_SIZE`]-byte header, a
//! [`SECTION_ENTRY_SIZE`]-byte entry per section, and a payload region
//! whose offsets and lengths are recorded in the entries. All multi-byte
//! integer fields are little-endian and stored unaligned, so snapshots can
//! be borrowed from any byte slice — `Vec<u8>`, mmap'd files, sub-slices —
//! without an alignment requirement on the base pointer.
//!
//! v2 mandates two integrity invariants. Every section entry carries a
//! CRC-32C over its payload bytes and the header carries a CRC-32C over the
//! section-table bytes; the crate is `no_std` and bundles no CRC
//! implementation, so writers and the checked/verify read paths take a
//! [`Checksum32`] function (e.g. [`oxgraph_layout_util::crc32c_append`]).
//! Section kinds must be strictly ascending across the table, which makes
//! the table duplicate-free and [`Snapshot::section`] a binary search.
//!
//! Section payloads are exposed as byte slices via [`Section::bytes`]; the
//! [`Section::try_as_slice`] helper checks the actual payload pointer's
//! alignment against `align_of::<T>()` so consumers can reinterpret
//! `&[u8]` as `&[T]` only when it is safe to do so.
//!
//! # Cargo features
//!
//! - default: reader-only API; `no_std`, no allocation.
//! - `alloc`: enables [`SnapshotWriter`], the owning write-through encoder that returns encoded
//!   `Vec<u8>` bytes.
//! - `std`: reserved for future mmap helpers; no effect beyond activating `alloc`.
//!
//! # Stability
//!
//! v2.0 is a clean format break from v1 (mandatory checksums, ascending
//! kinds); v1 bytes are rejected at open. The bytes are intentionally not
//! yet promised as a stable ABI. Format minor bumps will preserve backward
//! read compatibility once they ship.
#![no_std]

#[cfg(kani)]
extern crate kani;

#[cfg(feature = "alloc")]
extern crate alloc;

mod container;
mod container_error;

#[cfg(kani)]
mod proofs;

#[cfg(feature = "alloc")]
pub use crate::container::{SectionSink, SnapshotWriter};
pub use crate::{
    container::{
        CRC32C_CHECK_INPUT, CRC32C_CHECK_VALUE, Checksum32, FORMAT_MAGIC, FORMAT_MAJOR,
        FORMAT_MINOR, HEADER_SIZE, HeaderOnlySnapshot, MAX_ALIGNMENT_LOG2, MAX_SECTION_COUNT,
        MAX_SUPPORTED_MINOR, PendingSection, SECTION_ENTRY_SIZE, Section, SectionIter, SectionKind,
        Snapshot, SnapshotPlan, ValidationLevel, kinds, patch_section_crc,
    },
    container_error::{PlanError, SectionBindError, SectionViewError, SnapshotError},
};
