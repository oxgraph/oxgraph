//! Compile-time traversal profile axes (direction, adjacency shape, overlay).

use crate::{config::QueryFreshness, overlay::OverlayState, traverse::TraversalDirection};

/// Collect node ids in BFS first-discovery order, or count only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraverseMode {
    /// Push each first-discovered node into scratch output (subject to `result_limit`).
    Collect,
    /// Count discoveries only (no output allocation).
    Count,
}

/// Neighbor iteration strategy selected once per query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TraverseProfile {
    /// Node-unique outgoing rows (`BaseOnly`, no edge tombstones).
    OutUnique {
        /// Whether overlay outgoing targets are merged after the base row.
        overlay: bool,
    },
    /// Parallel CSR outgoing slots with per-edge tombstone checks.
    OutParallel {
        /// Whether overlay outgoing targets are merged after the base walk.
        overlay: bool,
    },
    /// Node-unique incoming rows (`BaseOnly`, no edge tombstones).
    InUnique {
        /// Whether overlay incoming sources are merged after the base row.
        overlay: bool,
    },
    /// Incoming predecessors with forward-edge tombstone filtering (documented slow path).
    InParallel {
        /// Whether overlay incoming sources are merged after the base walk.
        overlay: bool,
    },
}

impl TraverseProfile {
    /// Resolves the profile for one query from freshness and overlay state.
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
        let overlay_active =
            freshness == QueryFreshness::OverlayAware && overlay.overlay_edge_count() > 0;
        let use_unique = freshness == QueryFreshness::BaseOnly && !overlay.has_edge_tombstones();
        if use_unique {
            match direction {
                TraversalDirection::Out => Self::OutUnique {
                    overlay: overlay_active,
                },
                TraversalDirection::In => Self::InUnique {
                    overlay: overlay_active,
                },
            }
        } else {
            match direction {
                TraversalDirection::Out => Self::OutParallel {
                    overlay: overlay_active,
                },
                TraversalDirection::In => Self::InParallel {
                    overlay: overlay_active,
                },
            }
        }
    }
}
