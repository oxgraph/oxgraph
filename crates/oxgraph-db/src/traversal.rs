//! Graph projection traversal primitives.

use core::ops::ControlFlow;

use oxgraph_algo::{
    BfsBounds, BfsEpochScratch, breadth_first_search_bounded, breadth_first_search_bounded_both,
    reverse_breadth_first_search_bounded,
};
use oxgraph_graph::{CanonicalElementIdentity, ElementIndex, LocalElementIdentity};

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

/// Traverses a materialized graph projection over the substrate bounded BFS.
///
/// Seeds, depth bound, row limit, and seed inclusion map onto
/// [`BfsBounds`]; the visitor records each discovered element as one
/// [`TraversalRow`] in first-discovery order. Outgoing traversal uses
/// [`breadth_first_search_bounded`], incoming uses
/// [`reverse_breadth_first_search_bounded`], and both uses
/// [`breadth_first_search_bounded_both`]. The projection already exposes the
/// `ContainsElement + ElementIndex + ElementSuccessors + ElementPredecessors`
/// capabilities these entry points require.
pub(crate) fn traverse_graph_projection(
    graph: &GraphProjection,
    seeds: &[ElementId],
    options: TraversalOptions,
) -> Result<TraversalResult, DbError> {
    if seeds.is_empty() || options.limit == 0 {
        return Ok(TraversalResult::new(Vec::new()));
    }

    let local_seeds = seeds
        .iter()
        .map(|seed| {
            graph
                .local_element_id(*seed)
                .ok_or(DbError::UnknownElement { id: *seed })
        })
        .collect::<Result<Vec<ProjectionElementId>, DbError>>()?;

    let bounds = BfsBounds {
        max_depth: Some(u32::try_from(options.max_depth).unwrap_or(u32::MAX)),
        result_limit: options.limit,
        include_seeds: options.include_start,
    };

    let bound = graph.element_bound();
    let mut marks = vec![0_u32; bound];
    let mut queue = vec![ProjectionElementId::new(0); bound];
    let mut scratch = BfsEpochScratch::for_graph(graph, &mut marks, &mut queue);

    let mut rows = Vec::new();
    {
        let mut visitor = |element: ProjectionElementId, depth: u32| {
            rows.push(TraversalRow {
                element: graph.canonical_element_id(element),
                depth: usize::try_from(depth).unwrap_or(usize::MAX),
            });
            ControlFlow::Continue(())
        };

        match options.direction {
            TraversalDirection::Outgoing => breadth_first_search_bounded(
                graph,
                &local_seeds,
                bounds,
                &mut scratch,
                &mut visitor,
            ),
            TraversalDirection::Incoming => reverse_breadth_first_search_bounded(
                graph,
                &local_seeds,
                bounds,
                &mut scratch,
                &mut visitor,
            ),
            TraversalDirection::Both => breadth_first_search_bounded_both(
                graph,
                &local_seeds,
                bounds,
                &mut scratch,
                &mut visitor,
            ),
        }
        .map_err(DbError::traversal)?;
    }

    Ok(TraversalResult::new(rows))
}
