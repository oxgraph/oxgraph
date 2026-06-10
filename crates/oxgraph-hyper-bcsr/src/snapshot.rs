//! Snapshot section-kind base constants for bipartite-CSR hypergraph layouts.
//!
//! Each constant is a 4-aligned base; the persisted kind of one fixed-width
//! little-endian section is `BASE | WIDTH_CODE`, where
//! [`SnapshotWidth::WIDTH_CODE`](oxgraph_layout_util::SnapshotWidth::WIDTH_CODE)
//! selects the word width in the low two bits (`0b00` = `u16`, `0b01` = `u32`,
//! `0b10` = `u64`). [`BcsrSnapshotIndex`](crate::BcsrSnapshotIndex) derives the
//! per-width kinds. Persisted BCSR snapshots do not have widthless topology
//! section kinds and never use `usize`.
//!
//! # Sections
//!
//! | Family | Base | Description |
//! | ------ | ---- | ----------- |
//! | `BCSR_HEAD_OFFSETS` | [`SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_BASE`] | Hyperedge-major head offsets |
//! | `BCSR_HEAD_PARTICIPANTS` | [`SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_BASE`] | Hyperedge-major head participants |
//! | `BCSR_TAIL_OFFSETS` | [`SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_BASE`] | Hyperedge-major tail offsets |
//! | `BCSR_TAIL_PARTICIPANTS` | [`SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_BASE`] | Hyperedge-major tail participants |
//! | `BCSR_VERTEX_OUTGOING_OFFSETS` | [`SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_BASE`] | Vertex-major outgoing offsets |
//! | `BCSR_VERTEX_OUTGOING_HYPEREDGES` | [`SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_BASE`] | Vertex-major outgoing hyperedge IDs |
//! | `BCSR_VERTEX_INCOMING_OFFSETS` | [`SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_BASE`] | Vertex-major incoming offsets |
//! | `BCSR_VERTEX_INCOMING_HYPEREDGES` | [`SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_BASE`] | Vertex-major incoming hyperedge IDs |
//!
//! The bases ascend by 4 in the exporter's emission order, so every derived
//! kind of one section sorts below every derived kind of the next section for
//! any vertex/relation/incidence width mix — the strictly-ascending order the
//! v2 container mandates. All constants are `perf: unspecified` — compile-time
//! `u32` section-kind tags.

/// 4-aligned base for hyperedge-major head offsets sections.
pub const SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_BASE: u32 = 0x0020;
/// 4-aligned base for hyperedge-major head participants sections.
pub const SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_BASE: u32 = 0x0024;
/// 4-aligned base for hyperedge-major tail offsets sections.
pub const SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_BASE: u32 = 0x0028;
/// 4-aligned base for hyperedge-major tail participants sections.
pub const SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_BASE: u32 = 0x002C;
/// 4-aligned base for vertex-major outgoing offsets sections.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_BASE: u32 = 0x0030;
/// 4-aligned base for vertex-major outgoing hyperedge ID sections.
pub const SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_BASE: u32 = 0x0034;
/// 4-aligned base for vertex-major incoming offsets sections.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_BASE: u32 = 0x0038;
/// 4-aligned base for vertex-major incoming hyperedge ID sections.
pub const SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_BASE: u32 = 0x003C;
