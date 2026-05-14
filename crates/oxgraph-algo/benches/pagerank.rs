//! Criterion benchmarks for `PageRank` iteration throughput.

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_algo::{
    HyperWeighted, HypergraphPageRankWorkspace, PageRankConfig, PageRankWorkspace, Uniform,
    Weighted, pagerank_graph, pagerank_graph_with_workspace, pagerank_hypergraph,
    pagerank_hypergraph_with_workspace,
};
use oxgraph_csr::{CsrError, CsrNativeGraph, CsrNodeId};
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHyperedgeId, BcsrNativeHypergraph, BcsrParticipantId, BcsrRole, BcsrSections,
    BcsrVertexId,
};
use oxgraph_topology::{IncidenceBase, IncidenceWeight, RelationWeight, TopologyBase};

/// Owned CSR arrays for a benchmark graph.
type OwnedCsr = (Vec<u32>, Vec<u32>);

/// Owned BCSR sections for a benchmark hypergraph.
struct OwnedBcsr {
    /// Per-hyperedge head-participant offsets.
    head_offsets: Vec<u32>,
    /// Head vertex participants concatenated across hyperedges.
    head_participants: Vec<u32>,
    /// Per-hyperedge tail-participant offsets.
    tail_offsets: Vec<u32>,
    /// Tail vertex participants concatenated across hyperedges.
    tail_participants: Vec<u32>,
    /// Per-vertex outgoing-hyperedge offsets.
    vertex_outgoing_offsets: Vec<u32>,
    /// Outgoing hyperedge IDs concatenated across vertices.
    vertex_outgoing_hyperedges: Vec<u32>,
    /// Per-vertex incoming-hyperedge offsets.
    vertex_incoming_offsets: Vec<u32>,
    /// Incoming hyperedge IDs concatenated across vertices.
    vertex_incoming_hyperedges: Vec<u32>,
}

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

/// Dense relation weights for the benchmark hypergraph.
struct BenchHyperRelationWeights<'view> {
    /// Weights keyed by BCSR hyperedge ID.
    values: &'view [f64],
}

impl TopologyBase for BenchHyperRelationWeights<'_> {
    type ElementId = BcsrVertexId<u32>;
    type RelationId = BcsrHyperedgeId<u32>;
}

impl RelationWeight for BenchHyperRelationWeights<'_> {
    type Weight = f64;

    fn relation_weight(&self, relation: BcsrHyperedgeId<u32>) -> Self::Weight {
        self.values[relation.0 as usize]
    }
}

/// Dense incidence weights for the benchmark hypergraph.
struct BenchHyperIncidenceWeights<'view> {
    /// Weights keyed by BCSR participant ID.
    values: &'view [f64],
}

impl TopologyBase for BenchHyperIncidenceWeights<'_> {
    type ElementId = BcsrVertexId<u32>;
    type RelationId = BcsrHyperedgeId<u32>;
}

impl IncidenceBase for BenchHyperIncidenceWeights<'_> {
    type IncidenceId = BcsrParticipantId<u32>;
    type Role = BcsrRole;
}

