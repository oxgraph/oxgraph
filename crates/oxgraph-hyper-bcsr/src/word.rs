//! Borrowed-section word abstraction for bipartite-CSR payloads.

use core::marker::PhantomData;

use oxgraph_layout_util::SnapshotWidth;
pub use oxgraph_layout_util::{LayoutIndex, LayoutSnapshotWord, LayoutWord};

use crate::snapshot::{
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_BASE, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_BASE,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_BASE, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_BASE,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_BASE,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_BASE,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_BASE,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_BASE,
};

/// Section version written and expected for all BCSR layout payloads.
pub const SNAPSHOT_BCSR_SECTION_VERSION: u32 = 1;

/// Bundles the three index widths and three storage words of one BCSR view.
///
/// [`BcsrHypergraph`](crate::BcsrHypergraph) is generic over exactly one
/// `BcsrWords` family instead of six coupled type parameters. The three
/// `*Index` types are the logical dense widths; the three `*Word` types are
/// the in-memory representations stored in the borrowed sections, each
/// constrained to decode to its matching index. Two families exist:
/// [`NativeWords`] for host-order build-path views (including `usize`) and
/// [`LeWords`] for little-endian snapshot-backed views.
///
/// # Performance
///
/// `perf: unspecified`; this is a type-level trait with no methods.
pub trait BcsrWords {
    /// Logical vertex index width.
    type VertexIndex: LayoutIndex;
    /// Logical hyperedge (relation) index width.
    type RelationIndex: LayoutIndex;
    /// Logical incidence index width.
    type IncidenceIndex: LayoutIndex;
    /// Storage word of the four offset sections; decodes to
    /// [`Self::IncidenceIndex`].
    type OffsetWord: LayoutWord<Index = Self::IncidenceIndex>;
    /// Storage word of the hyperedge-major participant sections; decodes to
    /// [`Self::VertexIndex`].
    type VertexWord: LayoutWord<Index = Self::VertexIndex>;
    /// Storage word of the vertex-major hyperedge sections; decodes to
    /// [`Self::RelationIndex`].
    type RelationWord: LayoutWord<Index = Self::RelationIndex>;
}

/// Type-level brand carried by the word-family markers; the `fn() -> …`
/// wrapper keeps auto traits and variance independent of the index widths.
type WordFamilyBrand<VertexIndex, RelationIndex, IncidenceIndex> =
    PhantomData<fn() -> (VertexIndex, RelationIndex, IncidenceIndex)>;

/// Native host word family: sections store each index type directly.
///
/// Selects identity storage words (`OffsetWord = IncidenceIndex`, and so on),
/// which is the build-path representation. `usize` is permitted because
/// nothing is persisted. This is a type-level carrier — it is never
/// constructed.
///
/// # Performance
///
/// `perf: unspecified`; this is a type-level marker.
pub struct NativeWords<VertexIndex, RelationIndex, IncidenceIndex> {
    /// Type-level brand selecting the three index widths.
    _family: WordFamilyBrand<VertexIndex, RelationIndex, IncidenceIndex>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex> BcsrWords
    for NativeWords<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: LayoutIndex + LayoutWord<Index = VertexIndex>,
    RelationIndex: LayoutIndex + LayoutWord<Index = RelationIndex>,
    IncidenceIndex: LayoutIndex + LayoutWord<Index = IncidenceIndex>,
{
    type IncidenceIndex = IncidenceIndex;
    type OffsetWord = IncidenceIndex;
    type RelationIndex = RelationIndex;
    type RelationWord = RelationIndex;
    type VertexIndex = VertexIndex;
    type VertexWord = VertexIndex;
}

/// Little-endian snapshot word family: sections store fixed-width
/// little-endian words.
///
/// Selects each width's [`SnapshotWidth::LittleEndianWord`] as the storage
/// word, which is the view-path representation over persisted snapshot bytes.
/// `usize` is excluded because snapshot bytes are fixed-width. This is a
/// type-level carrier — it is never constructed.
///
/// # Performance
///
/// `perf: unspecified`; this is a type-level marker.
pub struct LeWords<VertexIndex, RelationIndex, IncidenceIndex> {
    /// Type-level brand selecting the three index widths.
    _family: WordFamilyBrand<VertexIndex, RelationIndex, IncidenceIndex>,
}

impl<VertexIndex, RelationIndex, IncidenceIndex> BcsrWords
    for LeWords<VertexIndex, RelationIndex, IncidenceIndex>
where
    VertexIndex: SnapshotWidth,
    RelationIndex: SnapshotWidth,
    IncidenceIndex: SnapshotWidth,
{
    type IncidenceIndex = IncidenceIndex;
    type OffsetWord = IncidenceIndex::LittleEndianWord;
    type RelationIndex = RelationIndex;
    type RelationWord = RelationIndex::LittleEndianWord;
    type VertexIndex = VertexIndex;
    type VertexWord = VertexIndex::LittleEndianWord;
}

/// Width-specific section-kind tags for persisted BCSR layout payloads.
///
/// This is the thin BCSR-specific layer over the shared
/// [`SnapshotWidth`](oxgraph_layout_util::SnapshotWidth) contract: it derives
/// the eight per-width section kinds from the crate's 4-aligned base constants
/// and [`SnapshotWidth::WIDTH_CODE`], and adds the section version. The
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
    const HEAD_OFFSETS_KIND: u32 = SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_BASE | Self::WIDTH_CODE;
    /// Head participants section kind for this width.
    const HEAD_PARTICIPANTS_KIND: u32 =
        SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_BASE | Self::WIDTH_CODE;
    /// Tail offsets section kind for this width.
    const TAIL_OFFSETS_KIND: u32 = SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_BASE | Self::WIDTH_CODE;
    /// Tail participants section kind for this width.
    const TAIL_PARTICIPANTS_KIND: u32 =
        SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_BASE | Self::WIDTH_CODE;
    /// Vertex outgoing offsets section kind for this width.
    const VERTEX_OUTGOING_OFFSETS_KIND: u32 =
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_BASE | Self::WIDTH_CODE;
    /// Vertex outgoing hyperedges section kind for this width.
    const VERTEX_OUTGOING_HYPEREDGES_KIND: u32 =
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_BASE | Self::WIDTH_CODE;
    /// Vertex incoming offsets section kind for this width.
    const VERTEX_INCOMING_OFFSETS_KIND: u32 =
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_BASE | Self::WIDTH_CODE;
    /// Vertex incoming hyperedges section kind for this width.
    const VERTEX_INCOMING_HYPEREDGES_KIND: u32 =
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_BASE | Self::WIDTH_CODE;
    /// Section version written for this width's BCSR payloads.
    const SECTION_VERSION: u32 = SNAPSHOT_BCSR_SECTION_VERSION;
}

impl BcsrSnapshotIndex for u16 {}
impl BcsrSnapshotIndex for u32 {}
impl BcsrSnapshotIndex for u64 {}
