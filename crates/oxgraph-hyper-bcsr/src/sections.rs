//! Borrowed-section parameter struct for opening a [`BcsrHypergraph`].
//!
//! [`BcsrHypergraph`]: crate::BcsrHypergraph

use crate::word::BcsrWord;

/// Borrowed input slices for a bipartite-CSR hypergraph view.
///
/// Eight slices are required: a head/tail offset and value pair per
/// hyperedge-major direction, and an outgoing/incoming offset and value pair
/// per vertex-major direction. Slices may come from in-memory `&[u32]`
/// arrays (tests, examples) or from a validated [`Snapshot`] borrowing
/// `&[U32<LE>]` payloads.
///
/// `BcsrSections` is the single argument to
/// [`BcsrHypergraph::open`](crate::BcsrHypergraph::open) and
/// [`BcsrHypergraph::open_with`](crate::BcsrHypergraph::open_with). Bundling
/// the eight slices into one parameter keeps each constructor at a single
/// argument under the workspace's `too_many_arguments` lint.
///
/// [`Snapshot`]: oxgraph_snapshot::Snapshot
///
/// # Performance
///
/// `perf: unspecified`; this is a borrowed parameter struct.
#[derive(Clone, Copy, Debug)]
pub struct BcsrSections<'view, Word: BcsrWord> {
    /// Hyperedge-major head offsets, length `hyperedge_count + 1`.
    pub head_offsets: &'view [Word],
    /// Flat vertex IDs in head sets, length `P_head`.
    pub head_participants: &'view [Word],
    /// Hyperedge-major tail offsets, length `hyperedge_count + 1`.
    pub tail_offsets: &'view [Word],
    /// Flat vertex IDs in tail sets, length `P_tail`.
    pub tail_participants: &'view [Word],
    /// Vertex-major outgoing offsets, length `vertex_count + 1`.
    pub vertex_outgoing_offsets: &'view [Word],
    /// Flat hyperedge IDs where the vertex is in head, length `P_outgoing`.
    pub vertex_outgoing_hyperedges: &'view [Word],
    /// Vertex-major incoming offsets, length `vertex_count + 1`.
    pub vertex_incoming_offsets: &'view [Word],
    /// Flat hyperedge IDs where the vertex is in tail, length `P_incoming`.
    pub vertex_incoming_hyperedges: &'view [Word],
}
