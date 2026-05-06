//! Criterion benchmarks for `PageRank` iteration throughput.

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_algo::{PageRankConfig, PageRankWorkspace, pagerank, pagerank_with_workspace};
use oxgraph_csr::{CsrError, CsrGraph, CsrNodeId};

/// Owned CSR arrays for a benchmark graph.
type OwnedCsr = (Vec<u32>, Vec<u32>);

/// Builds a deterministic directed ring with one skip edge per node.
#[expect(
    clippy::cast_possible_truncation,
    reason = "benchmark fixture uses 10k nodes, far below u32::MAX"
)]
fn build_graph(nodes: usize) -> Result<OwnedCsr, CsrError<u32>> {
    let mut offsets = Vec::with_capacity(nodes + 1);
    let mut targets = Vec::with_capacity(nodes.saturating_mul(2));
    offsets.push(0_u32);
    for node in 0..nodes {
        targets.push(((node + 1) % nodes) as u32);
        targets.push(((node + 7) % nodes) as u32);
        offsets.push(targets.len() as u32);
    }
    let _graph = CsrGraph::validate(nodes as u32, &offsets, &targets)?;
    Ok((offsets, targets))
}

/// Benchmarks `PageRank` over a 10k-node CSR fixture.
fn pagerank_throughput(c: &mut Criterion) {
    let (offsets, targets) = build_graph(10_000)
        .unwrap_or_else(|error| panic!("benchmark CSR graph should validate: {error}"));
    let graph = CsrGraph::validate(10_000_u32, &offsets, &targets)
        .unwrap_or_else(|error| panic!("benchmark CSR graph should validate: {error}"));
    let elements: Vec<CsrNodeId<u32>> = (0..10_000_u32).map(CsrNodeId).collect();
    let config = PageRankConfig::new(0.85, 1.0e-9, 50);
    c.bench_function("pagerank_csr_10k_20k_edges", |b| {
        b.iter(|| {
            let mut ranks = vec![0.0; 10_000];
            pagerank(&graph, elements.iter().copied(), config, None, &mut ranks)
                .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
        });
    });
    c.bench_function("pagerank_workspace_csr_10k_20k_edges", |b| {
        let mut workspace = PageRankWorkspace::for_graph(&graph);
        let mut ranks = vec![0.0; 10_000];
        b.iter(|| {
            pagerank_with_workspace(
                &graph,
                elements.iter().copied(),
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
