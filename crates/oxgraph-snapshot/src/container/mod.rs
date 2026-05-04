//! Topology-agnostic snapshot container modules and format constants.
//!
//! This module collects the byte-level format definition for the snapshot
//! container along with the reader, writer, and validator implementations.
//! All public types are re-exported through the crate root; consumers should
//! depend on the crate-level paths rather than reaching in here.
//!
//! When the container ever graduates to a separate `topology-snapshot` crate
//! the entire module tree moves wholesale, and the crate root becomes a
//! shim of `pub use topology_snapshot::*`.

mod error;
mod header;
mod plan;
mod reader;
mod section;
mod validate;

#[cfg(feature = "alloc")]
mod builder;

#[cfg(feature = "alloc")]
pub use self::builder::SnapshotBuilder;
pub use self::{
    error::{PlanError, SectionViewError, SnapshotError},
    plan::{PendingSection, SnapshotPlan},
    reader::{HeaderOnlySnapshot, SectionIter, Snapshot},
    section::Section,
    validate::ValidationLevel,
};

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
pub(in crate::container) const HEADER_SIZE_U32: u32 = 32;

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
pub(in crate::container) fn u64_to_usize_validated(value: u64) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(_error) => unreachable!("validated u64 must fit usize on this target"),
    }
}
