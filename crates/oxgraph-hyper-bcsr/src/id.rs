//! Local identifier newtypes for bipartite-CSR hypergraph views.

use core::fmt;

/// Local vertex ID for [`BcsrHypergraph`](crate::BcsrHypergraph).
///
/// Values are dense `u32` handles in `0..vertex_count` for one validated
/// view. They are layout-local IDs and are not stable across rebuilding or
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BcsrVertexId(pub u32);

impl fmt::Debug for BcsrVertexId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BcsrVertexId")
            .field(&self.0)
            .finish()
    }
}

/// Local hyperedge ID for [`BcsrHypergraph`](crate::BcsrHypergraph).
///
/// Values are dense `u32` handles in `0..hyperedge_count` for one validated
/// view. They are layout-local IDs and are not stable across rebuilding or
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BcsrHyperedgeId(pub u32);

impl fmt::Debug for BcsrHyperedgeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BcsrHyperedgeId")
            .field(&self.0)
            .finish()
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
pub struct BcsrParticipantId(pub u32);

impl fmt::Debug for BcsrParticipantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BcsrParticipantId")
            .field(&self.0)
            .finish()
    }
}
