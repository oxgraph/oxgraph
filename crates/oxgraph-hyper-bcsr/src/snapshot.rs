//! Snapshot section kind constants for bipartite-CSR hypergraph layouts.
//!
//! Each constant identifies one fixed-width little-endian section. The width
//! suffix is part of the wire contract; persisted BCSR snapshots do not have
//! widthless topology section kinds and never use `usize`.
//!
//! # Sections
//!
//! | Family | Description |
//! | ------ | ----------- |
//! | `BCSR_HEAD_OFFSETS_*` | Hyperedge-major head offsets |
//! | `BCSR_HEAD_PARTICIPANTS_*` | Hyperedge-major head participants |
//! | `BCSR_TAIL_OFFSETS_*` | Hyperedge-major tail offsets |
//! | `BCSR_TAIL_PARTICIPANTS_*` | Hyperedge-major tail participants |
//! | `BCSR_VERTEX_OUTGOING_OFFSETS_*` | Vertex-major outgoing offsets |
//! | `BCSR_VERTEX_OUTGOING_HYPEREDGES_*` | Vertex-major outgoing hyperedge IDs |
//! | `BCSR_VERTEX_INCOMING_OFFSETS_*` | Vertex-major incoming offsets |
//! | `BCSR_VERTEX_INCOMING_HYPEREDGES_*` | Vertex-major incoming hyperedge IDs |
//!
//! The `_U16` / `_U32` / `_U64` suffix selects the little-endian word width
//! used to store each entry. All constants are `perf: unspecified` —
//! compile-time `u32` section-kind tags.

/// Hyperedge-major head offsets, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U16: u32 = 0x0020;
/// Hyperedge-major head offsets, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32: u32 = 0x0021;
/// Hyperedge-major head offsets, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64: u32 = 0x0022;

/// Hyperedge-major head participants, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U16: u32 = 0x0023;
/// Hyperedge-major head participants, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32: u32 = 0x0024;
/// Hyperedge-major head participants, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U64: u32 = 0x0025;

/// Hyperedge-major tail offsets, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U16: u32 = 0x0026;
/// Hyperedge-major tail offsets, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32: u32 = 0x0027;
/// Hyperedge-major tail offsets, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64: u32 = 0x0028;

/// Hyperedge-major tail participants, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16: u32 = 0x0029;
/// Hyperedge-major tail participants, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32: u32 = 0x002A;
/// Hyperedge-major tail participants, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U64: u32 = 0x002B;

/// Vertex-major outgoing offsets, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U16: u32 = 0x002C;
/// Vertex-major outgoing offsets, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32: u32 = 0x002D;
/// Vertex-major outgoing offsets, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64: u32 = 0x002E;

/// Vertex-major outgoing hyperedge IDs, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U16: u32 = 0x002F;
/// Vertex-major outgoing hyperedge IDs, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32: u32 = 0x0030;
/// Vertex-major outgoing hyperedge IDs, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U64: u32 = 0x0031;

/// Vertex-major incoming offsets, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U16: u32 = 0x0032;
/// Vertex-major incoming offsets, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32: u32 = 0x0033;
/// Vertex-major incoming offsets, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64: u32 = 0x0034;

/// Vertex-major incoming hyperedge IDs, `u16` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U16: u32 = 0x0035;
/// Vertex-major incoming hyperedge IDs, `u32` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32: u32 = 0x0036;
/// Vertex-major incoming hyperedge IDs, `u64` little-endian.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U64: u32 = 0x0037;
