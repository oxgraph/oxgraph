//! Graph projection traversal primitives.

use core::ops::ControlFlow;
use std::collections::{BTreeMap, BTreeSet};

use oxgraph_algo::{
    BfsBounds, BfsEpochScratch, breadth_first_search_bounded, breadth_first_search_bounded_both,
    reverse_breadth_first_search_bounded,
};
use oxgraph_graph::{
    CanonicalElementIdentity, CanonicalRelationIdentity, EdgeSourceGraph, EdgeTargetGraph,
    ElementIndex, LocalElementIdentity, OutgoingGraph,
};

use crate::{
    DbError, ElementId, RelationId,
    projection::{GraphProjection, ProjectionElementId},
};

/// Direction a graph navigation expands along.
///
/// Used by both [`Reader::neighbors`](crate::Reader::neighbors) (single-hop
/// reverse-adjacency lookup) and [`Reader::walk`](crate::Reader::walk) (bounded
/// projection BFS). `Outgoing` follows source→target edges, `Incoming` follows
/// target→source edges, and `Both` follows either.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Direction {
    /// Follow outgoing edges: the element plays the source role.
    #[default]
    Outgoing,
    /// Follow incoming edges: the element plays the target role.
    Incoming,
    /// Follow outgoing edges first, then incoming edges.
    Both,
}

/// Bounds for a bounded graph projection [`walk`](crate::Reader::walk).
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Walk {
    /// Maximum hop depth to expand from any seed.
    pub max_depth: usize,
    /// Direction used to expand each frontier element.
    pub direction: Direction,
    /// Maximum number of nodes to emit.
    pub limit: usize,
    /// Whether depth-0 seed elements are included in the result nodes.
    pub include_start: bool,
}

impl Default for Walk {
    /// Creates an outgoing depth-1 walk with an unbounded node limit, excluding
    /// the seeds.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn default() -> Self {
        Self {
            max_depth: 1,
            direction: Direction::Outgoing,
            limit: usize::MAX,
            include_start: false,
        }
    }
}

/// One node discovered by a [`walk`](crate::Reader::walk).
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraversedNode {
    /// Canonical element discovered by the walk.
    pub element: ElementId,
    /// Shortest discovered hop depth from any seed.
    pub depth: usize,
}

/// One edge traversed by a [`walk`](crate::Reader::walk).
///
/// Edges connect two discovered nodes; the source and target follow the
/// projection's source/target roles.
///
/// # Performance
///
/// Copying and comparing are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraversedEdge {
    /// Canonical relation backing this edge.
    pub relation: RelationId,
    /// Source endpoint element (the projection source role).
    pub source: ElementId,
    /// Target endpoint element (the projection target role).
    pub target: ElementId,
}

/// A discovered subgraph: nodes in BFS first-discovery order plus the projection
/// edges among them.
///
/// Nodes are unique by canonical element. Edges connect two discovered nodes and
/// are unique by canonical relation, ordered by ascending `(source, target,
/// relation)`.
///
/// # Performance
///
/// Iterating nodes or edges is `O(node count)` / `O(edge count)`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Subgraph {
    /// Discovered nodes in BFS first-discovery order.
    pub nodes: Vec<TraversedNode>,
    /// Edges connecting two discovered nodes.
    pub edges: Vec<TraversedEdge>,
}

impl Subgraph {
    /// Returns the discovered nodes.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn nodes(&self) -> &[TraversedNode] {
        &self.nodes
    }

    /// Returns the traversed edges.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn edges(&self) -> &[TraversedEdge] {
        &self.edges
    }
}

