//! Shared benchmark and extension-integration fixtures (pgGraph-aligned `graph_gen`).

pub mod artifact;
pub mod graph;
pub mod relational;

pub use artifact::build_oxgraph_bytes;
pub use graph::{
    BENCH_AVG_DEGREE, BENCH_LABEL_10K, BENCH_NODE_COUNT, BENCH_SEED, GeneratedBenchmarkGraph,
    RawEdge, Rng, build_benchmark_fixture, find_supernode, out_degrees,
};
pub use relational::{CHAIN_CATALOG_DML, CHAIN_RELATIONAL_DDL, chain_catalog, chain_edges};