impl IncidenceWeight for BenchHyperIncidenceWeights<'_> {
    type Weight = f64;

    fn incidence_weight(&self, incidence: BcsrParticipantId<u32>) -> Self::Weight {
        self.values[incidence.0 as usize]
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

/// Builds a deterministic 1-source/1-target hypergraph with one ring and one
/// skip hyperedge per vertex.
#[expect(
    clippy::cast_possible_truncation,
    reason = "benchmark fixture uses 10k vertices, far below u32::MAX"
)]
fn build_hypergraph(vertices: usize) -> Result<OwnedBcsr, BcsrError> {
    let edges = vertices.saturating_mul(2);
    let mut head_offsets = Vec::with_capacity(edges + 1);
    let mut head_participants = Vec::with_capacity(edges);
    let mut tail_offsets = Vec::with_capacity(edges + 1);
    let mut tail_participants = Vec::with_capacity(edges);
    head_offsets.push(0_u32);
    tail_offsets.push(0_u32);
    for vertex in 0..vertices {
        head_participants.push(vertex as u32);
        head_offsets.push(head_participants.len() as u32);
        tail_participants.push(((vertex + 1) % vertices) as u32);
        tail_offsets.push(tail_participants.len() as u32);
        head_participants.push(vertex as u32);
        head_offsets.push(head_participants.len() as u32);
        tail_participants.push(((vertex + 7) % vertices) as u32);
        tail_offsets.push(tail_participants.len() as u32);
    }
    let mut outgoing_buckets: Vec<Vec<u32>> = vec![Vec::new(); vertices];
    let mut incoming_buckets: Vec<Vec<u32>> = vec![Vec::new(); vertices];
    for (vertex, outgoing) in outgoing_buckets.iter_mut().enumerate() {
        let ring_edge = (vertex * 2) as u32;
        let skip_edge = (vertex * 2 + 1) as u32;
        outgoing.push(ring_edge);
        outgoing.push(skip_edge);
        let ring_target = (vertex + 1) % vertices;
        let skip_target = (vertex + 7) % vertices;
        incoming_buckets[ring_target].push(ring_edge);
        incoming_buckets[skip_target].push(skip_edge);
    }
    for bucket in &mut incoming_buckets {
        bucket.sort_unstable();
    }
    let mut vertex_outgoing_offsets = Vec::with_capacity(vertices + 1);
    let mut vertex_outgoing_hyperedges = Vec::new();
    let mut vertex_incoming_offsets = Vec::with_capacity(vertices + 1);
    let mut vertex_incoming_hyperedges = Vec::new();
    vertex_outgoing_offsets.push(0_u32);
    vertex_incoming_offsets.push(0_u32);
    for bucket in outgoing_buckets {
        vertex_outgoing_hyperedges.extend(bucket);
        vertex_outgoing_offsets.push(vertex_outgoing_hyperedges.len() as u32);
    }
    for bucket in incoming_buckets {
        vertex_incoming_hyperedges.extend(bucket);
        vertex_incoming_offsets.push(vertex_incoming_hyperedges.len() as u32);
    }
    let _hypergraph = BcsrNativeHypergraph::<u32, u32, u32>::open(BcsrSections {
        head_offsets: &head_offsets,
        head_participants: &head_participants,
        tail_offsets: &tail_offsets,
        tail_participants: &tail_participants,
        vertex_outgoing_offsets: &vertex_outgoing_offsets,
        vertex_outgoing_hyperedges: &vertex_outgoing_hyperedges,
        vertex_incoming_offsets: &vertex_incoming_offsets,
        vertex_incoming_hyperedges: &vertex_incoming_hyperedges,
    })?;
    Ok(OwnedBcsr {
        head_offsets,
        head_participants,
        tail_offsets,
        tail_participants,
        vertex_outgoing_offsets,
        vertex_outgoing_hyperedges,
        vertex_incoming_offsets,
        vertex_incoming_hyperedges,
    })
}

/// Opens an owned BCSR fixture as a native hypergraph view.
fn open_bench_hypergraph(sections: &OwnedBcsr) -> BcsrNativeHypergraph<'_, u32, u32, u32> {
    BcsrNativeHypergraph::<u32, u32, u32>::open(BcsrSections {
        head_offsets: &sections.head_offsets,
        head_participants: &sections.head_participants,
        tail_offsets: &sections.tail_offsets,
        tail_participants: &sections.tail_participants,
        vertex_outgoing_offsets: &sections.vertex_outgoing_offsets,
        vertex_outgoing_hyperedges: &sections.vertex_outgoing_hyperedges,
        vertex_incoming_offsets: &sections.vertex_incoming_offsets,
        vertex_incoming_hyperedges: &sections.vertex_incoming_hyperedges,
    })
    .unwrap_or_else(|error| panic!("benchmark BCSR hypergraph should validate: {error}"))
}

