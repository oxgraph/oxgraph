//! Graph projection traversal primitives.

use std::collections::{BTreeSet, VecDeque};

use oxgraph_graph::{
    CanonicalElementIdentity, ElementPredecessors, ElementSuccessors, LocalElementIdentity,
};

use crate::{
    DbError, ElementId,
    projection::{GraphProjection, ProjectionElementId},
};

/// Direction used by graph projection traversal.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TraversalDirection {
    /// Follow outgoing graph projection neighbors.
    #[default]
    Outgoing,
    /// Follow incoming graph projection neighbors.
    Incoming,
    /// Follow outgoing neighbors first, then incoming neighbors.
    Both,
}

/// Options for bounded graph projection traversal.
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraversalOptions {
    /// Maximum hop depth to traverse from any seed.
    pub max_depth: usize,
    /// Direction used to expand each frontier element.
    pub direction: TraversalDirection,
    /// Maximum number of rows to emit.
    pub limit: usize,
    /// Whether depth-0 seed elements are included in the result rows.
    pub include_start: bool,
}

impl Default for TraversalOptions {
    /// Creates outgoing depth-1 traversal options with an unbounded row limit.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn default() -> Self {
        Self {
            max_depth: 1,
            direction: TraversalDirection::Outgoing,
            limit: usize::MAX,
            include_start: false,
        }
    }
}

/// One graph traversal result row.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraversalRow {
    /// Canonical element discovered by traversal.
    pub element: ElementId,
    /// Shortest discovered hop depth from any seed.
    pub depth: usize,
}

/// Materialized graph traversal result.
///
/// Rows are unique by canonical element and ordered by BFS first discovery.
///
/// # Performance
///
/// Iterating rows is `O(row count)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraversalResult {
    /// Materialized traversal rows.
    rows: Vec<TraversalRow>,
}

impl TraversalResult {
    /// Creates a traversal result from rows.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` excluding row ownership transfer.
    #[must_use]
    pub(crate) const fn new(rows: Vec<TraversalRow>) -> Self {
        Self { rows }
    }

    /// Returns result rows.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn rows(&self) -> &[TraversalRow] {
        &self.rows
    }
}

/// Traverses a materialized graph projection.
pub(crate) fn traverse_graph_projection(
    graph: &GraphProjection,
    seeds: &[ElementId],
    options: TraversalOptions,
) -> Result<TraversalResult, DbError> {
    if seeds.is_empty() || options.limit == 0 {
        return Ok(TraversalResult::new(Vec::new()));
    }

    let mut traversal = TraversalState::new(graph, options.limit);
    for seed in seeds {
        let local = graph
            .local_element_id(*seed)
            .ok_or(DbError::UnknownElement { id: *seed })?;
        traversal.push_seed(local, *seed, options.include_start);
        if traversal.at_limit() {
            return Ok(traversal.finish());
        }
    }

    while let Some((element, depth)) = traversal.pop_frontier() {
        if depth >= options.max_depth {
            continue;
        }
        let next_depth = depth + 1;
        let reached_limit = match options.direction {
            TraversalDirection::Outgoing => traversal.visit_outgoing(element, next_depth),
            TraversalDirection::Incoming => traversal.visit_incoming(element, next_depth),
            TraversalDirection::Both => {
                traversal.visit_outgoing(element, next_depth)
                    || traversal.visit_incoming(element, next_depth)
            }
        };
        if reached_limit {
            return Ok(traversal.finish());
        }
    }
    Ok(traversal.finish())
}

/// Mutable state for one traversal.
struct TraversalState<'graph> {
    /// Graph being traversed.
    graph: &'graph GraphProjection,
    /// Maximum emitted rows.
    limit: usize,
    /// Projection-local elements already discovered.
    visited: BTreeSet<ProjectionElementId>,
    /// FIFO BFS frontier.
    queue: VecDeque<(ProjectionElementId, usize)>,
    /// Materialized result rows.
    rows: Vec<TraversalRow>,
}

impl<'graph> TraversalState<'graph> {
    /// Creates empty traversal state.
    const fn new(graph: &'graph GraphProjection, limit: usize) -> Self {
        Self {
            graph,
            limit,
            visited: BTreeSet::new(),
            queue: VecDeque::new(),
            rows: Vec::new(),
        }
    }

    /// Adds one seed to the BFS roots.
    fn push_seed(&mut self, local: ProjectionElementId, seed: ElementId, include_start: bool) {
        if !self.visited.insert(local) {
            return;
        }
        self.queue.push_back((local, 0));
        if include_start {
            self.rows.push(TraversalRow {
                element: seed,
                depth: 0,
            });
        }
    }

    /// Pops one frontier item.
    fn pop_frontier(&mut self) -> Option<(ProjectionElementId, usize)> {
        self.queue.pop_front()
    }

    /// Returns whether the emitted row limit has been reached.
    const fn at_limit(&self) -> bool {
        self.rows.len() == self.limit
    }

    /// Visits outgoing neighbors from one frontier element.
    fn visit_outgoing(&mut self, element: ProjectionElementId, depth: usize) -> bool {
        for neighbor in self.graph.element_successors(element) {
            if self.push_discovered(neighbor, depth) {
                return true;
            }
        }
        false
    }

    /// Visits incoming neighbors from one frontier element.
    fn visit_incoming(&mut self, element: ProjectionElementId, depth: usize) -> bool {
        for neighbor in self.graph.element_predecessors(element) {
            if self.push_discovered(neighbor, depth) {
                return true;
            }
        }
        false
    }

    /// Adds one newly discovered neighbor.
    fn push_discovered(&mut self, neighbor: ProjectionElementId, depth: usize) -> bool {
        if !self.visited.insert(neighbor) {
            return false;
        }
        self.queue.push_back((neighbor, depth));
        self.rows.push(TraversalRow {
            element: self.graph.canonical_element_id(neighbor),
            depth,
        });
        self.at_limit()
    }

    /// Finishes traversal into a result.
    fn finish(self) -> TraversalResult {
        TraversalResult::new(self.rows)
    }
}
