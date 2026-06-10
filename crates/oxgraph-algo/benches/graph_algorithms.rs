//! Criterion benches defending the stated near-linear `O(b + n + m)` perf
//! contracts of the graph algorithms over a generated dense-index view.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_algo::{
    connected_components, shortest_path_lengths, strongly_connected_components, topological_sort,
};
use oxgraph_topology::{DenseElementIndex, ElementSuccessors, TopologyBase};

/// Minimal directed adjacency-list view used by the benches.
struct FixtureGraph {
    /// Successor list per dense element index.
    adjacency: Vec<Vec<usize>>,
}

impl TopologyBase for FixtureGraph {
    type ElementId = usize;
    type RelationId = usize;
}

impl DenseElementIndex for FixtureGraph {
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

/// Builds an acyclic graph (`i -> i+1`, `i -> i+2`) for toposort/SSSP.
fn build_dag(node_count: usize) -> FixtureGraph {
    let mut adjacency = vec![Vec::new(); node_count];
    for (source, successors) in adjacency.iter_mut().enumerate() {
        for step in 1..=2 {
            if source + step < node_count {
                successors.push(source + step);
            }
        }
    }
    FixtureGraph { adjacency }
}

/// Builds a single big cycle (`i -> (i+1) % n`) plus a forward chord, exercising
/// SCC and weakly-connected components on one large component.
fn build_cyclic(node_count: usize) -> FixtureGraph {
    let mut adjacency = vec![Vec::new(); node_count];
    for (source, successors) in adjacency.iter_mut().enumerate() {
        successors.push((source + 1) % node_count);
        if source + 7 < node_count {
            successors.push(source + 7);
        }
    }
    FixtureGraph { adjacency }
}

/// Benchmarks each graph algorithm on a 5k-node generated view.
fn bench_graph_algorithms(criterion: &mut Criterion) {
    let node_count = 5_000;
    let nodes: Vec<usize> = (0..node_count).collect();
    let dag = build_dag(node_count);
    let cyclic = build_cyclic(node_count);

    criterion.bench_function("topological_sort_5k", |bencher| {
        bencher.iter(|| topological_sort(black_box(&dag), black_box(&nodes)));
    });
    criterion.bench_function("shortest_path_lengths_5k", |bencher| {
        bencher.iter(|| shortest_path_lengths(black_box(&dag), black_box(0), black_box(&nodes)));
    });
    criterion.bench_function("strongly_connected_components_5k", |bencher| {
        bencher.iter(|| strongly_connected_components(black_box(&cyclic), black_box(&nodes)));
    });
    criterion.bench_function("connected_components_5k", |bencher| {
        bencher.iter(|| connected_components(black_box(&cyclic), black_box(&nodes)));
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_graph_algorithms
}
criterion_main!(benches);
