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
//! Section payloads are exposed as byte slices via [`Section::bytes`]; the
//! [`Section::try_as_slice`] helper checks the actual payload pointer's
//! alignment against `align_of::<T>()` so consumers can reinterpret
//! `&[u8]` as `&[T]` only when it is safe to do so.
//!
//! # Cargo features
//!
//! - default: reader-only API; `no_std`, no allocation.
//! - `alloc`: enables [`SnapshotBuilder`], an owning builder that returns an encoded `Vec<u8>`.
//! - `std`: reserved for future mmap helpers; no effect in v1 beyond activating `alloc`.
//!
//! # Stability
//!
//! v1.0 is the first topology-agnostic format major; the bytes are
//! intentionally not yet promised as a stable ABI. Format minor bumps
//! will preserve backward read compatibility once they ship.
#![no_std]

#[cfg(kani)]
extern crate kani;

#[cfg(feature = "alloc")]
extern crate alloc;

mod container;

#[cfg(kani)]
mod proofs;

#[cfg(feature = "alloc")]
pub use crate::container::SnapshotBuilder;
pub use crate::container::{
    FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, HeaderOnlySnapshot, MAX_ALIGNMENT_LOG2,
    MAX_SECTION_COUNT, MAX_SUPPORTED_MINOR, PendingSection, PlanError, SECTION_ENTRY_SIZE, Section,
    SectionIter, SectionViewError, Snapshot, SnapshotError, SnapshotPlan, ValidationLevel,
};
