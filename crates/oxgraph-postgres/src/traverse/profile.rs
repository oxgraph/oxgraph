//! Per-query traversal profile resolved from the freshness GUC and overlay state.

use crate::{config::QueryFreshness, overlay::OverlayState, traverse::TraversalDirection};

/// Collect node ids in BFS first-discovery order, or count only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraverseMode {
    /// Push each first-discovered node into the result buffer (subject to `result_limit`).
    Collect,
    /// Count discoveries only (no output allocation).
    Count,
}

/// Neighbor-resolution policy selected once per query.
///
/// Encodes the GUC [`QueryFreshness`] decision into the two axes the overlay
/// topology views consume: whether to walk node-unique deduplicated rows
/// (`use_unique`, the `UniqueAdjacency` semantics) and whether to chain overlay
/// adjacency after the base row (`merge_overlay`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TraverseProfile {
    /// Expansion direction for this query.
    pub direction: TraversalDirection,
    /// Walk node-unique deduplicated base rows instead of the parallel CSR/CSC walk.
    pub use_unique: bool,
    /// Chain overlay adjacency after the base neighbors.
    pub merge_overlay: bool,
}

impl TraverseProfile {
    /// Resolves the profile for one query from freshness and overlay state.
    ///
    /// The policy is unchanged from the prior 4-variant dispatch: the unique
    /// path is taken only under [`QueryFreshness::BaseOnly`] with no active edge
    /// tombstones, and overlay adjacency is merged only under
    /// [`QueryFreshness::OverlayAware`] when overlay edges exist.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(super) fn resolve(
        direction: TraversalDirection,
        freshness: QueryFreshness,
        overlay: &OverlayState,
    ) -> Self {
        let merge_overlay =
            freshness == QueryFreshness::OverlayAware && overlay.overlay_edge_count() > 0;
        let use_unique = freshness == QueryFreshness::BaseOnly && !overlay.has_edge_tombstones();
        Self {
            direction,
            use_unique,
            merge_overlay,
        }
    }
}
