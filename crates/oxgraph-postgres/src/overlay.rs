//! Overlay buffers and tombstones layered on immutable base snapshots.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

/// Edge insertion applied on top of the frozen artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OverlayEdge {
    /// Source node id in dense index space.
    pub source: u32,
    /// Target node id in dense index space.
    pub target: u32,
}

/// Mutable overlay state for sync replay and query freshness.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlayState {
    /// Edges inserted after the base artifact was published.
    pub added_edges: Vec<OverlayEdge>,
    /// Overlay targets grouped by source for `O(1)` forward expansion lookup.
    by_source: BTreeMap<u32, Vec<u32>>,
    /// Overlay sources grouped by target for `O(1)` inbound expansion lookup.
    by_target: BTreeMap<u32, Vec<u32>>,
    /// Base edge ids tombstoned by sync events.
    pub tombstoned_edges: BTreeSet<u32>,
    /// Node ids hidden from traversal results.
    pub tombstoned_nodes: BTreeSet<u32>,
}

impl OverlayState {
    /// Clears all overlay buffers and adjacency indexes.
    pub fn clear(&mut self) {
        self.added_edges.clear();
        self.by_source.clear();
        self.by_target.clear();
        self.tombstoned_edges.clear();
        self.tombstoned_nodes.clear();
    }

    /// Returns whether any base edge tombstones are recorded.
    #[must_use]
    pub fn has_edge_tombstones(&self) -> bool {
        !self.tombstoned_edges.is_empty()
    }

    /// Returns whether any node tombstones are recorded.
    #[must_use]
    pub fn has_node_tombstones(&self) -> bool {
        !self.tombstoned_nodes.is_empty()
    }

    /// Returns whether an edge id is visible to traversal.
    ///
    /// # Performance
    ///
    /// This method is `O(log t)` for `t` tombstoned edge ids.
    #[must_use]
    pub fn edge_visible(&self, edge_id: u32) -> bool {
        !self.tombstoned_edges.contains(&edge_id)
    }

    /// Returns whether a node id is visible to traversal.
    ///
    /// # Performance
    ///
    /// This method is `O(log t)` for `t` tombstoned node ids.
    #[must_use]
    pub fn node_visible(&self, node_id: u32) -> bool {
        !self.tombstoned_nodes.contains(&node_id)
    }

    /// Records an overlay edge and updates adjacency indexes.
    ///
    /// # Performance
    ///
    /// This method is `O(log s + log t)` for index map sizes `s` and `t`.
    pub fn push_edge(&mut self, edge: OverlayEdge) {
        self.added_edges.push(edge);
        self.by_source
            .entry(edge.source)
            .or_default()
            .push(edge.target);
        self.by_target
            .entry(edge.target)
            .or_default()
            .push(edge.source);
    }

    /// Returns overlay outgoing targets for `source` (empty when none).
    ///
    /// # Performance
    ///
    /// This method is `O(log s)` for `s` indexed sources.
    #[must_use]
    pub fn overlay_targets(&self, source: u32) -> &[u32] {
        self.by_source
            .get(&source)
            .map_or(&[] as &[u32], Vec::as_slice)
    }

    /// Returns overlay incoming sources for `target` (empty when none).
    ///
    /// # Performance
    ///
    /// This method is `O(log t)` for `t` indexed targets.
    #[must_use]
    pub fn overlay_sources(&self, target: u32) -> &[u32] {
        self.by_target
            .get(&target)
            .map_or(&[] as &[u32], Vec::as_slice)
    }

    /// Removes one overlay edge and rebuilds adjacency indexes.
    ///
    /// # Performance
    ///
    /// This method is `O(a)` for `a` overlay edges.
    pub fn remove_edge(&mut self, source: u32, target: u32) {
        self.added_edges
            .retain(|edge| edge.source != source || edge.target != target);
        self.rebuild_indexes();
    }

    /// Rebuilds `by_source` / `by_target` from `added_edges`.
    ///
    /// # Performance
    ///
    /// This method is `O(a)` for `a` overlay edges.
    pub fn rebuild_indexes(&mut self) {
        self.by_source.clear();
        self.by_target.clear();
        for edge in &self.added_edges {
            self.by_source
                .entry(edge.source)
                .or_default()
                .push(edge.target);
            self.by_target
                .entry(edge.target)
                .or_default()
                .push(edge.source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OverlayEdge, OverlayState};

    #[test]
    fn indexes_match_linear_scan() {
        let mut overlay = OverlayState::default();
        overlay.push_edge(OverlayEdge {
            source: 1,
            target: 2,
        });
        overlay.push_edge(OverlayEdge {
            source: 1,
            target: 3,
        });
        overlay.push_edge(OverlayEdge {
            source: 4,
            target: 1,
        });
        assert_eq!(overlay.overlay_targets(1), &[2, 3]);
        assert_eq!(overlay.overlay_sources(1), &[4]);
        overlay.clear();
        assert!(overlay.overlay_targets(1).is_empty());
    }

    #[test]
    fn rebuild_indexes_after_manual_edges() {
        let mut overlay = OverlayState {
            added_edges: vec![
                OverlayEdge {
                    source: 0,
                    target: 1,
                },
                OverlayEdge {
                    source: 2,
                    target: 0,
                },
            ],
            ..OverlayState::default()
        };
        overlay.rebuild_indexes();
        assert_eq!(overlay.overlay_targets(0), &[1]);
        assert_eq!(overlay.overlay_sources(0), &[2]);
    }
}
