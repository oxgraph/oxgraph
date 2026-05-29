//! Depth-bounded, multi-seed, level-synchronous BFS with a visitor sink.
//!
//! The per-element iterators in [`crate::bfs`] yield one element per
//! `Iterator::next` with no depth and a single seed. Consumers such as the
//! embedded database and the Postgres engine instead need to: seed many
//! starts, stop expanding at a maximum hop depth (while still *discovering*
//! the boundary), cap the total result count, and observe each element's
//! depth. This module provides that one level-synchronous driver over the
//! reusable [`BfsEpochScratch`], so those consumers stop hand-rolling BFS.
//!
//! Forward expansion binds [`ElementSuccessors`], reverse binds
//! [`ElementPredecessors`], and the bidirectional variant binds both and
//! expands successors then predecessors into one shared visited set so a node
//! reachable by either direction is emitted once at its shortest depth.

use core::ops::ControlFlow;

use oxgraph_topology::{
    ContainsElement, ElementId, ElementIndex, ElementPredecessors, ElementSuccessors, TopologyBase,
};

use crate::bfs::{BfsError, epoch::BfsEpochScratch};

/// Wave and limit configuration shared by the bounded BFS entry points.
///
/// # Performance
///
/// `perf: unspecified`; this is a configuration struct.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BfsBounds {
    /// Maximum hop depth. `None` is unbounded; `Some(d)` discovers (emits) but
    /// does not expand elements at depth `d`.
    pub max_depth: Option<u32>,
    /// Hard cap on the total number of emitted elements (seeds count toward
    /// it). Use `usize::MAX` for no cap.
    pub result_limit: usize,
    /// Whether depth-0 seeds are emitted to the visitor.
    pub include_seeds: bool,
}

impl BfsBounds {
    /// Returns whether an element discovered at `depth` should be expanded.
    const fn expands(self, depth: u32) -> bool {
        match self.max_depth {
            Some(max) => depth < max,
            None => true,
        }
    }
}

/// Sink notified once per discovered element, in first-discovery (breadth)
/// order, with the element's shortest depth from the seed set.
///
/// Returning [`ControlFlow::Break`] halts the traversal immediately. A blanket
/// impl covers `FnMut(ElementId<G>, u32) -> ControlFlow<()>` closures.
///
/// # Performance
///
/// `perf: unspecified`; cost is the implementor's per-element work.
pub trait BfsVisitor<G: TopologyBase> {
    /// Observes `element` discovered at `depth`.
    fn visit(&mut self, element: ElementId<G>, depth: u32) -> ControlFlow<()>;
}

impl<G, F> BfsVisitor<G> for F
where
    G: TopologyBase,
    F: FnMut(ElementId<G>, u32) -> ControlFlow<()>,
{
    fn visit(&mut self, element: ElementId<G>, depth: u32) -> ControlFlow<()> {
        self(element, depth)
    }
}

/// Mutable wave state for one bounded traversal over caller-provided scratch.
struct BoundedRun<'run, G, V>
where
    G: ContainsElement + ElementIndex,
    V: BfsVisitor<G>,
{
    /// Dense visited-epoch marks indexed by `element_index`.
    marks: &'run mut [u32],
    /// Frontier queue storage (capacity at least `element_bound`).
    queue: &'run mut [ElementId<G>],
    /// Current traversal epoch; a mark equal to it means "visited this run".
    epoch: u32,
    /// Index of the next element to pop.
    head: usize,
    /// Exclusive end of the queued region.
    tail: usize,
    /// Exclusive end of the current depth wave.
    wave_end: usize,
    /// Depth of the elements currently being popped.
    depth: u32,
    /// Count of elements emitted to the visitor so far.
    emitted: usize,
    /// Wave and limit configuration.
    bounds: BfsBounds,
    /// Exclusive bound observed at construction; expansion targets must be below it.
    element_bound: usize,
    /// Discovery sink.
    visitor: &'run mut V,
}

