//! Borrowed-section parameter struct for opening a [`BcsrHypergraph`].
//!
//! [`BcsrHypergraph`]: crate::BcsrHypergraph

use crate::word::BcsrWord;

/// Borrowed input slices for a bipartite-CSR hypergraph view.
///
/// Offset slices use `OffsetWord` and decode to the incidence index width.
/// Participant value slices use `VertexWord` and decode to the vertex index
/// width. Vertex-major relation slices use `RelationWord` and decode to the
/// relation index width.
///
/// # Performance
///
/// `perf: unspecified`; this is a borrowed parameter struct.
#[derive(Clone, Copy, Debug)]
pub struct BcsrSections<'view, OffsetWord, VertexWord, RelationWord>
where
    OffsetWord: BcsrWord,
    VertexWord: BcsrWord,
    RelationWord: BcsrWord,
{
    /// Hyperedge-major head offsets, length `hyperedge_count + 1`.
    pub head_offsets: &'view [OffsetWord],
    /// Flat vertex IDs in head sets, length `P_head`.
    pub head_participants: &'view [VertexWord],
    /// Hyperedge-major tail offsets, length `hyperedge_count + 1`.
    pub tail_offsets: &'view [OffsetWord],
    /// Flat vertex IDs in tail sets, length `P_tail`.
    pub tail_participants: &'view [VertexWord],
    /// Vertex-major outgoing offsets, length `vertex_count + 1`.
    pub vertex_outgoing_offsets: &'view [OffsetWord],
    /// Flat hyperedge IDs where the vertex is in head, length `P_outgoing`.
    pub vertex_outgoing_hyperedges: &'view [RelationWord],
    /// Vertex-major incoming offsets, length `vertex_count + 1`.
    pub vertex_incoming_offsets: &'view [OffsetWord],
    /// Flat hyperedge IDs where the vertex is in tail, length `P_incoming`.
    pub vertex_incoming_hyperedges: &'view [RelationWord],
}
