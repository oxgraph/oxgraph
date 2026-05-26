//! Deterministic graph generator aligned with pgGraph `benches/graph_gen.rs`.
//!
//! Produces power-law degree distributions (seed 42, bidirectional raw edges).
//! `oxgraph-postgres` builds directed CSR from the same `raw_edges` list pgGraph feeds
//! into `EdgeStoreBuilder`.
#![expect(
    clippy::missing_const_for_fn,
    reason = "RNG methods mutate internal state"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "benchmark node counts are small enough for usize on CI targets"
)]

/// Simple xorshift64 PRNG — deterministic, fast, no external dependency.
pub struct Rng(u64);

impl Rng {
    /// Creates a deterministic generator from `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    /// Returns the next pseudo-random `u64`.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Returns the next pseudo-random `u32`.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        u32::try_from(self.next_u64() >> 16).unwrap_or(u32::MAX)
    }

    /// Returns a value in `[0, n)`.
    #[inline]
    pub fn next_bounded(&mut self, n: u32) -> u32 {
        u32::try_from((u64::from(self.next_u32()) * u64::from(n)) >> 32).unwrap_or(0)
    }
}

/// One directed edge in the benchmark fixture (pgGraph `RawEdge` without type/weight).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEdge {
    /// Source node index.
    pub source: u32,
    /// Target node index.
    pub target: u32,
}

/// Generated benchmark fixture before CSR and secondary indexes are built.
#[derive(Clone, Debug)]
pub struct GeneratedBenchmarkGraph {
    /// Number of nodes `0..node_count`.
    pub node_count: u32,
    /// Synthetic directed edges (includes reverse-direction pairs from generation).
    pub raw_edges: Vec<RawEdge>,
}

/// Default seed for 10k `compare_pggraph` / extension benches.
pub const BENCH_SEED: u64 = 42;

/// Default node count for 10k workloads.
pub const BENCH_NODE_COUNT: u32 = 10_000;

/// Default average degree for 10k workloads.
pub const BENCH_AVG_DEGREE: u32 = 3;

/// Label string used in criterion benchmark ids.
pub const BENCH_LABEL_10K: &str = "10k";

/// Generates deterministic rows and raw edges without building graph indexes.
///
/// Matches pgGraph `build_benchmark_fixture` edge-generation logic.
///
/// # Performance
///
/// `O(m)` where `m` is the number of generated edges.
#[must_use]
pub fn build_benchmark_fixture(
    node_count: u32,
    avg_degree: u32,
    seed: u64,
) -> GeneratedBenchmarkGraph {
    let mut rng = Rng::new(seed);

    let total_edges = (u64::from(node_count) * u64::from(avg_degree)) as usize;
    let mut raw_edges = Vec::with_capacity(total_edges.saturating_mul(2));

    let mut edge_list: Vec<u32> = (0..node_count).collect();

    for _ in 0..total_edges {
        let source = rng.next_bounded(node_count);
        let target_idx = (rng.next_u64() % edge_list.len() as u64) as usize;
        let target = edge_list[target_idx];

        if source == target {
            continue;
        }

        raw_edges.push(RawEdge { source, target });
        raw_edges.push(RawEdge {
            source: target,
            target: source,
        });

        edge_list.push(source);
        edge_list.push(target);
    }

    GeneratedBenchmarkGraph {
        node_count,
        raw_edges,
    }
}

/// Outgoing degree per node from the fixture edge list.
#[must_use]
pub fn out_degrees(fixture: &GeneratedBenchmarkGraph) -> Vec<u32> {
    let mut degree = vec![0_u32; fixture.node_count as usize];
    for edge in &fixture.raw_edges {
        let source = edge.source as usize;
        if source < degree.len() {
            degree[source] = degree[source].saturating_add(1);
        }
    }
    degree
}

/// Returns the node index with highest outgoing degree in the fixture.
#[must_use]
pub fn find_supernode(fixture: &GeneratedBenchmarkGraph) -> u32 {
    let mut best_idx = 0_u32;
    let mut best_degree = 0_u32;
    for (idx, &deg) in out_degrees(fixture).iter().enumerate() {
        if deg > best_degree {
            best_degree = deg;
            best_idx = u32::try_from(idx).unwrap_or(0);
        }
    }
    best_idx
}
