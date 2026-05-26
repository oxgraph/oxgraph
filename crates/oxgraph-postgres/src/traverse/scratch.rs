//! Reusable traversal scratch allocated once per engine.

use alloc::vec::Vec;

/// Per-engine BFS scratch reused across queries (epoch marks, dual frontiers, collect buffer).
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "shared with engine and builder modules"
)]
pub(crate) struct TraverseScratch {
    /// Dense epoch marks indexed by node id (`0` = never visited this engine).
    marks: Vec<u32>,
    /// Current frontier wave.
    frontier: Vec<u32>,
    /// Next frontier wave (swapped with [`Self::frontier`] each hop).
    next: Vec<u32>,
    /// Reused collect output buffer (truncated per collect query).
    output: Vec<u32>,
    /// Current traversal epoch (advanced once per query).
    epoch: u32,
}

#[expect(
    clippy::missing_const_for_fn,
    reason = "Vec mutation and borrows are not const-compatible"
)]
impl TraverseScratch {
    /// Resizes scratch buffers to `node_count` nodes.
    ///
    /// # Performance
    ///
    /// This method is `O(n)` when growing, `O(1)` when size unchanged.
    pub(crate) fn resize_for_nodes(&mut self, node_count: usize) {
        self.marks.resize(node_count, 0);
        if self.frontier.capacity() < node_count {
            self.frontier.reserve(node_count);
        }
        if self.next.capacity() < node_count {
            self.next.reserve(node_count);
        }
    }

    /// Clears scratch after snapshot replacement (marks reset).
    ///
    /// # Performance
    ///
    /// This method is `O(n)` for `n = marks.len()`.
    pub(crate) fn reset_after_snapshot(&mut self, node_count: usize) {
        self.resize_for_nodes(node_count);
        self.marks.fill(0);
        self.frontier.clear();
        self.next.clear();
        self.output.clear();
        self.epoch = 0;
    }

    /// Advances to the next traversal epoch and returns it.
    ///
    /// # Performance
    ///
    /// `O(1)` except on `u32` overflow, where marks are cleared in `O(n)`.
    pub(crate) fn bump_epoch(&mut self) -> u32 {
        if self.epoch == u32::MAX {
            self.marks.fill(0);
            self.epoch = 1;
        } else {
            self.epoch += 1;
        }
        self.epoch
    }

    /// Marks `node` visited for `epoch` when not already visited in `epoch`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` with a single marks-array access.
    #[must_use]
    pub(crate) fn try_mark_visited(&mut self, node: u32, epoch: u32) -> bool {
        let Some(slot) = self.marks.get_mut(node as usize) else {
            return false;
        };
        if *slot == epoch {
            return false;
        }
        *slot = epoch;
        true
    }

    /// Borrows the current BFS frontier wave.
    pub(crate) fn frontier(&self) -> &[u32] {
        &self.frontier
    }

    /// Borrows the mutable current frontier.
    pub(crate) fn frontier_mut(&mut self) -> &mut Vec<u32> {
        &mut self.frontier
    }

    /// Borrows the mutable next frontier.
    pub(crate) fn next_mut(&mut self) -> &mut Vec<u32> {
        &mut self.next
    }

    /// Swaps current and next frontier vectors after a hop.
    pub(crate) fn swap_frontiers(&mut self) {
        core::mem::swap(&mut self.frontier, &mut self.next);
    }

    /// Clears the next frontier after a hop.
    pub(crate) fn clear_next(&mut self) {
        self.next.clear();
    }

    /// Borrows the reusable collect output buffer.
    pub(crate) fn output_mut(&mut self) -> &mut Vec<u32> {
        &mut self.output
    }
}
