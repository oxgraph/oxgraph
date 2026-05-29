//! Single depth-limited BFS kernel for collect and count modes.

use alloc::vec::Vec;

use oxgraph_csr::CsrNodeId;
use oxgraph_graph::{EdgeTargetGraph, OutgoingGraph};

use super::{
    profile::{TraverseMode, TraverseProfile},
    scratch::TraverseScratch,
    session::TraverseSession,
};
use crate::{engine::Engine, overlay::OverlayState, topology::ForwardCsr};

/// Mutable per-query discovery state threaded through neighbor expansion.
struct DiscoverCtx<'a, 'b> {
    /// Query parameters.
    session: &'a TraverseSession,
    /// Overlay tombstones and inserts.
    overlay: &'a OverlayState,
    /// Engine scratch buffers.
    scratch: &'a mut TraverseScratch,
    /// Active traversal epoch.
    epoch: u32,
    /// Whether node tombstones must be checked.
    check_nodes: bool,
    /// Whether discovered nodes may be enqueued on the next wave.
    enqueue_next: bool,
    /// Running visited count.
    visited_count: &'b mut usize,
    /// Collect output length (collect mode only).
    output_len: &'b mut usize,
}

/// Runs the profile-dispatched BFS kernel for the open session.
///
/// # Performance
///
/// `O(n + m + overlay_touch)` bounded by session limits; scratch reuse is `O(1)` per query
/// except epoch overflow (`O(n)` mark clear).
pub(super) fn run_bfs_multi(engine: &mut Engine, session: &TraverseSession) -> KernelOutcome {
    if session.seeds.is_empty() {
        return KernelOutcome::empty(session.mode);
    }

    let (hot, unique, overlay, scratch) = engine.traverse_workspace_mut();
    let unique = &*unique;
    let check_nodes = session.check_nodes;

    let epoch = scratch.bump_epoch();
    scratch.frontier_mut().clear();
    scratch.clear_next();
    scratch.output_mut().clear();

    let mut visited_count = 0_usize;
    let mut output_len = 0_usize;

    for seed in &session.seeds {
        if visited_count >= session.result_limit {
            break;
        }
        if scratch.try_mark_visited(*seed, epoch) {
            visited_count += 1;
            if session.mode == TraverseMode::Collect {
                scratch.output_mut().push(*seed);
                output_len = scratch.output_mut().len();
            }
            if visited_count < session.result_limit {
                scratch.frontier_mut().push(*seed);
            }
        }
    }

    if visited_count == 0 {
        return KernelOutcome::empty(session.mode);
    }

    if visited_count >= session.result_limit {
        return finish(session.mode, visited_count, scratch);
    }

    let max_depth = session.max_depth;
    let mut wave_depth = 0_u32;

    while !scratch.frontier().is_empty() {
        if max_depth.is_some_and(|bound| wave_depth >= bound) {
            break;
        }
        let enqueue_next = max_depth.is_none_or(|bound| wave_depth.saturating_add(1) < bound);
        let frontier_len = scratch.frontier().len();
        for i in 0..frontier_len {
            let current = scratch.frontier()[i];
            if stop_at_limit(
                session.mode,
                visited_count,
                output_len,
                session.result_limit,
            ) {
                return finish(session.mode, visited_count, scratch);
            }
            let mut ctx = DiscoverCtx {
                session,
                overlay,
                scratch,
                epoch,
                check_nodes,
                enqueue_next,
                visited_count: &mut visited_count,
                output_len: &mut output_len,
            };
            if expand_node(&mut ctx, hot, unique, current) {
                return finish(session.mode, visited_count, scratch);
            }
        }
        wave_depth = wave_depth.saturating_add(1);
        scratch.swap_frontiers();
        scratch.clear_next();
    }

    finish(session.mode, visited_count, scratch)
}

/// Result of [`run_bfs`].
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

/// Finalizes kernel output from scratch and running count.
fn finish(mode: TraverseMode, count: usize, scratch: &mut TraverseScratch) -> KernelOutcome {
    match mode {
        TraverseMode::Collect => {
            let mut out = Vec::new();
            out.extend_from_slice(scratch.output_mut());
            KernelOutcome::Nodes(out)
        }
        TraverseMode::Count => KernelOutcome::Count(count),
    }
}

/// Returns whether the query hit `result_limit`.
const fn stop_at_limit(
    mode: TraverseMode,
    visited_count: usize,
    output_len: usize,
    limit: usize,
) -> bool {
    match mode {
        TraverseMode::Count => visited_count >= limit,
        TraverseMode::Collect => output_len >= limit,
    }
}

/// Expands one frontier node for the active profile; returns `true` when `result_limit` is hit.
fn expand_node(
    ctx: &mut DiscoverCtx<'_, '_>,
    hot: crate::topology::TopologyHot<'_>,
    unique: &crate::topology::UniqueAdjacency,
    current: u32,
) -> bool {
    match ctx.session.profile {
        TraverseProfile::OutUnique { overlay } => expand_unique_out(ctx, unique, current, overlay),
        TraverseProfile::OutParallel { overlay } => {
            expand_parallel_out(ctx, hot.forward, current, overlay)
        }
        TraverseProfile::InUnique { overlay } => expand_unique_in(ctx, unique, current, overlay),
        TraverseProfile::InParallel { overlay } => expand_parallel_in(ctx, hot, current, overlay),
    }
}

