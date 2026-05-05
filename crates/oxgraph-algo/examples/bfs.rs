//! Demonstrates scratch-backed directed BFS over a borrowed CSR graph.

use std::fmt;

use oxgraph_algo::{BfsError, breadth_first_search_with_scratch};
use oxgraph_csr::{CsrError, CsrGraph, CsrNodeId};

/// Error returned by the example.
#[derive(Debug)]
enum ExampleError {
    /// CSR validation failed.
    Csr(CsrError<u32>),
    /// BFS scratch validation failed.
    Bfs(BfsError),
}

impl From<CsrError<u32>> for ExampleError {
    fn from(error: CsrError<u32>) -> Self {
        Self::Csr(error)
    }
}

impl From<BfsError> for ExampleError {
    fn from(error: BfsError) -> Self {
        Self::Bfs(error)
    }
}

impl fmt::Display for ExampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Csr(error) => write!(formatter, "CSR validation failed: {error:?}"),
            Self::Bfs(error) => write!(formatter, "BFS scratch validation failed: {error:?}"),
        }
    }
}

/// Runs the example and reports CSR validation errors.
fn main() -> Result<(), ExampleError> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 2, 3];

    let graph = CsrGraph::validate(4, OFFSETS, TARGETS)?;
    let mut visited = [0; 4];
    let mut queue = [CsrNodeId(0); 4];

    for node in breadth_first_search_with_scratch(&graph, CsrNodeId(0), &mut visited, &mut queue)? {
        println!("bfs_node={node:?}");
    }

    Ok(())
}