/// Walks a materialized graph projection over the substrate bounded BFS,
/// returning the discovered nodes AND the projection edges among them.
///
/// Seeds, depth bound, node limit, and seed inclusion map onto [`BfsBounds`];
/// the visitor records each discovered element as one [`TraversedNode`] in
/// first-discovery order. `Outgoing` uses [`breadth_first_search_bounded`],
/// `Incoming` uses [`reverse_breadth_first_search_bounded`], and `Both` uses
/// [`breadth_first_search_bounded_both`]. After the node set is fixed, every
/// projection edge whose source and target are BOTH discovered is emitted as one
/// [`TraversedEdge`], deduplicated by relation and ordered by `(source, target,
/// relation)`, so the result never references a node it omitted.
///
/// # Errors
///
/// Returns [`DbError::UnknownElement`] when a seed is not part of the
/// projection, or [`DbError::Traversal`] when bounded BFS fails.
///
/// # Performance
///
/// This function is `O(visited nodes + visited edges)` over the materialized
/// projection.
pub(crate) fn walk_graph_projection(
    graph: &GraphProjection,
    seeds: &[ElementId],
    walk: Walk,
) -> Result<Subgraph, DbError> {
    if seeds.is_empty() || walk.limit == 0 {
        return Ok(Subgraph::default());
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
        max_depth: Some(u32::try_from(walk.max_depth).unwrap_or(u32::MAX)),
        result_limit: walk.limit,
        include_seeds: walk.include_start,
    };

    let bound = graph.element_bound();
    let mut marks = vec![0_u32; bound];
    let mut queue = vec![ProjectionElementId::new(0); bound];
    let mut scratch = BfsEpochScratch::for_graph(graph, &mut marks, &mut queue);

    let mut nodes = Vec::new();
    // Seeds are always part of the discovered node set for edge collection, even
    // when `include_start` excludes them from the emitted result nodes — an edge
    // rooted at a seed connects to a discovered node and must be reported.
    let mut discovered: BTreeMap<ElementId, ProjectionElementId> = local_seeds
        .iter()
        .map(|&local| (graph.canonical_element_id(local), local))
        .collect();
    {
        let mut visitor = |element: ProjectionElementId, depth: u32| {
            let canonical = graph.canonical_element_id(element);
            let depth = usize::try_from(depth).unwrap_or(usize::MAX);
            nodes.push(TraversedNode {
                element: canonical,
                depth,
            });
            discovered.insert(canonical, element);
            ControlFlow::Continue(())
        };

        match walk.direction {
            Direction::Outgoing => breadth_first_search_bounded(
                graph,
                &local_seeds,
                bounds,
                &mut scratch,
                &mut visitor,
            ),
            Direction::Incoming => reverse_breadth_first_search_bounded(
                graph,
                &local_seeds,
                bounds,
                &mut scratch,
                &mut visitor,
            ),
            Direction::Both => breadth_first_search_bounded_both(
                graph,
                &local_seeds,
                bounds,
                &mut scratch,
                &mut visitor,
            ),
        }
        .map_err(|_error| DbError::traversal("bounded traversal failed"))?;
    }

    let edges = collect_internal_edges(graph, &discovered);
    Ok(Subgraph { nodes, edges })
}

/// Collects the projection edges whose source and target are both discovered.
///
/// Edges are deduplicated by canonical relation and ordered by `(source, target,
/// relation)` so the result is deterministic and never references an undiscovered
/// node.
///
/// # Performance
///
/// This function is `O(discovered nodes * out-degree + edge count log edge
/// count)`.
fn collect_internal_edges(
    graph: &GraphProjection,
    discovered: &BTreeMap<ElementId, ProjectionElementId>,
) -> Vec<TraversedEdge> {
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    for local in discovered.values().copied() {
        for edge in graph.outgoing_edges(local) {
            let source = graph.canonical_element_id(graph.source(edge));
            let target = graph.canonical_element_id(graph.target(edge));
            if !discovered.contains_key(&target) {
                continue;
            }
            let relation = graph.canonical_relation_id(edge);
            if !seen.insert(relation) {
                continue;
            }
            edges.push(TraversedEdge {
                relation,
                source,
                target,
            });
        }
    }
    edges.sort_unstable();
    edges
}