impl<G, V> BoundedRun<'_, G, V>
where
    G: ContainsElement + ElementIndex,
    V: BfsVisitor<G>,
{
    /// Pops the next frontier element with its depth, advancing the wave
    /// boundary when the current depth is exhausted.
    fn pop(&mut self) -> Option<(ElementId<G>, u32)> {
        if self.head == self.tail {
            return None;
        }
        if self.head == self.wave_end {
            self.depth = self.depth.saturating_add(1);
            self.wave_end = self.tail;
        }
        let element = self.queue[self.head];
        self.head += 1;
        Some((element, self.depth))
    }

    /// Marks, queues, and emits `target` at `depth` when first seen.
    ///
    /// Returns `Break` when the visitor asks to stop or the result cap is hit.
    /// Returns an error when `target` maps outside the construction-time bound.
    fn discover(
        &mut self,
        target: ElementId<G>,
        index: usize,
        depth: u32,
    ) -> Result<ControlFlow<()>, BfsError> {
        if index >= self.element_bound {
            return Err(BfsError::NeighborIndexOutOfBounds {
                index,
                bound: self.element_bound,
            });
        }
        if self.marks[index] == self.epoch {
            return Ok(ControlFlow::Continue(()));
        }
        self.marks[index] = self.epoch;
        self.queue[self.tail] = target;
        self.tail += 1;
        Ok(self.emit(target, depth))
    }

    /// Emits one element and applies the result-cap stop condition.
    fn emit(&mut self, element: ElementId<G>, depth: u32) -> ControlFlow<()> {
        if self.visitor.visit(element, depth).is_break() {
            return ControlFlow::Break(());
        }
        self.emitted += 1;
        if self.emitted >= self.bounds.result_limit {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

/// Validates scratch capacity and seeds the run, emitting depth-0 seeds.
///
/// Returns `Ok(None)` when the traversal should stop immediately (limit hit
/// while emitting seeds).
fn start_run<'run, G, V>(
    graph: &G,
    scratch: &'run mut BfsEpochScratch<'_, G>,
    seeds: &[ElementId<G>],
    bounds: BfsBounds,
    visitor: &'run mut V,
) -> Result<Option<BoundedRun<'run, G, V>>, BfsError>
where
    G: ContainsElement + ElementIndex,
    V: BfsVisitor<G>,
{
    let element_bound = graph.element_bound();
    if scratch.mark_capacity() < element_bound {
        return Err(BfsError::VisitedTooSmall {
            needed: element_bound,
            actual: scratch.mark_capacity(),
        });
    }
    if scratch.queue_capacity() < element_bound {
        return Err(BfsError::QueueTooSmall {
            needed: element_bound,
            actual: scratch.queue_capacity(),
        });
    }
    let (marks, queue, epoch) = scratch.bounded_parts();
    let mut run = BoundedRun {
        marks,
        queue,
        epoch,
        head: 0,
        tail: 0,
        wave_end: 0,
        depth: 0,
        emitted: 0,
        bounds,
        element_bound,
        visitor,
    };

    for &seed in seeds {
        if !graph.contains_element(seed) {
            return Err(BfsError::StartElementNotContained);
        }
        let index = graph.element_index(seed);
        if index >= element_bound {
            return Err(BfsError::StartIndexOutOfBounds {
                index,
                bound: element_bound,
            });
        }
        if run.marks[index] == run.epoch {
            continue;
        }
        run.marks[index] = run.epoch;
        run.queue[run.tail] = seed;
        run.tail += 1;
        if bounds.include_seeds && run.emit(seed, 0).is_break() {
            return Ok(None);
        }
    }
    // Depth-0 wave spans the seeds queued above.
    run.wave_end = run.tail;
    Ok(Some(run))
}

/// Runs the forward bounded BFS, calling `visitor` once per discovered element.
///
/// Seeds are marked and (when `bounds.include_seeds`) emitted at depth 0 in
/// caller order, then expanded wave by wave through
/// [`ElementSuccessors`](oxgraph_topology::ElementSuccessors). Each reachable
/// element is emitted once, at its shortest depth, in breadth order; elements
/// at `bounds.max_depth` are emitted but not expanded.
///
/// # Errors
///
/// Returns [`BfsError::VisitedTooSmall`] / [`BfsError::QueueTooSmall`] when
/// `scratch` is smaller than `graph.element_bound()`,
/// [`BfsError::StartElementNotContained`] / [`BfsError::StartIndexOutOfBounds`]
/// for an invalid seed, and [`BfsError::NeighborIndexOutOfBounds`] when an
/// expansion target maps outside the bound.
///
/// # Performance
///
/// `O(n + m)` over the visited subgraph (`n` elements, `m` inspected
/// successor entries) using only the caller-provided scratch.
pub fn breadth_first_search_bounded<G, V>(
    graph: &G,
    seeds: &[ElementId<G>],
    bounds: BfsBounds,
    scratch: &mut BfsEpochScratch<'_, G>,
    visitor: &mut V,
) -> Result<(), BfsError>
where
    G: ContainsElement + ElementSuccessors + ElementIndex,
    V: BfsVisitor<G>,
{
    let Some(mut run) = start_run(graph, scratch, seeds, bounds, visitor)? else {
        return Ok(());
    };
    while let Some((element, depth)) = run.pop() {
        if !bounds.expands(depth) {
            continue;
        }
        for target in graph.element_successors(element) {
            let index = graph.element_index(target);
            if run.discover(target, index, depth + 1)?.is_break() {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Runs the reverse bounded BFS, expanding through
/// [`ElementPredecessors`](oxgraph_topology::ElementPredecessors).
///
/// Same seeding, depth, limit, and emission contract as
/// [`breadth_first_search_bounded`], with predecessor expansion.
///
/// # Errors
///
/// Same as [`breadth_first_search_bounded`].
///
/// # Performance
///
/// `O(n + m)` over the reverse-visited subgraph using only caller scratch.
pub fn reverse_breadth_first_search_bounded<G, V>(
    graph: &G,
    seeds: &[ElementId<G>],
    bounds: BfsBounds,
    scratch: &mut BfsEpochScratch<'_, G>,
    visitor: &mut V,
) -> Result<(), BfsError>
where
    G: ContainsElement + ElementPredecessors + ElementIndex,
    V: BfsVisitor<G>,
{
    let Some(mut run) = start_run(graph, scratch, seeds, bounds, visitor)? else {
        return Ok(());
    };
    while let Some((element, depth)) = run.pop() {
        if !bounds.expands(depth) {
            continue;
        }
        for target in graph.element_predecessors(element) {
            let index = graph.element_index(target);
            if run.discover(target, index, depth + 1)?.is_break() {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Runs the bidirectional bounded BFS, expanding successors then predecessors
/// per element into one shared visited set.
///
/// A node reachable by either direction is emitted once at its shortest depth.
/// Same seeding, depth, limit, and emission contract as
/// [`breadth_first_search_bounded`].
///
/// # Errors
///
/// Same as [`breadth_first_search_bounded`].
///
/// # Performance
///
/// `O(n + m_out + m_in)` over the visited subgraph using only caller scratch.
pub fn breadth_first_search_bounded_both<G, V>(
    graph: &G,
    seeds: &[ElementId<G>],
    bounds: BfsBounds,
    scratch: &mut BfsEpochScratch<'_, G>,
    visitor: &mut V,
) -> Result<(), BfsError>
where
    G: ContainsElement + ElementSuccessors + ElementPredecessors + ElementIndex,
    V: BfsVisitor<G>,
{
    let Some(mut run) = start_run(graph, scratch, seeds, bounds, visitor)? else {
        return Ok(());
    };
    while let Some((element, depth)) = run.pop() {
        if !bounds.expands(depth) {
            continue;
        }
        for target in graph.element_successors(element) {
            let index = graph.element_index(target);
            if run.discover(target, index, depth + 1)?.is_break() {
                return Ok(());
            }
        }
        for target in graph.element_predecessors(element) {
            let index = graph.element_index(target);
            if run.discover(target, index, depth + 1)?.is_break() {
                return Ok(());
            }
        }
    }
    Ok(())
}