/// Outgoing unique-row expansion plus optional overlay targets.
fn expand_unique_out(
    ctx: &mut DiscoverCtx<'_, '_>,
    unique: &crate::topology::UniqueAdjacency,
    current: u32,
    merge_overlay: bool,
) -> bool {
    if visit_slice(ctx, unique.outgoing(current)) {
        return true;
    }
    merge_overlay && visit_slice(ctx, ctx.overlay.overlay_targets(current))
}

/// Outgoing parallel CSR expansion plus optional overlay targets.
fn expand_parallel_out(
    ctx: &mut DiscoverCtx<'_, '_>,
    forward: ForwardCsr<'_>,
    current: u32,
    merge_overlay: bool,
) -> bool {
    if visit_out_parallel(ctx, forward, current) {
        return true;
    }
    merge_overlay && visit_slice(ctx, ctx.overlay.overlay_targets(current))
}

/// Incoming unique-row expansion plus optional overlay sources.
fn expand_unique_in(
    ctx: &mut DiscoverCtx<'_, '_>,
    unique: &crate::topology::UniqueAdjacency,
    current: u32,
    merge_overlay: bool,
) -> bool {
    if visit_slice(ctx, unique.incoming(current)) {
        return true;
    }
    merge_overlay && visit_slice(ctx, ctx.overlay.overlay_sources(current))
}

/// Incoming parallel CSC expansion plus optional overlay sources.
fn expand_parallel_in(
    ctx: &mut DiscoverCtx<'_, '_>,
    hot: crate::topology::TopologyHot<'_>,
    current: u32,
    merge_overlay: bool,
) -> bool {
    if visit_in_parallel(ctx, hot, current) {
        return true;
    }
    merge_overlay && visit_slice(ctx, ctx.overlay.overlay_sources(current))
}

/// Visits a neighbor slice; returns `true` when `result_limit` is reached.
fn visit_slice(ctx: &mut DiscoverCtx<'_, '_>, neighbors: &[u32]) -> bool {
    for &neighbor in neighbors {
        if discover_neighbor(ctx, neighbor) {
            return true;
        }
    }
    false
}

/// Outgoing parallel CSR walk with optional edge-tombstone filtering.
fn visit_out_parallel(
    ctx: &mut DiscoverCtx<'_, '_>,
    forward: ForwardCsr<'_>,
    current: u32,
) -> bool {
    let overlay = ctx.overlay;
    if overlay.has_edge_tombstones() {
        let visit_edge = |target, edge_id| {
            if !overlay.edge_visible(edge_id) {
                return false;
            }
            discover_neighbor(ctx, target)
        };
        return forward.for_each_out_edge(current, visit_edge);
    }
    forward.for_each_out_target(current, |target| discover_neighbor(ctx, target))
}

/// Inbound parallel profile: filters predecessors by visible forward edge id.
///
/// # Performance
///
/// `perf: unspecified` on this path — worst case `O(k × degree)` per predecessor when edge
/// tombstones are active (documented slow path).
fn visit_in_parallel(
    ctx: &mut DiscoverCtx<'_, '_>,
    hot: crate::topology::TopologyHot<'_>,
    current: u32,
) -> bool {
    let inbound = hot.inbound;
    let forward = hot.forward;
    let overlay = ctx.overlay;
    let visit_pred = |pred: u32| {
        if !has_visible_forward_edge(forward, overlay, pred, current) {
            return false;
        }
        discover_neighbor(ctx, pred)
    };
    inbound.for_each_in_source(current, visit_pred)
}

/// Marks and records one newly discovered neighbor; returns `true` at `result_limit`.
fn discover_neighbor(ctx: &mut DiscoverCtx<'_, '_>, neighbor: u32) -> bool {
    if neighbor >= ctx.session.node_count {
        return false;
    }
    if ctx.check_nodes && !ctx.overlay.node_visible(neighbor) {
        return false;
    }
    if !ctx.scratch.try_mark_visited(neighbor, ctx.epoch) {
        return false;
    }
    *ctx.visited_count += 1;
    if ctx.session.mode == TraverseMode::Collect {
        let output = ctx.scratch.output_mut();
        output.push(neighbor);
        *ctx.output_len = output.len();
    }
    if ctx.enqueue_next {
        ctx.scratch.next_mut().push(neighbor);
    }
    stop_at_limit(
        ctx.session.mode,
        *ctx.visited_count,
        *ctx.output_len,
        ctx.session.result_limit,
    )
}

/// Returns whether a forward edge `source -> target` is visible under edge tombstones.
fn has_visible_forward_edge(
    forward: ForwardCsr<'_>,
    overlay: &OverlayState,
    source: u32,
    target: u32,
) -> bool {
    let graph = &forward.0;
    let target_id = CsrNodeId::new(target);
    for edge in OutgoingGraph::outgoing_edges(graph, CsrNodeId::new(source)) {
        if EdgeTargetGraph::target(graph, edge) == target_id && overlay.edge_visible(edge.get()) {
            return true;
        }
    }
    false
}
