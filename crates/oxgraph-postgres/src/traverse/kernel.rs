//! Bounded BFS kernel driving the substrate algo over overlay topology views.
//!
//! The engine no longer hand-rolls epoch BFS. Each query resolves a
//! [`TraverseProfile`](super::profile::TraverseProfile), builds the matching
//! overlay topology view ([`OverlayOutView`] / [`OverlayInView`]), and calls
//! [`breadth_first_search_bounded`] / [`reverse_breadth_first_search_bounded`]
//! with a visitor that collects or counts discoveries.

use alloc::vec::Vec;
use core::ops::ControlFlow;

use oxgraph_algo::{
    BfsBounds, BfsEpochScratch, breadth_first_search_bounded, reverse_breadth_first_search_bounded,
};

use super::{profile::TraverseMode, session::TraverseSession};
use crate::{
    engine::Engine,
    error::QueryError,
    topology::{OverlayInView, OverlayOutView, OverlayViewFlags},
    traverse::TraversalDirection,
};

/// Result of [`run_bfs_multi`].
pub(super) enum KernelOutcome {
    /// Collected node ids in BFS first-discovery order.
    Nodes(Vec<u32>),
    /// Visited-node cardinality.
    Count(usize),
}

impl KernelOutcome {
    /// Empty outcome for invisible or out-of-bounds seeds.
    const fn empty(mode: TraverseMode) -> Self {
        match mode {
            TraverseMode::Collect => Self::Nodes(Vec::new()),
            TraverseMode::Count => Self::Count(0),
        }
    }
}

/// Discovery sink shared by collect and count modes.
///
/// Collect pushes each first-discovered node into `nodes`; count tracks the
/// running cardinality. The substrate BFS enforces `result_limit` and depth
/// bounds itself, so the visitor never returns early on its own.
struct OutcomeVisitor {
    /// Collected node ids (empty in count mode).
    nodes: Vec<u32>,
    /// Running discovery count (matches `nodes.len()` in collect mode).
    count: usize,
    /// Whether collected node ids are retained.
    collect: bool,
}

impl OutcomeVisitor {
    /// Records one discovered node and continues the traversal.
    fn observe(&mut self, node: u32) -> ControlFlow<()> {
        self.count += 1;
        if self.collect {
            self.nodes.push(node);
        }
        ControlFlow::Continue(())
    }

    /// Finalizes the visitor into the kernel outcome for `mode`.
    fn finish(self, mode: TraverseMode) -> KernelOutcome {
        match mode {
            TraverseMode::Collect => KernelOutcome::Nodes(self.nodes),
            TraverseMode::Count => KernelOutcome::Count(self.count),
        }
    }
}

/// Runs the bounded BFS for the open session through the substrate algo.
///
/// # Errors
///
/// Returns [`QueryError::InternalInvariant`] only if the substrate BFS rejects
/// engine-owned scratch or seeds, which the session and scratch sizing
/// guarantee cannot happen for a validated engine.
///
/// # Performance
///
/// `O(n + e)` bounded by session limits over the visited subgraph; per-query
/// scratch reuse plus a single `O(n)` mark clear when the epoch is rebranded.
pub(super) fn run_bfs_multi(
    engine: &mut Engine,
    session: &TraverseSession,
) -> Result<KernelOutcome, crate::error::PostgresGraphError> {
    if session.seeds.is_empty() {
        return Ok(KernelOutcome::empty(session.mode));
    }

    let bounds = BfsBounds {
        max_depth: session.max_depth,
        result_limit: session.result_limit,
        include_seeds: true,
    };
    let flags = OverlayViewFlags {
        use_unique: session.profile.use_unique,
        merge_overlay: session.profile.merge_overlay,
        check_nodes: session.check_nodes,
    };
    let node_count = session.node_count as usize;
    let mut visitor = OutcomeVisitor {
        nodes: Vec::new(),
        count: 0,
        collect: session.mode == TraverseMode::Collect,
    };

    let (topology, unique, overlay, scratch) = engine.traverse_workspace_mut(flags.use_unique);
    let (marks, queue) = scratch.bounded_slices();

    let result = match session.profile.direction {
        TraversalDirection::Out => {
            let view = OverlayOutView::new(topology, overlay, unique, flags, node_count);
            let mut bfs_scratch = BfsEpochScratch::new(marks, queue);
            breadth_first_search_bounded(
                &view,
                &session.seeds,
                bounds,
                &mut bfs_scratch,
                &mut |node, _depth| visitor.observe(node),
            )
        }
        TraversalDirection::In => {
            let view = OverlayInView::new(topology, overlay, unique, flags, node_count);
            let mut bfs_scratch = BfsEpochScratch::new(marks, queue);
            reverse_breadth_first_search_bounded(
                &view,
                &session.seeds,
                bounds,
                &mut bfs_scratch,
                &mut |node, _depth| visitor.observe(node),
            )
        }
    };
    if result.is_err() {
        return Err(QueryError::InternalInvariant("bounded BFS rejected engine scratch").into());
    }
    Ok(visitor.finish(session.mode))
}
