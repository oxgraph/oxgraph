//! Tests for the substrate-agnostic graph algorithms (topological sort,
//! strongly/weakly connected components, unweighted shortest paths) over an
//! in-test topology view.

#![cfg(feature = "alloc")]

use oxgraph_algo::{
    LongestPathError, ToposortError, connected_components, longest_path_dag, shortest_path_lengths,
    strongly_connected_components, topological_sort,
};
use oxgraph_topology::{ElementIndex, ElementSuccessors, TopologyBase};
use proptest::prelude::*;

/// Minimal directed adjacency-list view implementing only the traits the
/// algorithms need. Element ids are dense `usize` slots.
struct FixtureGraph {
    /// Successor list per dense element index.
    adjacency: Vec<Vec<usize>>,
}

impl FixtureGraph {
    /// Builds a view from a per-node successor list.
    const fn new(adjacency: Vec<Vec<usize>>) -> Self {
        Self { adjacency }
    }

    /// Returns every node id, the usual full induced set.
    fn all(&self) -> Vec<usize> {
        (0..self.adjacency.len()).collect()
    }
}

impl TopologyBase for FixtureGraph {
    type ElementId = usize;
    type RelationId = usize;
}

impl ElementIndex for FixtureGraph {
    fn element_bound(&self) -> usize {
        self.adjacency.len()
    }

    fn element_index(&self, element: usize) -> usize {
        element
    }
}

impl ElementSuccessors for FixtureGraph {
    type Successors<'view>
        = core::iter::Copied<core::slice::Iter<'view, usize>>
    where
        Self: 'view;

    fn element_successors(&self, element: usize) -> Self::Successors<'_> {
        self.adjacency[element].iter().copied()
    }
}

/// Sorts components and their members for order-insensitive comparison.
fn normalize(mut components: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    for component in &mut components {
        component.sort_unstable();
    }
    components.sort_unstable();
    components
}

#[test]
fn topological_sort_orders_a_dag() {
    let graph = FixtureGraph::new(vec![vec![1, 2], vec![3], vec![3], vec![]]);
    let Ok(order) = topological_sort(&graph, &graph.all()) else {
        panic!("dag is acyclic");
    };
    assert_eq!(order.len(), 4);
    let mut position = [0usize; 4];
    for (rank, &node) in order.iter().enumerate() {
        position[node] = rank;
    }
    for (source, targets) in graph.adjacency.iter().enumerate() {
        for &target in targets {
            assert!(position[source] < position[target]);
        }
    }
}

#[test]
fn topological_sort_detects_a_cycle() {
    let graph = FixtureGraph::new(vec![vec![1], vec![2], vec![0]]);
    assert_eq!(
        topological_sort(&graph, &graph.all()),
        Err(ToposortError::Cycle)
    );
}

#[test]
fn topological_sort_treats_a_self_loop_as_a_cycle() {
    let graph = FixtureGraph::new(vec![vec![0]]);
    assert_eq!(
        topological_sort(&graph, &graph.all()),
        Err(ToposortError::Cycle)
    );
}

#[test]
fn strongly_connected_components_partition_cycles() {
    // {0,1,2} form a cycle; 3 is a sink reachable from the cycle.
    let graph = FixtureGraph::new(vec![vec![1], vec![2], vec![0, 3], vec![]]);
    let components = normalize(strongly_connected_components(&graph, &graph.all()));
    assert_eq!(components, vec![vec![0, 1, 2], vec![3]]);
}

#[test]
fn connected_components_group_weakly_reachable_nodes() {
    // 0->1 and 2->3 form two pairs; 4 is isolated.
    let graph = FixtureGraph::new(vec![vec![1], vec![], vec![3], vec![], vec![]]);
    let components = normalize(connected_components(&graph, &graph.all()));
    assert_eq!(components, vec![vec![0, 1], vec![2, 3], vec![4]]);
}

#[test]
fn shortest_path_lengths_layer_by_hops() {
    // 0->1->2 and 0->3.
    let graph = FixtureGraph::new(vec![vec![1, 3], vec![2], vec![], vec![]]);
    let mut distances = shortest_path_lengths(&graph, 0, &graph.all());
    distances.sort_unstable();
    assert_eq!(distances, vec![(0, 0), (1, 1), (2, 2), (3, 1)]);
}

#[test]
fn shortest_path_lengths_reject_an_absent_source() {
    let graph = FixtureGraph::new(vec![vec![1], vec![]]);
    assert!(shortest_path_lengths(&graph, 5, &graph.all()).is_empty());
}

#[test]
fn longest_path_dag_follows_a_linear_chain() {
    let graph = FixtureGraph::new(vec![vec![1], vec![2], vec![3], vec![]]);
    let Ok(path) = longest_path_dag(&graph, &graph.all()) else {
        panic!("dag is acyclic");
    };
    assert_eq!(path, vec![0, 1, 2, 3]);
}

#[test]
fn longest_path_dag_takes_the_longer_diamond_arm() {
    // 0 -> 1 -> 3 and 0 -> 2; the 0,1,3 chain (length 2) beats 0,2 (length 1).
    let graph = FixtureGraph::new(vec![vec![1, 2], vec![3], vec![], vec![]]);
    let Ok(path) = longest_path_dag(&graph, &graph.all()) else {
        panic!("dag is acyclic");
    };
    assert_eq!(path, vec![0, 1, 3]);
}

