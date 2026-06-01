//! Borrowed-section word abstraction for bipartite-CSR payloads.

use oxgraph_layout_util::SnapshotWidth;
pub use oxgraph_layout_util::{LayoutIndex, LayoutSnapshotWord, LayoutWord};

use crate::snapshot::{
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U16, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U16, SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U16, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U16, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64,
};

/// Section version written and expected for all BCSR layout payloads.
pub const SNAPSHOT_BCSR_SECTION_VERSION: u32 = 1;

/// Width-specific section-kind tags for persisted BCSR layout payloads.
///
/// This is the thin BCSR-specific layer over the shared
/// [`SnapshotWidth`](oxgraph_layout_util::SnapshotWidth) contract: it adds only
/// the eight per-width section-kind constants and the section version. The
/// little-endian storage word and the native/LE conversions come from
/// `SnapshotWidth`, so `Index::LittleEndianWord` and `Index::to_le_word` keep
/// resolving through that trait.
///
/// `usize` deliberately does not implement this trait: snapshot bytes are
/// fixed-width.
///
/// # Performance
///
/// Reading the kind/version constants is `O(1)`.
pub trait BcsrSnapshotIndex: SnapshotWidth {
    /// Head offsets section kind for this width.
    const HEAD_OFFSETS_KIND: u32;
    /// Head participants section kind for this width.
    const HEAD_PARTICIPANTS_KIND: u32;
    /// Tail offsets section kind for this width.
    const TAIL_OFFSETS_KIND: u32;
    /// Tail participants section kind for this width.
    const TAIL_PARTICIPANTS_KIND: u32;
    /// Vertex outgoing offsets section kind for this width.
    const VERTEX_OUTGOING_OFFSETS_KIND: u32;
    /// Vertex outgoing hyperedges section kind for this width.
    const VERTEX_OUTGOING_HYPEREDGES_KIND: u32;
    /// Vertex incoming offsets section kind for this width.
    const VERTEX_INCOMING_OFFSETS_KIND: u32;
    /// Vertex incoming hyperedges section kind for this width.
    const VERTEX_INCOMING_HYPEREDGES_KIND: u32;
    /// Section version written for this width's BCSR payloads.
    const SECTION_VERSION: u32;
}

/// Implements [`BcsrSnapshotIndex`] for one portable snapshot width.
macro_rules! impl_bcsr_snapshot_index {
    (
        $index:ty,
        $head_offsets:expr,
        $head_participants:expr,
        $tail_offsets:expr,
        $tail_participants:expr,
        $vertex_outgoing_offsets:expr,
        $vertex_outgoing_hyperedges:expr,
        $vertex_incoming_offsets:expr,
        $vertex_incoming_hyperedges:expr
    ) => {
        impl BcsrSnapshotIndex for $index {
            const HEAD_OFFSETS_KIND: u32 = $head_offsets;
            const HEAD_PARTICIPANTS_KIND: u32 = $head_participants;
            const TAIL_OFFSETS_KIND: u32 = $tail_offsets;
            const TAIL_PARTICIPANTS_KIND: u32 = $tail_participants;
            const VERTEX_OUTGOING_OFFSETS_KIND: u32 = $vertex_outgoing_offsets;
            const VERTEX_OUTGOING_HYPEREDGES_KIND: u32 = $vertex_outgoing_hyperedges;
            const VERTEX_INCOMING_OFFSETS_KIND: u32 = $vertex_incoming_offsets;
            const VERTEX_INCOMING_HYPEREDGES_KIND: u32 = $vertex_incoming_hyperedges;
            const SECTION_VERSION: u32 = SNAPSHOT_BCSR_SECTION_VERSION;
        }
    };
}

impl_bcsr_snapshot_index!(
    u16,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U16
);
impl_bcsr_snapshot_index!(
    u32,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32
);
impl_bcsr_snapshot_index!(
    u64,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U64
);