/// Registers the uniform allocating and workspace hypergraph `PageRank` benches.
fn bench_hyper_uniform(
    c: &mut Criterion,
    hypergraph: &BcsrNativeHypergraph<'_, u32, u32, u32>,
    elements: &[BcsrVertexId<u32>],
    relations: &[BcsrHyperedgeId<u32>],
    config: PageRankConfig<f64>,
) {
    c.bench_function("pagerank_hyper_bcsr_10k", |b| {
        let mut element_ranks = vec![0.0; 10_000];
        let mut relation_ranks = vec![0.0; 20_000];
        b.iter(|| {
            element_ranks.fill(0.0);
            relation_ranks.fill(0.0);
            pagerank_hypergraph(
                hypergraph,
                &Uniform,
                elements.iter().copied(),
                relations.iter().copied(),
                config,
                None,
                &mut element_ranks,
                &mut relation_ranks,
            )
            .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
        });
    });
    c.bench_function("pagerank_workspace_hyper_bcsr_10k", |b| {
        let mut workspace = HypergraphPageRankWorkspace::for_hypergraph(hypergraph);
        let mut element_ranks = vec![0.0; 10_000];
        let mut relation_ranks = vec![0.0; 20_000];
        b.iter(|| {
            pagerank_hypergraph_with_workspace(
                hypergraph,
                &Uniform,
                elements.iter().copied(),
                relations.iter().copied(),
                config,
                None,
                &mut element_ranks,
                &mut relation_ranks,
                &mut workspace,
            )
            .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
        });
    });
}

/// Registers the weighted visible-half hypergraph `PageRank` workspace bench.
#[expect(
    clippy::too_many_arguments,
    reason = "bench helper threads hypergraph, visible iterators, and weight adapters explicitly"
)]
fn bench_hyper_weighted_visible_half(
    c: &mut Criterion,
    hypergraph: &BcsrNativeHypergraph<'_, u32, u32, u32>,
    visible_elements: &[BcsrVertexId<u32>],
    visible_relations: &[BcsrHyperedgeId<u32>],
    relation_weights: &BenchHyperRelationWeights<'_>,
    incidence_weights: &BenchHyperIncidenceWeights<'_>,
    config: PageRankConfig<f64>,
) {
    c.bench_function(
        "weighted_pagerank_workspace_visible_half_hyper_bcsr_10k",
        |b| {
            let mut workspace = HypergraphPageRankWorkspace::for_hypergraph(hypergraph);
            let mut element_ranks = vec![0.0; 10_000];
            let mut relation_ranks = vec![0.0; 20_000];
            b.iter(|| {
                element_ranks.fill(0.0);
                relation_ranks.fill(0.0);
                pagerank_hypergraph_with_workspace(
                    hypergraph,
                    &HyperWeighted::new(relation_weights, incidence_weights),
                    visible_elements.iter().copied(),
                    visible_relations.iter().copied(),
                    config,
                    None,
                    &mut element_ranks,
                    &mut relation_ranks,
                    &mut workspace,
                )
                .unwrap_or_else(|error| panic!("benchmark PageRank should converge: {error}"))
            });
        },
    );
}

/// Benchmarks hypergraph `PageRank` over a 10k-vertex synthetic BCSR fixture.
fn hypergraph_pagerank_throughput(c: &mut Criterion) {
    let sections = build_hypergraph(10_000)
        .unwrap_or_else(|error| panic!("benchmark BCSR hypergraph should validate: {error}"));
    let hypergraph = open_bench_hypergraph(&sections);
    let elements: Vec<BcsrVertexId<u32>> = (0..10_000_u32).map(BcsrVertexId).collect();
    let relations: Vec<BcsrHyperedgeId<u32>> = (0..20_000_u32).map(BcsrHyperedgeId).collect();
    let visible_half: Vec<BcsrVertexId<u32>> = (0..10_000_u32)
        .filter(|vertex| vertex % 2 == 0)
        .map(BcsrVertexId)
        .collect();
    let visible_relations_half: Vec<BcsrHyperedgeId<u32>> = (0..20_000_u32)
        .filter(|relation| relation % 2 == 0)
        .map(BcsrHyperedgeId)
        .collect();
    let relation_weight_values = vec![1.0_f64; 20_000];
    let incidence_weight_values = vec![1.0_f64; 40_000];
    let relation_weights = BenchHyperRelationWeights {
        values: &relation_weight_values,
    };
    let incidence_weights = BenchHyperIncidenceWeights {
        values: &incidence_weight_values,
    };
    let config = PageRankConfig::new(0.85, 1.0e-9, 500);

    bench_hyper_uniform(c, &hypergraph, &elements, &relations, config);
    bench_hyper_weighted_visible_half(
        c,
        &hypergraph,
        &visible_half,
        &visible_relations_half,
        &relation_weights,
        &incidence_weights,
        config,
    );
}

criterion_group!(benches, pagerank_throughput, hypergraph_pagerank_throughput);
criterion_main!(benches);