#[test]
fn longest_path_dag_detects_a_cycle() {
    let graph = FixtureGraph::new(vec![vec![1], vec![2], vec![0]]);
    assert_eq!(
        longest_path_dag(&graph, &graph.all()),
        Err(LongestPathError::Cycle)
    );
}

#[test]
fn longest_path_dag_treats_a_self_loop_as_a_cycle() {
    let graph = FixtureGraph::new(vec![vec![0]]);
    assert_eq!(
        longest_path_dag(&graph, &graph.all()),
        Err(LongestPathError::Cycle)
    );
}

#[test]
fn longest_path_dag_handles_empty_and_singleton_sets() {
    let graph = FixtureGraph::new(vec![vec![1], vec![]]);
    assert_eq!(longest_path_dag(&graph, &[]), Ok(Vec::new()));
    assert_eq!(longest_path_dag(&graph, &[1]), Ok(vec![1]));
}

#[test]
fn longest_path_dag_picks_the_longest_disconnected_component() {
    // Component {0,1,2} has length 2; component {3,4} has length 1.
    let graph = FixtureGraph::new(vec![vec![1], vec![2], vec![], vec![4], vec![]]);
    let Ok(path) = longest_path_dag(&graph, &graph.all()) else {
        panic!("dag is acyclic");
    };
    assert_eq!(path, vec![0, 1, 2]);
}

#[test]
fn longest_path_dag_respects_the_induced_subset() {
    // Full chain 0->1->2->3; excluding 2 from the element set drops edges 1->2
    // and 2->3, so the induced longest path over {0,1,3} is just 0->1.
    let graph = FixtureGraph::new(vec![vec![1], vec![2], vec![3], vec![]]);
    let Ok(path) = longest_path_dag(&graph, &[0, 1, 3]) else {
        panic!("induced subgraph is acyclic");
    };
    assert_eq!(path, vec![0, 1]);
}

#[test]
fn longest_path_dag_breaks_cycles_outside_the_induced_subset() {
    // The full graph has a cycle 0->1->2->0, but the subset {0,1} induces only
    // the acyclic edge 0->1, so a path is found rather than a Cycle error.
    let graph = FixtureGraph::new(vec![vec![1], vec![2], vec![0]]);
    assert_eq!(longest_path_dag(&graph, &[0, 1]), Ok(vec![0, 1]));
}

/// Independent oracle: the longest chain (in edges) starting at `node` within a
/// forward-only adjacency list, memoized over the small node set.
fn brute_longest_from(node: usize, adjacency: &[Vec<usize>], memo: &mut [Option<usize>]) -> usize {
    if let Some(cached) = memo[node] {
        return cached;
    }
    let mut best = 0usize;
    for &successor in &adjacency[node] {
        best = best.max(1 + brute_longest_from(successor, adjacency, memo));
    }
    memo[node] = Some(best);
    best
}

proptest! {
    /// On any forward-only DAG, the returned path is a valid simple chain whose
    /// every step is a real edge, and its edge count equals the true longest
    /// chain computed by an independent brute-force oracle.
    #[test]
    fn longest_path_dag_matches_a_brute_force_oracle(
        node_count in 1usize..8,
        raw_edges in proptest::collection::vec((0usize..8, 0usize..8), 0..20),
    ) {
        let mut adjacency = vec![Vec::new(); node_count];
        for (source, target) in raw_edges {
            if source < node_count && target < node_count && source < target {
                adjacency[source].push(target);
            }
        }
        let graph = FixtureGraph::new(adjacency);
        let Ok(path) = longest_path_dag(&graph, &graph.all()) else {
            panic!("forward-only graph should be acyclic");
        };

        // Every consecutive pair on the path is a real edge.
        for window in path.windows(2) {
            prop_assert!(graph.adjacency[window[0]].contains(&window[1]));
        }
        // The path visits each node at most once.
        let mut seen = vec![false; node_count];
        for &node in &path {
            prop_assert!(!seen[node]);
            seen[node] = true;
        }
        // Its edge count is the global maximum.
        let mut memo = vec![None; node_count];
        let oracle = (0..node_count)
            .map(|node| brute_longest_from(node, &graph.adjacency, &mut memo))
            .max()
            .unwrap_or(0);
        prop_assert_eq!(path.len().saturating_sub(1), oracle);
    }
}

proptest! {
    /// A topological order of any DAG places every edge's source before its
    /// target. DAGs are generated by only allowing forward edges `i -> j` with
    /// `i < j`.
    #[test]
    fn topological_sort_respects_every_edge(
        node_count in 1usize..8,
        raw_edges in proptest::collection::vec((0usize..8, 0usize..8), 0..20),
    ) {
        let mut adjacency = vec![Vec::new(); node_count];
        for (source, target) in raw_edges {
            if source < node_count && target < node_count && source < target {
                adjacency[source].push(target);
            }
        }
        let graph = FixtureGraph::new(adjacency);
        let Ok(order) = topological_sort(&graph, &graph.all()) else {
            panic!("forward-only graph should be acyclic");
        };
        prop_assert_eq!(order.len(), node_count);
        let mut position = vec![0usize; node_count];
        for (rank, &node) in order.iter().enumerate() {
            position[node] = rank;
        }
        for (source, targets) in graph.adjacency.iter().enumerate() {
            for &target in targets {
                prop_assert!(position[source] < position[target]);
            }
        }
    }
}
