//! Local identifier newtypes and role enum for bipartite-CSR hypergraph
//! views.

use oxgraph_layout_util::{HyperedgeAxis, IncidenceAxis, LocalId, VertexAxis};

/// Side of a directed hyperedge an incidence belongs to.
///
/// `oxgraph-hyper` keeps the participation role as an associated type rather
/// than a concrete enum so views can pick whatever vocabulary fits their
/// storage. `BcsrRole` is the role chosen for [`BcsrHypergraph`](crate::BcsrHypergraph):
/// each participant is either on the head side or the tail side of a directed
/// hyperedge. Role bytes are not stored — the role is recovered from which
/// section the participant lives in.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BcsrRole {
    /// The vertex participates on the source (head) side of a directed hyperedge.
    Head,
    /// The vertex participates on the target (tail) side of a directed hyperedge.
    Tail,
}

/// Local vertex ID for [`BcsrHypergraph`](crate::BcsrHypergraph).
///
/// Values are dense handles in `0..vertex_count` for one validated
/// view. They are layout-local IDs and are not stable across rebuilding or
/// compaction unless a higher layer defines that contract. This is an alias of
/// the shared [`LocalId`](oxgraph_layout_util::LocalId) branded by the vertex
/// axis, so a built hypergraph and its borrowed view yield the same handle
/// type.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
pub type BcsrVertexId<VertexIndex> = LocalId<VertexAxis, VertexIndex>;

/// Local hyperedge ID for [`BcsrHypergraph`](crate::BcsrHypergraph).
///
/// Values are dense handles in `0..hyperedge_count` for one validated
/// view. They are layout-local IDs and are not stable across rebuilding or
/// compaction unless a higher layer defines that contract. This is an alias of
/// the shared [`LocalId`](oxgraph_layout_util::LocalId) branded by the
/// hyperedge axis.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
pub type BcsrHyperedgeId<RelationIndex> = LocalId<HyperedgeAxis, RelationIndex>;

/// Local participant (incidence) ID for [`BcsrHypergraph`](crate::BcsrHypergraph).
///
/// Participant IDs span a single dense `u32` index space anchored on the
/// hyperedge-major arrays:
///
/// - `[0, P_head)` are head incidences; the value indexes `head_participants`.
/// - `[P_head, P_head + P_tail)` are tail incidences; subtracting `P_head` yields a position in
///   `tail_participants`.
///
/// They are layout-local IDs and are not stable across rebuilding or
/// compaction unless a higher layer defines that contract. This is an alias of
/// the shared [`LocalId`](oxgraph_layout_util::LocalId) branded by the
/// incidence axis.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
pub type BcsrParticipantId<IncidenceIndex> = LocalId<IncidenceAxis, IncidenceIndex>;
