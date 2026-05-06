//! Curated feature-gated re-exports for `OxGraph`.
//!
//! The umbrella crate does not add helper APIs that kernel crates lack. It only
//! collects the workspace surface behind explicit feature flags so consumers can
//! choose the layers they need.
#![no_std]

/// Substrate-neutral topology capability traits.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "topology")]
pub mod topology {
    pub use oxgraph_topology::*;
}

/// Binary graph vocabulary traits.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "graph")]
pub mod graph {
    pub use oxgraph_graph::*;
}

/// Hypergraph vocabulary traits.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "hyper")]
pub mod hyper {
    pub use oxgraph_hyper::*;
}

/// Borrowed CSR graph layout.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "csr")]
pub mod csr {
    pub use oxgraph_csr::*;
}

/// Borrowed bipartite-CSR hypergraph layout.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "hyper-bcsr")]
pub mod hyper_bcsr {
    pub use oxgraph_hyper_bcsr::*;
}

/// Topology-agnostic snapshot container.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "snapshot")]
pub mod snapshot {
    pub use oxgraph_snapshot::*;
}

/// Substrate-agnostic algorithms.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "algo")]
pub mod algo {
    pub use oxgraph_algo::*;
}

/// Arrow-backed property layers.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "property")]
pub mod property {
    pub use oxgraph_property::*;
}

/// Append/update graph builders.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "graph-build")]
pub mod graph_build {
    pub use oxgraph_graph_build::*;
}

/// Append/update hypergraph builders.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "hyper-build")]
pub mod hyper_build {
    pub use oxgraph_hyper_build::*;
}
