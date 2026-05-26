//! OXGTOPO snapshot bytes from benchmark graph fixtures.

use super::graph::GeneratedBenchmarkGraph;
use crate::{DualTopologySnapshot, PostgresGraphError};

/// Builds OXGTOPO bytes (forward CSR + inbound CSC + metadata) from a shared fixture.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Build`] when CSR construction fails.
///
/// # Performance
///
/// `O(m)` for `m` edges in the fixture.
pub fn build_oxgraph_bytes(
    fixture: &GeneratedBenchmarkGraph,
) -> Result<Vec<u8>, PostgresGraphError> {
    let edges: alloc::vec::Vec<(u32, u32)> = fixture
        .raw_edges
        .iter()
        .map(|edge| (edge.source, edge.target))
        .collect();
    DualTopologySnapshot::from_dense_u32_edges(&edges, 0)
}
