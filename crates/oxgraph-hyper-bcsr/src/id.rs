//! Local identifier newtypes and role enum for bipartite-CSR hypergraph
//! views.

use core::fmt;

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
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BcsrVertexId<VertexIndex>(pub VertexIndex);

impl<VertexIndex> fmt::Debug for BcsrVertexId<VertexIndex>
where
    VertexIndex: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BcsrVertexId")
            .field(&self.0)
            .finish()
    }
}

impl<VertexIndex> From<VertexIndex> for BcsrVertexId<VertexIndex> {
    fn from(value: VertexIndex) -> Self {
        Self(value)
    }
}

/// Local hyperedge ID for [`BcsrHypergraph`](crate::BcsrHypergraph).
///
/// Values are dense handles in `0..hyperedge_count` for one validated
/// view. They are layout-local IDs and are not stable across rebuilding or
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BcsrHyperedgeId<RelationIndex>(pub RelationIndex);

impl<RelationIndex> fmt::Debug for BcsrHyperedgeId<RelationIndex>
where
    RelationIndex: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BcsrHyperedgeId")
            .field(&self.0)
            .finish()
    }
}

impl<RelationIndex> From<RelationIndex> for BcsrHyperedgeId<RelationIndex> {
    fn from(value: RelationIndex) -> Self {
        Self(value)
    }
}

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
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BcsrParticipantId<IncidenceIndex>(pub IncidenceIndex);

impl<IncidenceIndex> fmt::Debug for BcsrParticipantId<IncidenceIndex>
where
    IncidenceIndex: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BcsrParticipantId")
            .field(&self.0)
            .finish()
    }
}
