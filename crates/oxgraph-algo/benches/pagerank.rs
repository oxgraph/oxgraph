//! Criterion benchmarks for `PageRank` iteration throughput.

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_algo::{
    PageRankConfig, PageRankWorkspace, Uniform, Weighted, pagerank_graph,
    pagerank_graph_with_workspace,
};
use oxgraph_csr::{CsrError, CsrNativeGraph, CsrNodeId};
use oxgraph_topology::{RelationWeight, TopologyBase};

/// Owned CSR arrays for a benchmark graph.
type OwnedCsr = (Vec<u32>, Vec<u32>);

/// Dense relation weights for the benchmark graph.
struct BenchRelationWeights<'view> {
    /// Weights keyed by CSR edge ID.
    values: &'view [f64],
}

impl TopologyBase for BenchRelationWeights<'_> {
    type ElementId = CsrNodeId<u32>;
    type RelationId = oxgraph_csr::CsrEdgeId<u32>;
}

impl RelationWeight for BenchRelationWeights<'_> {
    type Weight = f64;

    fn relation_weight(&self, relation: oxgraph_csr::CsrEdgeId<u32>) -> Self::Weight {
        self.values[relation.0 as usize]
    }
}

/// Builds a deterministic directed ring with one skip edge per node.
#[expect(
    clippy::cast_possible_truncation,
    reason = "benchmark fixture uses 10k nodes, far below u32::MAX"
)]
fn build_graph(nodes: usize) -> Result<OwnedCsr, CsrError<u32, u32>> {
    let mut offsets = Vec::with_capacity(nodes + 1);
    let mut targets = Vec::with_capacity(nodes.saturating_mul(2));
    offsets.push(0_u32);
    for node in 0..nodes {
        targets.push(((node + 1) % nodes) as u32);
        targets.push(((node + 7) % nodes) as u32);
        offsets.push(targets.len() as u32);
    }
    let _graph = CsrNativeGraph::<u32, u32>::validate(nodes as u32, &offsets, &targets)?;
    Ok((offsets, targets))
}

/// Benchmarks `PageRank` over a 10k-node CSR fixture.
fn pagerank_throughput(c: &mut Criterion) {
    let (offsets, targets) = build_graph(10_000)
        .unwrap_or_else(|error| panic!("benchmark CSR graph should validate: {error}"));
    let graph = CsrNativeGraph::<u32, u32>::validate(10_000_u32, &offsets, &targets)
        .unwrap_or_else(|error| panic!("benchmark CSR graph should validate: {error}"));
    let elements: Vec<CsrNodeId<u32>> = (0..10_000_u32).map(CsrNodeId).collect();
    let visible_half: Vec<CsrNodeId<u32>> = (0..10_000_u32)
        .filter(|node| node % 2 == 0)
        .map(CsrNodeId)
        .collect();
    let relation_weights = vec![1.0; targets.len()];
    let weights = BenchRelationWeights {
        values: &relation_weights,
    };
    let config = PageRankConfig::new(0.85, 1.0e-9, 50);
    c.bench_function("pagerank_csr_10k_20k_edges", |b| {
        let mut ranks = vec![0.0; 10_000];
        b.iter(|| {
            ranks.fill(0.0);
            pagerank_graph(
                &graph,
                &Uniform,
                elements.iter().copied(),
                config,
                None,
                &mut ranks,
            )
            .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
        });
    });
    c.bench_function("pagerank_workspace_csr_10k_20k_edges", |b| {
        let mut workspace = PageRankWorkspace::for_graph(&graph);
        let mut ranks = vec![0.0; 10_000];
        b.iter(|| {
            pagerank_graph_with_workspace(
                &graph,
                &Uniform,
                elements.iter().copied(),
                config,
                None,
                &mut ranks,
                &mut workspace,
            )
            .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
        });
    });
    c.bench_function("pagerank_workspace_visible_half_csr_10k", |b| {
        let mut workspace = PageRankWorkspace::for_graph(&graph);
        let mut ranks = vec![0.0; 10_000];
        b.iter(|| {
            ranks.fill(0.0);
            pagerank_graph_with_workspace(
                &graph,
                &Uniform,
                visible_half.iter().copied(),
                config,
                None,
                &mut ranks,
                &mut workspace,
            )
            .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
        });
    });
    c.bench_function("weighted_pagerank_workspace_visible_half_csr_10k", |b| {
        let mut workspace = PageRankWorkspace::for_graph(&graph);
        let mut ranks = vec![0.0; 10_000];
        b.iter(|| {
            ranks.fill(0.0);
            pagerank_graph_with_workspace(
                &graph,
                &Weighted::new(&weights),
                visible_half.iter().copied(),
                config,
                None,
                &mut ranks,
                &mut workspace,
            )
            .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
        });
    });
}

criterion_group!(benches, pagerank_throughput);
criterion_main!(benches);
