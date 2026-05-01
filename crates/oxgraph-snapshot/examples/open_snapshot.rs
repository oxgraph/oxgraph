//! Demonstrates opening a v0 graph snapshot and interpreting CSR sections.

use oxgraph_algo::{BfsError, breadth_first_search};
use oxgraph_csr::{CsrError, CsrGraph, CsrNodeId};
use oxgraph_graph::{EdgeTargetGraph, GraphCounts, OutgoingGraph};
use oxgraph_snapshot::GraphSnapshot;

/// CSR offsets section kind used by this example fixture.
const SECTION_CSR_OFFSETS: u32 = 1;

/// CSR targets section kind used by this example fixture.
const SECTION_CSR_TARGETS: u32 = 2;

/// Runs the example and reports snapshot validation errors.
fn main() -> Result<(), ExampleError> {
    static BYTES: &[u8] = &[
        79, 67, 84, 88, 71, 48, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0,
        48, 0, 0, 0, 20, 0, 0, 0, 2, 0, 0, 0, 68, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 3,
        0, 0, 0, 4, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0,
    ];

    let snapshot = GraphSnapshot::open(BYTES)?;
    let offsets = snapshot
        .section_words(SECTION_CSR_OFFSETS)?
        .ok_or(ExampleError::MissingSection(SECTION_CSR_OFFSETS))?;
    let targets = snapshot
        .section_words(SECTION_CSR_TARGETS)?
        .ok_or(ExampleError::MissingSection(SECTION_CSR_TARGETS))?;
    let graph = CsrGraph::validate(snapshot.node_count(), offsets, targets)?;

    println!("nodes={}", graph.node_count());
    println!("edges={}", graph.edge_count());
    println!(
        "targets_from_0={:?}",
        graph
            .outgoing_edges(CsrNodeId(0))
            .map(|edge| graph.target(edge))
            .collect::<Vec<_>>()
    );
    println!(
        "bfs={:?}",
        breadth_first_search(&graph, CsrNodeId(0))?.collect::<Vec<_>>()
    );

    Ok(())
}

/// Error returned by the example.
#[derive(Debug)]
enum ExampleError {
    /// Snapshot validation failed.
    Snapshot(oxgraph_snapshot::SnapshotError),
    /// A section required by the example fixture was missing.
    MissingSection(u32),
    /// CSR validation failed.
    Csr(CsrError),
    /// BFS construction failed.
    Bfs(BfsError),
}

impl From<oxgraph_snapshot::SnapshotError> for ExampleError {
    fn from(error: oxgraph_snapshot::SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<BfsError> for ExampleError {
    fn from(error: BfsError) -> Self {
        Self::Bfs(error)
    }
}

impl From<CsrError> for ExampleError {
    fn from(error: CsrError) -> Self {
        Self::Csr(error)
    }
}

impl std::fmt::Display for ExampleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot validation failed: {error}"),
            Self::MissingSection(kind) => write!(formatter, "missing section {kind}"),
            Self::Csr(error) => write!(formatter, "CSR validation failed: {error}"),
            Self::Bfs(error) => write!(formatter, "BFS construction failed: {error}"),
        }
    }
}
