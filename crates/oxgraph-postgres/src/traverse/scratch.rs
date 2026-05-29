//! Reusable BFS scratch storage allocated once per engine.
//!
//! Holds the dense mark array and frontier queue backing
//! [`oxgraph_algo::BfsEpochScratch`]. The engine owns the buffers across queries
//! and resizes them on snapshot replacement; each query borrows them to brand a
//! fresh `BfsEpochScratch` against the per-query overlay topology view.

use alloc::vec::Vec;

/// Per-engine BFS scratch buffers reused across queries.
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "shared with engine and builder modules"
)]
pub(crate) struct TraverseScratch {
    /// Dense epoch marks indexed by node id, sized to `node_count`.
    marks: Vec<u32>,
    /// Frontier queue storage, sized to at least `node_count`.
    queue: Vec<u32>,
}

impl TraverseScratch {
    /// Resizes scratch buffers to back a `node_count`-node traversal.
    ///
    /// # Performance
    ///
    /// This method is `O(n)` when growing, `O(1)` when size unchanged.
    pub(crate) fn resize_for_nodes(&mut self, node_count: usize) {
        self.marks.resize(node_count, 0);
        self.queue.resize(node_count, 0);
    }

    /// Clears scratch after snapshot replacement.
    ///
    /// # Performance
    ///
    /// This method is `O(n)` for `n = node_count`.
    pub(crate) fn reset_after_snapshot(&mut self, node_count: usize) {
        self.marks.clear();
        self.queue.clear();
        self.resize_for_nodes(node_count);
    }

    /// Borrows the disjoint mark and queue slices for one bounded BFS run.
    ///
    /// The caller resizes via [`Self::resize_for_nodes`] before borrowing so both
    /// slices cover `graph.element_bound()`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) fn bounded_slices(&mut self) -> (&mut [u32], &mut [u32]) {
        (&mut self.marks, &mut self.queue)
    }
}
