//! Role enum for bipartite-CSR participant entries.

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
