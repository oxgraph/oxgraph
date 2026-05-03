//! Snapshot section kind constants for bipartite-CSR hypergraph layouts.
//!
//! Each constant identifies one of the eight u32 little-endian sections that
//! together form a valid bipartite-CSR snapshot. Section kinds in the
//! `0x0010..=0x0017` range are reserved for the v1.0 (u32-offset) layout;
//! `0x0018..=0x001F` is reserved for future u64-offset variants.

/// Hyperedge-major head offsets section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `hyperedge_count + 1`. Position `h` and `h + 1` enclose the contiguous
/// range of vertex IDs in the head set of hyperedge `h`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_HEAD_OFFSETS: u32 = 0x0010;

/// Hyperedge-major head participants section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `P_head`, the total number of head-side participations across all
/// hyperedges. Each value is a vertex ID in `0..vertex_count`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS: u32 = 0x0011;

/// Hyperedge-major tail offsets section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `hyperedge_count + 1`. Position `h` and `h + 1` enclose the contiguous
/// range of vertex IDs in the tail set of hyperedge `h`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_TAIL_OFFSETS: u32 = 0x0012;

/// Hyperedge-major tail participants section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `P_tail`, the total number of tail-side participations across all
/// hyperedges. Each value is a vertex ID in `0..vertex_count`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS: u32 = 0x0013;

/// Vertex-major outgoing offsets section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `vertex_count + 1`. Position `v` and `v + 1` enclose the contiguous range
/// of hyperedge IDs in which vertex `v` participates as a head.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS: u32 = 0x0014;

/// Vertex-major outgoing hyperedge IDs section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `P_outgoing`. Each value is a hyperedge ID in `0..hyperedge_count`. The
/// total length must equal `P_head` (every head-side participation is
/// recorded once on each side of the bipartite index).
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES: u32 = 0x0015;

/// Vertex-major incoming offsets section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `vertex_count + 1`. Position `v` and `v + 1` enclose the contiguous range
/// of hyperedge IDs in which vertex `v` participates as a tail.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS: u32 = 0x0016;

/// Vertex-major incoming hyperedge IDs section.
///
/// Payload is a sequence of unaligned little-endian `u32` words of length
/// `P_incoming`. Each value is a hyperedge ID in `0..hyperedge_count`. The
/// total length must equal `P_tail`.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES: u32 = 0x0017;
