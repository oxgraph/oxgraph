//! Walkthrough: build a CSR snapshot, open it, and run BFS over it.
//!
//! The snapshot crate is topology-agnostic; this example shows how a graph
//! crate stacks on top of it by registering its own section kinds and
//! validating its own payload shapes.
//!
//! Run with: `cargo run -p oxgraph-csr --example open_snapshot --features oxgraph-snapshot/alloc`

use oxgraph_algo::breadth_first_search;
use oxgraph_csr::{CsrNodeId, CsrSnapshotError, CsrSnapshotGraph, CsrSnapshotIndex};
use oxgraph_graph::GraphCounts;
use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotError, SnapshotWriter};

/// `u32` CSR offsets section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_CSR_OFFSETS_U32: u32 = <u32 as CsrSnapshotIndex>::OFFSETS_KIND;
/// `u32` CSR targets section kind derived from the base-plus-width scheme.
const SNAPSHOT_KIND_CSR_TARGETS_U32: u32 = <u32 as CsrSnapshotIndex>::TARGETS_KIND;

/// Local error type covering both snapshot and CSR adaptor failures.
#[derive(Debug)]
enum DemoError {
    /// Snapshot opening failed.
    Snapshot(SnapshotError),
    /// CSR adaptor failed.
    Adaptor(CsrSnapshotError<u32, u32>),
}

impl From<SnapshotError> for DemoError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<CsrSnapshotError<u32, u32>> for DemoError {
    fn from(error: CsrSnapshotError<u32, u32>) -> Self {
        Self::Adaptor(error)
    }
}

impl core::fmt::Display for DemoError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "snapshot: {error}"),
            Self::Adaptor(error) => write!(formatter, "adaptor: {error}"),
        }
    }
}

impl std::error::Error for DemoError {}

fn main() -> Result<(), DemoError> {
    let offsets: [u32; 5] = [0, 2, 3, 4, 4];
    let targets: [u32; 4] = [1, 2, 2, 3];

    let offsets_bytes: Vec<u8> = offsets.iter().flat_map(|word| word.to_le_bytes()).collect();
    let targets_bytes: Vec<u8> = targets.iter().flat_map(|word| word.to_le_bytes()).collect();

    let mut writer = match SnapshotWriter::new(2, crc32c_append) {
        Ok(value) => value,
        Err(error) => panic!("writer: {error:?}"),
    };
    if let Err(error) = writer.section_bytes(SNAPSHOT_KIND_CSR_OFFSETS_U32, 0, 2, &offsets_bytes) {
        panic!("offsets section: {error:?}");
    }
    if let Err(error) = writer.section_bytes(SNAPSHOT_KIND_CSR_TARGETS_U32, 0, 2, &targets_bytes) {
        panic!("targets section: {error:?}");
    }
    let bytes = match writer.finish() {
        Ok(value) => value,
        Err(error) => panic!("writer finish: {error:?}"),
    };
    println!("encoded snapshot: {} bytes", bytes.len());

    let snapshot = Snapshot::open(&bytes)?;
    let graph = CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot)?;
    println!(
        "graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    let order: Vec<u32> = match breadth_first_search(&graph, CsrNodeId::new(0)) {
        Ok(walk) => walk.map(CsrNodeId::get).collect(),
        Err(error) => panic!("bfs failed: {error:?}"),
    };
    println!("bfs from node 0: {order:?}");

    Ok(())
}
