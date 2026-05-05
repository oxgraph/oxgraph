//! Demonstrates traversing a borrowed CSR graph through `oxgraph-graph` traits.

use oxgraph_csr::{CsrGraph, CsrNodeId};
use oxgraph_graph::{EdgeTargetGraph, GraphCounts, OutgoingEdgeCount, OutgoingGraph};

/// Runs the example and reports validation errors.
fn main() -> Result<(), oxgraph_csr::CsrError<u32>> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 2, 3];

    let graph = CsrGraph::validate(4, OFFSETS, TARGETS)?;

    println!("nodes={}", graph.node_count());
    println!("edges={}", graph.edge_count());

    for edge in graph.outgoing_edges(CsrNodeId(0)) {
        println!("edge={:?} target={:?}", edge, graph.target(edge));
    }

    println!("out_degree(0)={}", graph.out_degree(CsrNodeId(0)));

    Ok(())
}
